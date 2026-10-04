//! Dedicated SQLite writer. No database I/O on response-body polling threads.
use super::{Delivery, RequestLog};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::Duration,
};
const ERROR: &str = "调用日志读取或保存失败，请检查日志目录权限、磁盘空间及是否运行了另一实例";
const QUEUE_SIZE: usize = 256;
// The bounded 256-slot queue costs about 64 KiB at this size. Keep completion
// values inline instead of adding an allocation to every finished request.
#[expect(
    clippy::large_enum_variant,
    reason = "bounded queue; avoid per-request boxing"
)]
enum Message {
    Insert {
        id: u64,
        row: String,
    },
    Finish {
        id: u64,
        response: Value,
        state: String,
        usage: Value,
        completion: Value,
        delivery: Option<Delivery>,
    },
    Read(mpsc::Sender<Result<Vec<Value>, String>>),
    Flush(mpsc::Sender<Result<(), String>>),
    Stop,
}
pub(super) struct Store {
    tx: mpsc::SyncSender<Message>,
    failed: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    path: std::path::PathBuf,
}
impl Store {
    pub(super) fn open(directory: &Path) -> Result<(Self, u64), String> {
        let (mut conn, lock) = open_database(directory).map_err(|_| ERROR.to_string())?;
        recover(&mut conn).map_err(|_| ERROR.to_string())?;
        super::history::initialize(&mut conn).map_err(|_| ERROR.to_string())?;
        let next: u64 = conn
            .query_row(
                "SELECT COALESCE(MAX(id), -1) + 1 FROM request_logs",
                [],
                |r| r.get(0),
            )
            .map_err(|_| ERROR.to_string())?;
        if next >= 9_007_199_254_740_991 {
            return Err(ERROR.into());
        }
        let (tx, rx) = mpsc::sync_channel(QUEUE_SIZE);
        let failed = Arc::new(AtomicBool::new(false));
        let health = failed.clone();
        let worker = std::thread::Builder::new()
            .name("manual-log-sqlite".into())
            .spawn(move || {
                let _lock = lock;
                while let Ok(message) = rx.recv() {
                    let result = match message {
                        Message::Insert { id, row } => insert(&mut conn, id, row),
                        Message::Finish {
                            id,
                            response,
                            state,
                            usage,
                            completion,
                            delivery,
                        } => finish(&mut conn, id, response, state, usage, completion, delivery),
                        Message::Read(reply) => {
                            let result = if health.load(Ordering::Relaxed) {
                                Err(ERROR.into())
                            } else {
                                read(&conn).map_err(|_| ERROR.to_string())
                            };
                            let _ = reply.send(result);
                            continue;
                        }
                        Message::Flush(reply) => {
                            let _ = reply.send(if health.load(Ordering::Relaxed) {
                                Err(ERROR.into())
                            } else {
                                Ok(())
                            });
                            continue;
                        }
                        Message::Stop => break,
                    };
                    if result.is_err() {
                        health.store(true, Ordering::Relaxed);
                        log::error!("调用日志 SQLite 写入失败；界面将提示存储异常，不影响请求转发");
                    }
                }
            })
            .map_err(|_| ERROR.to_string())?;
        Ok((
            Self {
                tx,
                failed,
                worker: Some(worker),
                path: directory.join("requests.sqlite3"),
            },
            next,
        ))
    }
    fn enqueue(&self, message: Message) {
        if self.tx.try_send(message).is_err() {
            self.failed.store(true, Ordering::Relaxed);
            log::error!("调用日志写入队列不可用；部分记录未保存");
        }
    }
    pub(super) fn insert(&self, row: &RequestLog) {
        match serde_json::to_string(row) {
            Ok(value) => self.enqueue(Message::Insert {
                id: row.id,
                row: value,
            }),
            Err(_) => {
                self.failed.store(true, Ordering::Relaxed);
            }
        }
    }
    pub(super) fn finish(
        &self,
        id: u64,
        response: Value,
        state: &str,
        usage: Value,
        completion: Value,
        delivery: Option<Delivery>,
    ) {
        self.enqueue(Message::Finish {
            id,
            response,
            state: state.into(),
            usage,
            completion,
            delivery,
        });
    }
    pub(super) fn healthy(&self) -> bool {
        !self.failed.load(Ordering::Relaxed)
    }
    pub(super) fn query(&self, query: &super::history::Query) -> Result<Value, String> {
        query.validate()?;
        self.flush()?;
        let result = (|| {
            let mut reader =
                Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|_| ERROR.to_string())?;
            reader
                .busy_timeout(Duration::from_secs(2))
                .map_err(|_| ERROR.to_string())?;
            super::history::query(&mut reader, query)
        })();
        if result.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }
        result
    }
    pub(super) fn snapshot(&self) -> Result<Vec<Value>, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Message::Read(tx))
            .map_err(|_| ERROR.to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| ERROR.to_string())?
    }
    pub(super) fn flush(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Message::Flush(tx))
            .map_err(|_| ERROR.to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| ERROR.to_string())?
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn open_database(directory: &Path) -> Result<(Connection, File), Box<dyn std::error::Error>> {
    if let Ok(meta) = fs::symlink_metadata(directory) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("invalid log directory".into());
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    for name in [
        "writer.lock",
        "requests.sqlite3",
        "requests.sqlite3-wal",
        "requests.sqlite3-shm",
        "requests.sqlite3-journal",
    ] {
        let path = directory.join(name);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("invalid log file".into());
            }
        }
    }
    let private_file = |name: &str| -> std::io::Result<File> {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(directory.join(name))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        Ok(file)
    };
    let lock = private_file("writer.lock")?;
    lock.try_lock()?;
    drop(private_file("requests.sqlite3")?);
    let conn = Connection::open(directory.join("requests.sqlite3"))?;
    conn.busy_timeout(Duration::from_millis(500))?;
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        return Err("unsupported log schema".into());
    }
    if version == 1 {
        conn.prepare("SELECT id, record_json, receiving FROM request_logs LIMIT 0")?;
    }
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS request_logs (
          id INTEGER PRIMARY KEY, record_json TEXT NOT NULL, receiving INTEGER NOT NULL CHECK(receiving IN (0, 1))
        );
        CREATE INDEX IF NOT EXISTS request_logs_receiving ON request_logs(receiving) WHERE receiving=1;
        PRAGMA user_version=1;")?;
    Ok((conn, lock))
}
fn decode(text: String) -> rusqlite::Result<Value> {
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    if !value.is_object() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(value)
}
fn read(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut query =
        conn.prepare("SELECT record_json FROM request_logs ORDER BY id DESC LIMIT 200")?;
    let rows = query.query_map([], |r| decode(r.get(0)?))?.collect();
    rows
}
fn insert(conn: &mut Connection, id: u64, row: String) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO request_logs(id, record_json, receiving) VALUES (?1, ?2, 1)",
        params![id, row],
    )?;
    super::history::sync_one(&tx, id)?;
    tx.commit()
}
fn finish(
    conn: &mut Connection,
    id: u64,
    response: Value,
    state: String,
    usage: Value,
    completion: Value,
    delivery: Option<Delivery>,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let mut row = decode(tx.query_row(
        "SELECT record_json FROM request_logs WHERE id=?1",
        [id],
        |r| r.get(0),
    )?)?;
    row["response"] = response;
    row["responseState"] = json!(state);
    row["usage"] = usage;
    row["completion"] = completion;
    row["delivery"] = json!(delivery);
    tx.execute(
        "UPDATE request_logs SET record_json=?2, receiving=0 WHERE id=?1",
        params![id, row.to_string()],
    )?;
    super::history::sync_one(&tx, id)?;
    tx.commit()
}
fn recover(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    loop {
        let rows = {
            let mut query =
                tx.prepare("SELECT id, record_json FROM request_logs WHERE receiving=1 LIMIT 200")?;
            let rows = query
                .query_map([], |r| Ok((r.get::<_, u64>(0)?, decode(r.get(1)?)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (id, mut row) in rows {
            row["responseState"] = json!("已中断（上次运行未结束）");
            row["completion"] = json!("unknown");
            if row.pointer("/usage/state").and_then(Value::as_str) == Some("pending") {
                row["usage"]["state"] = json!("unavailable");
                row["usage"]["reason"] = json!("incomplete_stream");
            }
            tx.execute(
                "UPDATE request_logs SET record_json=?2, receiving=0 WHERE id=?1",
                params![id, row.to_string()],
            )?;
        }
    }
    tx.commit()
}
