use super::super::token_usage::{Usage, UsageState};
use super::*;
use serde_json::json;

fn row() -> RequestLog {
    RequestLog {
        started_at: "2026-09-18T00:00:00Z".into(),
        endpoint: "/v1/responses".into(),
        status: 200,
        streaming: true,
        response_state: "接收中".into(),
        caller: Caller::Key {
            key_id: "fixture-id".into(),
            name: "MacBook".into(),
        },
        input: json!({"text":"sanitized input"}),
        ..RequestLog::default()
    }
}
fn usage(state: UsageState) -> Usage {
    Usage {
        state,
        input_tokens: Some(100),
        output_tokens: Some(25),
        total_tokens: Some(125),
        cache_read_tokens: Some(40),
        cache_write_tokens: None,
        reasoning_tokens: Some(5),
        reason: None,
    }
}
#[test]
fn completed_records_usage_and_stable_ids_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(dir.path()).unwrap();
    let first = logs.push(row());
    logs.finish_observed(
        first,
        json!({"text":"sanitized response"}),
        "已结束",
        Some(usage(UsageState::Reported)),
        Some(crate::manual::completion::Completion::Completed),
    );
    let expected = logs.persisted_snapshot().unwrap();
    assert_eq!(expected[0]["usage"]["totalTokens"], 125);
    assert_eq!(expected[0]["completion"], "completed");
    assert_eq!(expected[0]["caller"]["keyId"], "fixture-id");
    assert!(
        RequestLogs::open(dir.path()).is_err(),
        "another writer must not recover live requests"
    );
    drop(logs);
    let reopened = RequestLogs::open(dir.path()).unwrap();
    assert_eq!(reopened.persisted_snapshot().unwrap(), expected);
    let second = reopened.push(row());
    assert!(second > first);
    reopened.finish(second, json!(null), "已结束", None);
    assert_eq!(reopened.persisted_snapshot().unwrap().len(), 2);
}
#[test]
fn display_limit_does_not_delete_history_or_lose_late_stream_completion() {
    let dir = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(dir.path()).unwrap();
    let first = logs.push(row());
    for _ in 0..5 {
        for _ in 0..50 {
            let id = logs.push(row());
            logs.finish(id, json!(null), "已结束", None);
        }
        logs.flush().unwrap();
    }
    logs.finish(
        first,
        json!({"text":"late ending"}),
        "已结束",
        Some(usage(UsageState::Reported)),
    );
    let visible = logs.persisted_snapshot().unwrap();
    assert_eq!(visible.len(), 200);
    assert_eq!(visible[0]["id"], 250);
    let db = rusqlite::Connection::open(dir.path().join("requests.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        251
    );
    let saved: String = db
        .query_row(
            "SELECT record_json FROM request_logs WHERE id=?1",
            [first],
            |r| r.get(0),
        )
        .unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["usage"]["totalTokens"], 125);
    assert_eq!(saved["responseState"], "已结束");
}
#[test]
fn unfinished_requests_recover_as_interrupted_not_permanently_pending() {
    let dir = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(dir.path()).unwrap();
    let mut pending = row();
    pending.usage = Some(usage(UsageState::Pending));
    logs.push(pending);
    let mut reported = row();
    reported.usage = Some(usage(UsageState::Reported));
    logs.push(reported);
    logs.flush().unwrap();
    drop(logs);
    let reopened = RequestLogs::open(dir.path()).unwrap();
    let rows = reopened.persisted_snapshot().unwrap();
    assert_eq!(rows[0]["usage"]["state"], "reported");
    assert_eq!(rows[1]["usage"]["state"], "unavailable");
    assert_eq!(rows[1]["usage"]["reason"], "incomplete_stream");
    assert_eq!(rows[1]["responseState"], "已中断（上次运行未结束）");
}
#[test]
fn corrupt_database_is_not_reset_and_write_failure_is_not_silent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("requests.sqlite3");
    std::fs::write(&file, b"not a SQLite database").unwrap();
    assert!(RequestLogs::open(dir.path()).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"not a SQLite database");
    let good = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(good.path()).unwrap();
    let conn = rusqlite::Connection::open(good.path().join("requests.sqlite3")).unwrap();
    conn.execute("DROP TABLE request_logs", []).unwrap();
    logs.push(row());
    assert!(logs.flush().is_err());
    assert!(logs.persisted_snapshot().is_err());
}
#[cfg(unix)]
#[test]
fn database_is_private_and_symlink_targets_are_not_followed() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(dir.path()).unwrap();
    logs.push(row());
    logs.flush().unwrap();
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in [
        "requests.sqlite3",
        "requests.sqlite3-wal",
        "requests.sqlite3-shm",
        "writer.lock",
    ] {
        assert_eq!(
            std::fs::metadata(dir.path().join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let other = tempfile::tempdir().unwrap();
    symlink(
        dir.path().join("requests.sqlite3"),
        other.path().join("requests.sqlite3"),
    )
    .unwrap();
    assert!(RequestLogs::open(other.path()).is_err());
}
