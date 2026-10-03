//! One-way, no-merge migration of this application's data. Never opens ~/.cc-switch.
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

const OLD: &str = ".cc-switch-manual";
const NEW: &str = ".lumagate";

fn private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
fn lock(path: &Path) -> io::Result<File> {
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err(io::Error::other("迁移锁不是普通文件"));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.try_lock()
        .map_err(|_| io::Error::other("LumaGate 或旧版本仍在运行，请退出后重试"))?;
    Ok(file)
}
fn exists(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "迁移路径不是普通目录：{}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
fn inventory(root: &Path) -> io::Result<BTreeMap<PathBuf, (u64, String, u32)>> {
    fn visit(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, (u64, String, u32)>,
    ) -> io::Result<()> {
        let meta = path.symlink_metadata()?;
        if meta.file_type().is_symlink() || !(meta.is_file() || meta.is_dir()) {
            return Err(io::Error::other("迁移目录含符号链接或特殊文件，未移动数据"));
        }
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = u32::from(meta.permissions().readonly());
        if meta.is_dir() {
            result.insert(
                path.strip_prefix(root).unwrap().into(),
                (0, "directory".into(), mode),
            );
            for entry in fs::read_dir(path)? {
                visit(root, &entry?.path(), result)?;
            }
        } else {
            let mut file = File::open(path)?;
            let mut digest = Sha256::new();
            io::copy(&mut file, &mut digest)?;
            result.insert(
                path.strip_prefix(root).unwrap().into(),
                (meta.len(), format!("{:x}", digest.finalize()), mode),
            );
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result)?;
    Ok(result)
}
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = from.symlink_metadata()?;
    if meta.is_dir() {
        fs::create_dir(to)?;
        fs::set_permissions(to, meta.permissions())?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if meta.is_file() {
        fs::copy(from, to)?;
        File::open(to)?.sync_all()?;
    } else {
        return Err(io::Error::other("拒绝复制特殊文件"));
    }
    Ok(())
}
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from = CString::new(from.as_os_str().as_bytes())?;
        let to = CString::new(to.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Windows rename refuses to replace an existing directory.
        fs::rename(from, to)
    }
}
fn databases(root: &Path) -> io::Result<Vec<Connection>> {
    let mut held = Vec::new();
    for relative in ["cc-switch.db", "logs/requests.sqlite3"] {
        let path = root.join(relative);
        if !path.exists() {
            continue;
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(io::Error::other)?;
        conn.busy_timeout(std::time::Duration::from_millis(250))
            .map_err(io::Error::other)?;
        let (busy, _, _): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(io::Error::other)?;
        if busy != 0 {
            return Err(io::Error::other("数据库仍有活动读写，请退出旧版本后重试"));
        }
        conn.execute_batch("BEGIN EXCLUSIVE")
            .map_err(io::Error::other)?;
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(io::Error::other)?;
        if integrity != "ok" {
            return Err(io::Error::other("数据库完整性检查失败，未移动数据"));
        }
        held.push(conn);
    }
    Ok(held)
}

#[cfg(target_os = "macos")]
fn support_moves(home: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let pairs = [
        (
            "Library/WebKit/cn.restry.ccswitch.manual",
            "Library/WebKit/cn.restry.lumagate",
        ),
        (
            "Library/Caches/cn.restry.ccswitch.manual",
            "Library/Caches/cn.restry.lumagate",
        ),
        (
            "Library/Application Support/cn.restry.ccswitch.manual",
            "Library/Application Support/cn.restry.lumagate",
        ),
        (
            "Library/Application Support/CC Switch Manual",
            "Library/Application Support/LumaGate",
        ),
    ];
    let mut moves = Vec::new();
    for (old, new) in pairs {
        let (old, new) = (home.join(old), home.join(new));
        if exists(&old)? {
            if exists(&new)? {
                return Err(io::Error::other(format!(
                    "应用支持目录冲突，未合并：{}",
                    new.display()
                )));
            }
            inventory(&old)?;
            moves.push((old, new));
        }
    }
    Ok(moves)
}

/// Hold the returned lock for the process lifetime. Backups are private and outside the repository.
/// A conflict is an error even when the destination looks empty; never merge or overwrite.
pub fn prepare(home: &Path) -> io::Result<File> {
    let control = home.join(".local/share/lumagate");
    private_dir(&control)?;
    let guard = lock(&control.join("instance.lock"))?;
    let old = home.join(OLD);
    let new = home.join(NEW);
    let old_exists = exists(&old)?;
    let new_exists = exists(&new)?;
    if old_exists && new_exists {
        return Err(io::Error::other(
            "~/.cc-switch-manual 与 ~/.lumagate 同时存在；未覆盖或合并，请保留备份并处理冲突后重试",
        ));
    }
    #[cfg(target_os = "macos")]
    let support = support_moves(home)?;
    if old_exists {
        // Old versions already hold this lock even while their gateway is stopped.
        let _old_writer = if old.join("logs").exists() {
            Some(lock(&old.join("logs/writer.lock"))?)
        } else {
            None
        };
        inventory(&old)?;
        let dbs = databases(&old)?;
        let before = inventory(&old)?;
        let backup_root = control.join("migration-backups");
        private_dir(&backup_root)?;
        let backup = backup_root.join(uuid::Uuid::new_v4().to_string());
        copy_tree(&old, &backup)?;
        if inventory(&backup)? != before {
            return Err(io::Error::other("迁移备份核验失败，原目录未移动"));
        }
        // Release SQLite transactions before renaming; writer locks remain held throughout.
        drop(dbs);
        rename_no_replace(&old, &new)?;
        let after = inventory(&new)?;
        // SQLite may remove its empty WAL/SHM sidecars on close. Compare durable content.
        let durable = |items: BTreeMap<PathBuf, (u64, String, u32)>| {
            items
                .into_iter()
                .filter(|(p, _)| {
                    !p.to_string_lossy().ends_with("-wal") && !p.to_string_lossy().ends_with("-shm")
                })
                .collect::<BTreeMap<_, _>>()
        };
        if durable(before) != durable(after) {
            return Err(io::Error::other(
                "迁移后核验不一致，请保留新目录及私有备份，勿重建配置",
            ));
        }
        let receipt = serde_json::json!({"from":old,"to":new,"backup":backup,"integrity":"ok","durableFilesAndPermissionsEqual":true});
        fs::write(
            control.join("migration-receipt.json"),
            serde_json::to_vec_pretty(&receipt)?,
        )?;
    } else if !new_exists {
        private_dir(&new)?;
    }
    #[cfg(target_os = "macos")]
    for (old, new) in support {
        let before = inventory(&old)?;
        let backup_root = control.join("migration-backups");
        private_dir(&backup_root)?;
        let backup = backup_root.join(format!("support-{}", uuid::Uuid::new_v4()));
        copy_tree(&old, &backup)?;
        if inventory(&backup)? != before {
            return Err(io::Error::other("应用支持目录备份核验失败"));
        }
        rename_no_replace(&old, &new)?;
        if inventory(&new)? != before {
            return Err(io::Error::other("应用支持目录迁移核验失败"));
        }
    }
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_install_and_already_migrated_never_touch_upstream() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".cc-switch")).unwrap();
        fs::write(home.path().join(".cc-switch/untouched"), "original").unwrap();
        drop(prepare(home.path()).unwrap());
        fs::write(home.path().join(NEW).join("marker"), "new").unwrap();
        drop(prepare(home.path()).unwrap());
        assert_eq!(
            fs::read(home.path().join(NEW).join("marker")).unwrap(),
            b"new"
        );
        assert_eq!(
            fs::read(home.path().join(".cc-switch/untouched")).unwrap(),
            b"original"
        );
        assert!(!home.path().join(OLD).exists());
    }
    #[test]
    fn old_data_wal_secrets_and_permissions_survive() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join(OLD);
        private_dir(&old.join("keys")).unwrap();
        fs::write(old.join("keys/fixture"), "private-fixture").unwrap();
        let conn = Connection::open(old.join("cc-switch.db")).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES('retained');").unwrap();
        drop(prepare(home.path()).unwrap());
        drop(conn);
        assert!(!old.exists());
        assert_eq!(
            fs::read(home.path().join(NEW).join("keys/fixture")).unwrap(),
            b"private-fixture"
        );
        let conn = Connection::open(home.path().join(NEW).join("cc-switch.db")).unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM fixture", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "retained"
        );
    }
    #[test]
    fn conflict_and_active_writer_never_move_data() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join(OLD);
        private_dir(&old.join("logs")).unwrap();
        private_dir(&home.path().join(NEW)).unwrap();
        assert!(prepare(home.path()).is_err());
        assert!(old.exists());
        fs::remove_dir(home.path().join(NEW)).unwrap();
        let held = lock(&old.join("logs/writer.lock")).unwrap();
        assert!(prepare(home.path()).is_err());
        assert!(old.exists());
        assert!(!home.path().join(NEW).exists());
        drop(held);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_store_conflict_precedes_data_move_and_preferences_survive() {
        let home = tempfile::tempdir().unwrap();
        private_dir(&home.path().join(OLD)).unwrap();
        let old = home.path().join("Library/WebKit/cn.restry.ccswitch.manual");
        let new = home.path().join("Library/WebKit/cn.restry.lumagate");
        private_dir(&old).unwrap();
        private_dir(&new).unwrap();
        fs::write(old.join("preferences"), "retained").unwrap();
        assert!(prepare(home.path()).is_err());
        assert!(home.path().join(OLD).exists());
        assert!(!home.path().join(NEW).exists());
        fs::remove_dir(&new).unwrap();
        drop(prepare(home.path()).unwrap());
        assert_eq!(fs::read(new.join("preferences")).unwrap(), b"retained");
        assert!(!old.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_destination_is_never_followed() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join(NEW)).unwrap();
        assert!(prepare(home.path()).is_err());
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }
}
