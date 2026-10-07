use super::*;
use crate::manual::{
    logs::{Caller, ProviderAttempt, RequestLog, RequestLogs},
    token_usage::{Usage, UsageState},
};
fn entry(day: u32, model: &str) -> RequestLog {
    RequestLog {
        started_at: format!("2026-09-{day:02}T12:00:00.000Z"),
        endpoint: "/v1/chat/completions".into(),
        model: Some(model.into()),
        status: 200,
        streaming: false,
        caller: Caller::Key {
            key_id: "key-fixture".into(),
            name: "Fixture key".into(),
        },
        providers: vec![
            ProviderAttempt {
                id: "a".into(),
                name: "Alpha".into(),
                outcome: "失败".into(),
            },
            ProviderAttempt {
                id: "b".into(),
                name: "Beta".into(),
                outcome: "已接收响应".into(),
            },
        ],
        usage: Some(Usage {
            state: UsageState::Reported,
            input_tokens: Some(100),
            output_tokens: Some(20),
            total_tokens: Some(120),
            cache_read_tokens: Some(80),
            cache_write_tokens: None,
            reasoning_tokens: None,
            reason: None,
        }),
        ..RequestLog::default()
    }
}
fn save(logs: &RequestLogs, row: RequestLog) {
    let usage = row.usage.clone();
    let id = logs.push(row);
    logs.finish(
        id,
        json!({"id":format!("resp-fixture-{id}"),"status":"completed"}),
        "已结束",
        usage,
    );
}
#[test]
fn dashboard_outcomes_cover_unknown_usage_and_all_pages_without_attributing_retries() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    let mut bad = entry(20, "failed-model");
    bad.usage = None;
    let failed_id = logs.push(bad);
    logs.finish(
        failed_id,
        json!({"status":"failed","error":{"code":"RateLimitReached"}}),
        "已结束",
        None,
    );
    for _ in 0..35 {
        save(&logs, entry(20, "good-model"));
    }
    let mut unknown = entry(20, "unknown-model");
    unknown.streaming = true;
    unknown.usage = None;
    let id = logs.push(unknown);
    logs.finish(id, json!({}), "已结束", None);
    let result = logs
        .query_history(&Query {
            page_size: 25,
            ..Query::default()
        })
        .unwrap();
    let points = result["requestTimeline"].as_array().unwrap();
    assert_eq!(
        points
            .iter()
            .map(|p| p["success"].as_u64().unwrap())
            .sum::<u64>(),
        35
    );
    assert_eq!(
        points
            .iter()
            .map(|p| p["failed"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
    assert_eq!(
        points
            .iter()
            .map(|p| p["pending"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
    assert_eq!(result["analytics"]["reported"], 35);
    assert_eq!(result["rows"].as_array().unwrap().len(), 25);
    assert_eq!(result["recentFailures"][0]["id"], failed_id);
    assert_eq!(result["recentFailures"][0]["code"], "RateLimitReached");
    let observed = result["providerObservations"].as_array().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0]["id"], "b");
    assert_eq!(observed[0]["records"].as_array().unwrap().len(), 20);
    let filtered = logs
        .query_history(&Query {
            status: "failed".into(),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(filtered["requestTimeline"][0]["failed"], 1);
    assert_eq!(filtered["requestTimeline"][0]["success"], 0);
}

#[test]
fn full_history_filters_and_totals_are_not_limited_by_page_or_200_rows() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    for i in 0..253 {
        save(
            &logs,
            entry(
                if i % 2 == 0 { 20 } else { 19 },
                if i % 2 == 0 { "Kimi-K3" } else { "gpt-6" },
            ),
        );
        if i % 30 == 0 {
            logs.flush().unwrap();
        }
    }
    let q = Query {
        model: "kimi".into(),
        provider: "a".into(),
        caller: "key:key-fixture".into(),
        status: "success".into(),
        ..Query::default()
    };
    let first = logs.query_history(&q).unwrap();
    assert_eq!(first["matched"], 127);
    assert_eq!(first["rows"].as_array().unwrap().len(), 50);
    assert_eq!(first["analytics"]["total"], 127 * 120);
    assert_eq!(first["cache"]["read"], 127 * 80);
    assert_eq!(first["cache"]["reported"], 127);
    assert_eq!(first["rows"][0]["id"], 252);
    let mut next = q.clone();
    next.page = 1;
    next.anchor = first["anchor"].as_u64();
    save(&logs, entry(20, "Kimi-K3"));
    let second = logs.query_history(&next).unwrap();
    assert_eq!(second["matched"], 127);
    assert_eq!(second["rows"][0]["id"], 152);
    assert_eq!(second["newerAvailable"], true);
    assert_eq!(first["analytics"], second["analytics"]);
    let day = chrono::DateTime::parse_from_rfc3339("2026-09-19T00:00:00Z")
        .unwrap()
        .timestamp_millis();
    let result = logs
        .query_history(&Query {
            from: Some(day),
            to: Some(day + 86_400_000),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(result["matched"], 126);
    let old = logs
        .query_history(&Query {
            search: "resp-fixture-0".into(),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(old["matched"], 1);
    let all = logs.query_history(&Query::default()).unwrap();
    assert_eq!(all["matched"], 254);
}
#[test]
fn literal_search_failure_codes_and_unknown_usage_are_honest() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    let mut bad = entry(20, "special%model");
    bad.streaming = true;
    bad.usage = None;
    let id = logs.push(bad);
    logs.finish(
        id,
        json!({"status":"failed","error":{"code":"RateLimitReached"}}),
        "已结束",
        None,
    );
    let result = logs
        .query_history(&Query {
            status: "failed".into(),
            search: "RateLimitReached".into(),
            model: "%".into(),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(result["matched"], 1);
    assert_eq!(result["stats"]["failed"], 1);
    assert!(result["analytics"]["total"].is_null());
    assert!(result["cache"]["read"].is_null());
    assert_eq!(
        logs.query_history(&Query {
            search: "%' OR 1=1 --".into(),
            ..Query::default()
        })
        .unwrap()["matched"],
        0
    );
    assert!(logs
        .query_history(&Query {
            from: Some(2),
            to: Some(1),
            ..Query::default()
        })
        .is_err());
}
#[test]
fn old_schema_backfill_and_legacy_writer_updates_do_not_rewrite_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("requests.sqlite3");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE request_logs(id INTEGER PRIMARY KEY,record_json TEXT NOT NULL,receiving INTEGER NOT NULL);PRAGMA user_version=1;").unwrap();
    let mut original = serde_json::to_value(entry(18, "legacy")).unwrap();
    original["id"] = json!(0);
    original["responseState"] = json!("已结束");
    let text = original.to_string();
    conn.execute("INSERT INTO request_logs VALUES(0,?1,0)", [&text])
        .unwrap();
    drop(conn);
    let logs = RequestLogs::open(temp.path()).unwrap();
    assert_eq!(logs.query_history(&Query::default()).unwrap()["matched"], 1);
    drop(logs);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT record_json FROM request_logs WHERE id=0", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        text
    );
    original["status"] = json!(429);
    conn.execute(
        "UPDATE request_logs SET record_json=?1 WHERE id=0",
        [original.to_string()],
    )
    .unwrap();
    drop(conn);
    let reopened = RequestLogs::open(temp.path()).unwrap();
    assert_eq!(
        reopened
            .query_history(&Query {
                status: "failed".into(),
                ..Query::default()
            })
            .unwrap()["matched"],
        1
    );
}
#[test]
fn oversized_totals_are_flagged_instead_of_rounded_or_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    for i in 0..201 {
        let mut row = entry(20, "huge");
        let u = row.usage.as_mut().unwrap();
        u.input_tokens = Some(MAX_ROW);
        u.output_tokens = Some(0);
        u.total_tokens = Some(MAX_ROW);
        u.cache_read_tokens = None;
        save(&logs, row);
        if i % 20 == 0 {
            logs.flush().unwrap();
        }
    }
    let result = logs.query_history(&Query::default()).unwrap();
    assert_eq!(result["overflow"], true);
    assert!(result["analytics"]["total"].is_null());
    assert_eq!(result["stats"]["total"], 201);
}

#[test]
fn completion_contract_matches_history_rows_filters_and_totals() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/manual/log-outcome-cases.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let conn = Connection::open(temp.path().join("requests.sqlite3")).unwrap();
    conn.execute_batch("CREATE TABLE request_logs(id INTEGER PRIMARY KEY,record_json TEXT NOT NULL,receiving INTEGER NOT NULL);PRAGMA user_version=1;").unwrap();
    for (id, case) in cases.iter().enumerate() {
        let mut row = serde_json::to_value(entry(20, case["case"].as_str().unwrap())).unwrap();
        row["id"] = json!(id);
        row["streaming"] = json!(true);
        row.as_object_mut()
            .unwrap()
            .extend(case["row"].as_object().unwrap().clone());
        conn.execute(
            "INSERT INTO request_logs VALUES(?1,?2,0)",
            params![id, row.to_string()],
        )
        .unwrap();
    }
    drop(conn);
    let logs = RequestLogs::open(temp.path()).unwrap();
    let all = logs.query_history(&Query::default()).unwrap();
    for row in all["rows"].as_array().unwrap() {
        let case = &cases[row["id"].as_u64().unwrap() as usize];
        assert_eq!(row["result"]["state"], case["state"], "{}", case["case"]);
    }
    for state in ["success", "failed", "pending"] {
        let expected: Vec<u64> = cases
            .iter()
            .enumerate()
            .filter(|(_, c)| c["state"] == state)
            .map(|(id, _)| id as u64)
            .collect();
        let filtered = logs
            .query_history(&Query {
                status: state.into(),
                ..Query::default()
            })
            .unwrap();
        let mut actual: Vec<u64> = filtered["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_u64().unwrap())
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert_eq!(all["stats"][state], expected.len());
    }
}

#[test]
fn stale_failure_projection_rebuild_preserves_raw_completed_record() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("requests.sqlite3");
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE request_logs(id INTEGER PRIMARY KEY,record_json TEXT NOT NULL,receiving INTEGER NOT NULL);PRAGMA user_version=1;").unwrap();
    let mut original = serde_json::to_value(entry(20, "legacy-completed")).unwrap();
    original["completion"] = json!("completed");
    original["responseState"] = json!("已中断");
    original["streaming"] = json!(true);
    let raw = original.to_string();
    conn.execute("INSERT INTO request_logs VALUES(0,?1,0)", [&raw])
        .unwrap();
    initialize(&mut conn).unwrap();
    conn.execute_batch("UPDATE manual_log_search_v1 SET outcome='failed',meta=json_set(meta,'$.outcome','failed','$.label','调用失败'); DROP TABLE IF EXISTS manual_log_projection_version;").unwrap();
    drop(conn);
    let logs = RequestLogs::open(temp.path()).unwrap();
    let result = logs.query_history(&Query::default()).unwrap();
    assert_eq!(
        result["stats"],
        json!({"total":1,"success":1,"failed":0,"pending":0})
    );
    assert_eq!(result["rows"][0]["responseState"], "已中断");
    assert_eq!(
        logs.query_history(&Query {
            status: "failed".into(),
            ..Query::default()
        })
        .unwrap()["matched"],
        0
    );
    drop(logs);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT record_json FROM request_logs WHERE id=0", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        raw
    );
    drop(conn);
    let reopened = RequestLogs::open(temp.path()).unwrap();
    assert_eq!(
        reopened.query_history(&Query::default()).unwrap()["stats"]["success"],
        1
    );
}

#[test]
fn default_cost_uses_recorded_response_model_and_entire_filtered_range() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    for _ in 0..35 {
        let mut row = entry(20, "requested-alias");
        row.usage.as_mut().unwrap().cache_write_tokens = Some(0);
        let usage = row.usage.clone();
        let id = logs.push(row);
        logs.finish(id, json!({"model":"gpt-6-astra"}), "已结束", usage);
    }
    save(&logs, entry(21, "FW-Kimi-K3"));
    let result = logs
        .query_history(&Query {
            model: "requested-alias".into(),
            page_size: 25,
            ..Query::default()
        })
        .unwrap();
    // Per request: (100-80)*10 + 80*1 + 20*50, divided by one million.
    assert_eq!(result["cost"]["totals"]["USD"], "0.0448");
    assert_eq!(result["cost"]["covered"], 35);
    let unknown = logs
        .query_history(&Query {
            model: "FW-Kimi-K3".into(),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(unknown["cost"]["covered"], 0);
    assert_eq!(unknown["cost"]["excluded"], 1);
    assert_eq!(unknown["cost"]["totals"], json!({}));
}

#[test]
fn default_cost_uses_final_route_on_failure_and_excludes_unforwarded_checks() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    let mut failed = entry(20, "gpt-6-astra");
    failed.effective_model = Some("gpt-4o-mini".into());
    failed.status = 500;
    save(&logs, failed);
    let mut directory = entry(20, "gpt-6-astra");
    directory.endpoint = "/v1/models".into();
    save(&logs, directory);
    let mut rejected = entry(20, "gpt-6-astra");
    rejected.providers.clear();
    save(&logs, rejected);
    let cost = logs.query_history(&Query::default()).unwrap()["cost"].clone();
    // 20 fresh * .15 + 80 cached * .075 + 20 output * .60, per million.
    assert_eq!(cost["totals"]["USD"], "0.000021");
    assert_eq!(cost["covered"], 1);
    assert_eq!(cost["excluded"], 0);
}

#[test]
fn legacy_estimate_includes_input_and_respects_range_without_rewriting_history() {
    let temp = tempfile::tempdir().unwrap();
    let logs = RequestLogs::open(temp.path()).unwrap();
    save(&logs, entry(18, "gpt-6-astra"));
    save(&logs, entry(24, "gpt-6-astra"));
    logs.flush().unwrap();
    let db = Connection::open(temp.path().join("requests.sqlite3")).unwrap();
    let original: String = db
        .query_row(
            "SELECT record_json FROM request_logs ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let all = logs.query_history(&Query::default()).unwrap();
    // Historical request-model equivalent: (20*10 + 80*1 + 20*50) / 1M each.
    assert_eq!(all["cost"]["totals"]["USD"], "0.00256");
    assert_eq!(all["cost"]["covered"], 2);
    assert_eq!(all["cost"]["estimated"], 2);
    let from = chrono::DateTime::parse_from_rfc3339("2026-09-24T00:00:00Z")
        .unwrap()
        .timestamp_millis();
    let recent = logs
        .query_history(&Query {
            from: Some(from),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(recent["cost"]["totals"]["USD"], "0.00128");
    assert_eq!(recent["cost"]["covered"], 1);
    let unchanged: String = db
        .query_row(
            "SELECT record_json FROM request_logs ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, unchanged);
}
