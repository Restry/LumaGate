//! Indexed, read-only history queries. Derived metadata never contains request/response text.
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
const SAFE: u128 = 9_007_199_254_740_991;
const MAX_ROW: u64 = 9_007_199_254_740_991 / 200;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Query {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub model: String,
    pub provider: String,
    pub caller: String,
    pub endpoint: String,
    pub status: String,
    pub search: String,
    pub page: u32,
    pub page_size: u32,
    pub anchor: Option<u64>,
    pub grouping: String,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            model: String::new(),
            provider: String::new(),
            caller: String::new(),
            endpoint: String::new(),
            status: String::new(),
            search: String::new(),
            page: 0,
            page_size: 50,
            anchor: None,
            grouping: "model".into(),
        }
    }
}
impl Query {
    pub fn validate(&self) -> Result<(), String> {
        if self.page_size == 0 || self.page_size > 100 || self.page > 1_000_000 {
            return Err("分页参数无效".into());
        }
        if [self.from, self.to]
            .into_iter()
            .flatten()
            .any(|v| !(-8_640_000_000_000_000..=8_640_000_000_000_000).contains(&v))
        {
            return Err("日期超出支持范围".into());
        }
        if self.from.zip(self.to).is_some_and(|(a, b)| a >= b) {
            return Err("开始日期必须早于结束日期".into());
        }
        if [
            &self.model,
            &self.provider,
            &self.caller,
            &self.endpoint,
            &self.status,
            &self.search,
        ]
        .iter()
        .any(|s| s.len() > 512)
        {
            return Err("查询条件过长".into());
        }
        if self.anchor.is_some_and(|id| id > SAFE as u64) {
            return Err("分页游标无效".into());
        }
        if !matches!(self.grouping.as_str(), "model" | "key") {
            return Err("分组参数无效".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    id: u64,
    at: Option<i64>,
    model: Option<String>,
    endpoint: String,
    http: u16,
    caller: String,
    caller_name: String,
    providers: Vec<(String, String)>,
    outcome: String,
    label: String,
    error_code: String,
    response_id: String,
    eligible: bool,
    phase: String,
    usage: Option<Counts>,
    #[serde(default)]
    meter: Option<crate::manual::pricing::Meter>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Counts {
    input: u64,
    output: u64,
    total: u64,
    cache: Option<u64>,
}
fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
fn project(id: u64, row: &Value) -> Meta {
    let response = row.get("response").unwrap_or(&Value::Null);
    let envelope_kind = text(response, "type");
    let response = response
        .get("response")
        .filter(|v| v.is_object())
        .unwrap_or(response);
    let error = response.get("error").filter(|v| !v.is_null());
    let code = error
        .and_then(|e| e.get("code").or_else(|| e.get("type")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let http = row.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
    let transport = text(row, "responseState");
    let completion = text(row, "completion");
    let delivery = row.get("delivery").filter(|v| v.is_object());
    let downstream = delivery.map(|v| text(v, "completion"));
    let closure = delivery.map(|v| text(v, "transport"));
    let status = text(response, "status");
    let legacy_finish = response
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|v| {
            !v.is_empty()
                && v.iter().all(|c| {
                    matches!(
                        c.get("finish_reason").and_then(Value::as_str),
                        Some("stop" | "tool_calls" | "function_call")
                    )
                })
        });
    let completed = match downstream.as_deref() {
        Some(value) => value == "completed",
        None => {
            completion == "completed"
                || (completion != "unknown" && (status == "completed" || legacy_finish))
        }
    };
    let (outcome, label) = if http == 429
        || matches!(
            code.as_str(),
            "rate_limit_exceeded" | "rate_limit_error" | "too_many_requests"
        ) {
        ("failed", "调用失败 · 限流（429）")
    } else if status == "cancelled" {
        ("failed", "调用失败 · 已取消")
    } else if !(200..300).contains(&http)
        || completion == "failed"
        || downstream.as_deref() == Some("failed")
        || status == "failed"
        || error.is_some()
        || matches!(envelope_kind.as_str(), "error" | "response.failed")
        || transport == "传输错误"
        || closure.as_deref() == Some("error")
    {
        ("failed", "调用失败")
    } else if status == "incomplete"
        || completion == "incomplete"
        || envelope_kind == "response.incomplete"
        || downstream.as_deref() == Some("incomplete")
    {
        ("failed", "调用失败 · 输出未完成")
    } else if transport.starts_with("调用失败") {
        ("failed", "调用失败")
    } else if transport == "接收中" {
        ("pending", "接收中")
    } else if completed
        && (transport.is_empty() || matches!(transport.as_str(), "已结束" | "已中断"))
    {
        (
            "success",
            if transport == "已中断" || closure.as_deref() == Some("dropped") {
                "协议完成 · 连接已关闭（交付未确认）"
            } else {
                "调用成功"
            },
        )
    } else if transport.starts_with("已中断") {
        ("failed", "调用失败")
    } else if delivery.is_some() || completion == "unknown" {
        (
            "pending",
            if completion == "completed" {
                "上游已完成 · 下游结束未确认"
            } else {
                "结束未确认"
            },
        )
    } else if row.get("streaming").and_then(Value::as_bool) != Some(true)
        && (transport.is_empty() || transport == "已结束")
    {
        ("success", "调用成功")
    } else {
        ("pending", "结果待确认")
    };
    let caller = row.get("caller").unwrap_or(&Value::Null);
    let kind = text(caller, "kind");
    let (caller_id, caller_name) = match kind.as_str() {
        "key" => (
            format!("key:{}", text(caller, "keyId")),
            text(caller, "name"),
        ),
        "local" => ("local".into(), "本机免鉴权".into()),
        "rejected" => ("rejected".into(), "鉴权拒绝".into()),
        _ => ("unknown".into(), "未记录的调用来源".into()),
    };
    let providers: Vec<_> = row
        .get("providers")
        .and_then(Value::as_array)
        .map(|v| v.iter().map(|p| (text(p, "id"), text(p, "name"))).collect())
        .unwrap_or_default();
    let endpoint = text(row, "endpoint");
    let eligible = matches!(
        endpoint.as_str(),
        "/v1/responses" | "/v1/responses/compact" | "/v1/chat/completions" | "/v1/messages"
    ) && (row.get("providers").is_none() || !providers.is_empty());
    let raw = row.get("usage").unwrap_or(&Value::Null);
    let count = |key| {
        raw.get(key)
            .and_then(Value::as_u64)
            .filter(|v| *v <= MAX_ROW)
    };
    let usage = if eligible && raw["state"] == "reported" {
        count("inputTokens")
            .zip(count("outputTokens"))
            .zip(count("totalTokens"))
            .and_then(|((input, output), total)| {
                (input + output == total).then(|| Counts {
                    input,
                    output,
                    total,
                    cache: count("cacheReadTokens").filter(|c| *c <= input),
                })
            })
    } else {
        None
    };
    let phase = if usage.is_some() {
        "reported"
    } else if transport == "接收中" || raw["state"] == "pending" {
        "pending"
    } else if raw["state"] == "partial" {
        "partial"
    } else {
        "unavailable"
    };
    // Only explicit recorded final identity; never infer it from a requested alias.
    let model = row
        .get("effectiveModel")
        .and_then(Value::as_str)
        .or_else(|| response.get("model").and_then(Value::as_str))
        .filter(|s| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control));
    let meter = (eligible && !providers.is_empty()).then(|| crate::manual::pricing::Meter {
        model: model.map(str::to_owned),
        requested: row.get("model").and_then(Value::as_str).map(str::to_owned),
        input: usage.as_ref().map(|u| u.input),
        output: usage.as_ref().map(|u| u.output),
        read: usage.as_ref().and_then(|u| u.cache),
        write: count("cacheWriteTokens"),
    });
    Meta {
        id,
        at: chrono::DateTime::parse_from_rfc3339(&text(row, "startedAt"))
            .ok()
            .map(|v| v.timestamp_millis()),
        model: row.get("model").and_then(Value::as_str).map(str::to_string),
        endpoint,
        http,
        caller: caller_id,
        caller_name,
        providers,
        outcome: outcome.into(),
        label: label.into(),
        error_code: code,
        response_id: text(response, "id"),
        eligible,
        phase: phase.into(),
        usage,
        meter,
    }
}

pub(super) fn initialize(conn: &mut Connection) -> rusqlite::Result<()> {
    // Version only the disposable projection; request_logs/user_version stay untouched.
    // Mark all rows dirty atomically before publishing the version, so interrupted
    // backfills resume through the existing bounded dirty queue on the next open.
    let tx = conn.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS manual_log_search_v1(id INTEGER PRIMARY KEY, at INTEGER, model TEXT, endpoint TEXT, caller TEXT, outcome TEXT, http INTEGER, error_code TEXT, response_id TEXT, meta TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS manual_log_time_v1 ON manual_log_search_v1(at,id);
      CREATE INDEX IF NOT EXISTS manual_log_outcome_v1 ON manual_log_search_v1(outcome,id);
      CREATE INDEX IF NOT EXISTS manual_log_caller_v1 ON manual_log_search_v1(caller,id);
      CREATE TABLE IF NOT EXISTS manual_log_dirty_v1(id INTEGER PRIMARY KEY);
      CREATE TRIGGER IF NOT EXISTS manual_log_insert_v1 AFTER INSERT ON request_logs BEGIN INSERT OR IGNORE INTO manual_log_dirty_v1 VALUES(new.id); END;
      CREATE TRIGGER IF NOT EXISTS manual_log_update_v1 AFTER UPDATE ON request_logs BEGIN INSERT OR IGNORE INTO manual_log_dirty_v1 VALUES(new.id); END;
      CREATE TRIGGER IF NOT EXISTS manual_log_delete_v1 AFTER DELETE ON request_logs BEGIN DELETE FROM manual_log_search_v1 WHERE id=old.id; DELETE FROM manual_log_dirty_v1 WHERE id=old.id; END;
      CREATE TABLE IF NOT EXISTS manual_log_projection_version(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);")?;
    let version: Option<u32> = tx
        .query_row(
            "SELECT version FROM manual_log_projection_version WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if version != Some(3) {
        tx.execute(
            "INSERT OR IGNORE INTO manual_log_dirty_v1 SELECT id FROM request_logs",
            [],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO manual_log_projection_version VALUES(1,3)",
            [],
        )?;
    }
    tx.execute("INSERT OR IGNORE INTO manual_log_dirty_v1 SELECT r.id FROM request_logs r LEFT JOIN manual_log_search_v1 s ON s.id=r.id WHERE s.id IS NULL",[])?;
    tx.commit()?;
    sync_dirty(conn)
}
pub(super) fn sync_dirty(conn: &mut Connection) -> rusqlite::Result<()> {
    loop {
        let tx = conn.transaction()?;
        let ids = {
            let mut s = tx.prepare("SELECT id FROM manual_log_dirty_v1 ORDER BY id LIMIT 200")?;
            let v = s
                .query_map([], |r| r.get::<_, u64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            sync_one(&tx, id)?;
        }
        tx.commit()?;
    }
}
pub(super) fn sync_one(conn: &Connection, id: u64) -> rusqlite::Result<()> {
    let raw: String = conn.query_row(
        "SELECT record_json FROM request_logs WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    let row: Value = serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if !row.is_object() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let m = project(id, &row);
    conn.execute(
        "INSERT OR REPLACE INTO manual_log_search_v1 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            m.at,
            m.model,
            m.endpoint,
            m.caller,
            m.outcome,
            m.http,
            m.error_code,
            m.response_id,
            serde_json::to_string(&m).map_err(|_| rusqlite::Error::InvalidQuery)?
        ],
    )?;
    conn.execute("DELETE FROM manual_log_dirty_v1 WHERE id=?1", [id])?;
    Ok(())
}
fn like(s: &str) -> String {
    format!(
        "%{}%",
        s.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}
fn conditions(q: &Query, anchor: u64) -> (String, Vec<Sql>) {
    let mut clauses = vec!["s.id<=?".to_string()];
    let mut args = vec![Sql::Integer(anchor as i64)];
    for (sql, value) in [("s.at>=?", q.from), ("s.at<?", q.to)] {
        if let Some(v) = value {
            clauses.push(sql.into());
            args.push(Sql::Integer(v));
        }
    }
    for (sql, value) in [
        (
            "s.model LIKE ? ESCAPE '\\'",
            if q.model.trim().is_empty() {
                None
            } else {
                Some(like(q.model.trim()))
            },
        ),
        (
            "s.endpoint=?",
            (!q.endpoint.is_empty() && q.endpoint != "all").then(|| q.endpoint.clone()),
        ),
        (
            "s.caller=?",
            (!q.caller.is_empty() && q.caller != "all").then(|| q.caller.clone()),
        ),
    ] {
        if let Some(v) = value {
            clauses.push(sql.into());
            args.push(Sql::Text(v));
        }
    }
    if !q.provider.is_empty() && q.provider != "all" {
        clauses.push("EXISTS(SELECT 1 FROM json_each(s.meta,'$.providers') p WHERE json_extract(p.value,'$[0]')=?)".into());
        args.push(Sql::Text(q.provider.clone()));
    }
    if matches!(q.status.as_str(), "success" | "failed" | "pending") {
        clauses.push("s.outcome=?".into());
        args.push(Sql::Text(q.status.clone()));
    } else if let Ok(code) = q.status.parse::<u16>() {
        clauses.push("s.http=?".into());
        args.push(Sql::Integer(code as i64));
    } else if !q.status.is_empty() && q.status != "all" {
        clauses.push("0".into());
    }
    if !q.search.trim().is_empty() {
        clauses.push("(s.error_code LIKE ? ESCAPE '\\' OR s.response_id LIKE ? ESCAPE '\\' OR CAST(s.id AS TEXT)=?)".into());
        args.extend([
            Sql::Text(like(q.search.trim())),
            Sql::Text(like(q.search.trim())),
            Sql::Text(q.search.trim().trim_start_matches('#').into()),
        ]);
    }
    (clauses.join(" AND "), args)
}
#[derive(Default)]
struct Sum {
    input: u128,
    output: u128,
    total: u128,
    requests: u64,
}
impl Sum {
    fn add(&mut self, u: &Counts) {
        self.input += u.input as u128;
        self.output += u.output as u128;
        self.total += u.total as u128;
        self.requests += 1;
    }
}
fn number(value: u128, overflow: &mut bool) -> Value {
    if value > SAFE {
        *overflow = true;
        Value::Null
    } else {
        json!(value as u64)
    }
}
fn point(id: &str, label: &str, sum: &Sum, overflow: &mut bool) -> Value {
    json!({"id":id,"label":label,"input":number(sum.input,overflow),"output":number(sum.output,overflow),"total":number(sum.total,overflow),"requests":sum.requests})
}
fn interval(span: i64) -> i64 {
    [60_000, 300_000, 900_000, 3_600_000, 21_600_000, 86_400_000]
        .into_iter()
        .find(|step| span / step < 48)
        .unwrap_or(((span / 48 / 86_400_000) + 1) * 86_400_000)
}

pub(super) fn query(conn: &mut Connection, q: &Query) -> Result<Value, String> {
    q.validate()?;
    query_inner(conn, q)
        .map_err(|_| "历史日志查询失败，请检查日志库、权限和磁盘空间；历史记录未被删除".into())
}
fn query_inner(conn: &mut Connection, q: &Query) -> rusqlite::Result<Value> {
    let tx = conn.transaction()?;
    let latest: u64 = tx.query_row("SELECT COALESCE(MAX(id),0) FROM request_logs", [], |r| {
        r.get(0)
    })?;
    let anchor = q.anchor.unwrap_or(latest).min(latest);
    let (filter, args) = conditions(q, anchor);
    let bounds = tx.query_row(
        &format!("SELECT MIN(at),MAX(at) FROM manual_log_search_v1 s WHERE {filter}"),
        params_from_iter(args.iter()),
        |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
    )?;
    let step = interval(
        bounds
            .0
            .zip(bounds.1)
            .map(|(a, b)| b.saturating_sub(a))
            .unwrap_or(0),
    );
    let mut matched = 0u64;
    let (mut success, mut failed, mut pending) = (0u64, 0u64, 0u64);
    let (mut eligible, mut waiting, mut partial, mut unavailable, mut invalid_times) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut sum = Sum::default();
    let (catalog, price_status) = crate::manual::pricing::SERVICE.snapshot();
    let mut cost = crate::manual::pricing::Estimate::default();
    let (mut cache_count, mut cache_input, mut cache_read) = (0u64, 0u128, 0u128);
    let mut timeline: BTreeMap<i64, Sum> = BTreeMap::new();
    // Outcome trends include every matching request, not just complete token reports.
    let mut outcome_timeline: BTreeMap<i64, [u64; 3]> = BTreeMap::new();
    let mut observations: BTreeMap<String, (String, Vec<Value>)> = BTreeMap::new();
    let mut recent_failures: Vec<Value> = Vec::new();
    let mut ranked: HashMap<String, (String, Sum)> = HashMap::new();
    {
        let mut stmt = tx.prepare(&format!(
            "SELECT meta FROM manual_log_search_v1 s WHERE {filter} ORDER BY id DESC"
        ))?;
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        while let Some(row) = rows.next()? {
            let m: Meta = serde_json::from_str(&row.get::<_, String>(0)?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            matched += 1;
            if let Some(meter) = &m.meter {
                cost.add(&catalog.catalog, meter);
            }
            match m.outcome.as_str() {
                "success" => success += 1,
                "failed" => failed += 1,
                _ => pending += 1,
            };
            let outcome_index = match m.outcome.as_str() {
                "success" => 0,
                "failed" => 1,
                _ => 2,
            };
            if let Some(at) = m.at {
                let bucket =
                    (at - q.from.unwrap_or(0)).div_euclid(step) * step + q.from.unwrap_or(0);
                outcome_timeline.entry(bucket).or_default()[outcome_index] += 1;
            }
            // This is the request's final provider, never a claim about every retry attempt.
            if let Some((id, name)) = m.providers.last() {
                if observations.len() < 1000 || observations.contains_key(id) {
                    let entry = observations
                        .entry(id.clone())
                        .or_insert_with(|| (name.clone(), Vec::new()));
                    if entry.1.len() < 20 {
                        entry.1.push(json!({"id":m.id,"at":m.at,"state":m.outcome}));
                    }
                }
            }
            if m.outcome == "failed" && recent_failures.len() < 3 {
                recent_failures.push(json!({"id":m.id,"at":m.at,"model":m.model,"provider":m.providers.last().map(|p|p.1.as_str()),"http":m.http,"code":m.error_code}));
            }
            if !m.eligible {
                continue;
            }
            eligible += 1;
            if let Some(u) = m.usage {
                sum.add(&u);
                if let Some(c) = u.cache {
                    cache_count += 1;
                    cache_input += u.input as u128;
                    cache_read += c as u128;
                }
                if let Some(at) = m.at {
                    timeline
                        .entry(
                            (at - q.from.unwrap_or(0)).div_euclid(step) * step
                                + q.from.unwrap_or(0),
                        )
                        .or_default()
                        .add(&u);
                } else {
                    invalid_times += 1;
                }
                let (mut id, mut label) = if q.grouping == "key" {
                    (
                        m.caller.clone(),
                        if m.caller.starts_with("key:") {
                            format!(
                                "{} ({})",
                                m.caller_name,
                                m.caller
                                    .trim_start_matches("key:")
                                    .chars()
                                    .take(8)
                                    .collect::<String>()
                            )
                        } else {
                            m.caller_name
                        },
                    )
                } else {
                    (
                        format!("model:{}", m.model.as_deref().unwrap_or("")),
                        m.model.unwrap_or("未识别模型".into()),
                    )
                };
                if ranked.len() >= 10_000 && !ranked.contains_key(&id) {
                    id = "overflow-groups".into();
                    label = "其他分组".into();
                }
                ranked
                    .entry(id)
                    .or_insert_with(|| (label, Sum::default()))
                    .1
                    .add(&u);
            } else {
                match m.phase.as_str() {
                    "pending" => waiting += 1,
                    "partial" => partial += 1,
                    _ => unavailable += 1,
                }
            }
        }
    }
    let pages = matched.div_ceil(q.page_size as u64);
    let page = (q.page as u64).min(pages.saturating_sub(1));
    let mut page_args = args.clone();
    page_args.extend([
        Sql::Integer(q.page_size as i64),
        Sql::Integer((page * q.page_size as u64) as i64),
    ]);
    let mut data = vec![];
    {
        let mut stmt=tx.prepare(&format!("SELECT r.record_json,s.meta FROM request_logs r JOIN manual_log_search_v1 s ON r.id=s.id WHERE {filter} ORDER BY s.id DESC LIMIT ? OFFSET ?"))?;
        let mut rows = stmt.query(params_from_iter(page_args.iter()))?;
        while let Some(row) = rows.next()? {
            let mut value: Value = serde_json::from_str(&row.get::<_, String>(0)?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let meta: Meta = serde_json::from_str(&row.get::<_, String>(1)?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            value["result"] = json!({"state":meta.outcome,"label":meta.label});
            data.push(value);
        }
    }
    let mut overflow = false;
    let mut ranking: Vec<_> = ranked.into_iter().collect();
    ranking.sort_by(|a, b| b.1 .1.total.cmp(&a.1 .1.total).then(a.0.cmp(&b.0)));
    let mut distribution: Vec<_> = ranking
        .iter()
        .take(5)
        .map(|(id, (label, s))| point(id, label, s, &mut overflow))
        .collect();
    if ranking.len() > 5 {
        let mut other = Sum::default();
        for (_, (_, s)) in ranking.iter().skip(5) {
            other.input += s.input;
            other.output += s.output;
            other.total += s.total;
            other.requests += s.requests;
        }
        distribution.push(point(
            "other",
            &format!("其他（{}项）", ranking.len() - 5),
            &other,
            &mut overflow,
        ));
    }
    let ranking_points: Vec<_> = ranking
        .iter()
        .map(|(id, (label, s))| point(id, label, s, &mut overflow))
        .collect();
    let mut points = vec![];
    if let (Some(first), Some(last)) = (
        timeline.keys().next().copied(),
        timeline.keys().next_back().copied(),
    ) {
        let mut at = first;
        while at <= last && points.len() < 64 {
            let empty = Sum::default();
            let v = point(
                &at.to_string(),
                &at.to_string(),
                timeline.get(&at).unwrap_or(&empty),
                &mut overflow,
            );
            points.push(v);
            at = at.saturating_add(step);
        }
    }
    let mut request_points = vec![];
    if let (Some(first), Some(last)) = (
        outcome_timeline.keys().next().copied(),
        outcome_timeline.keys().next_back().copied(),
    ) {
        let mut at = first;
        while at <= last && request_points.len() < 64 {
            let counts = outcome_timeline.get(&at).copied().unwrap_or_default();
            request_points
                .push(json!({"at":at,"success":counts[0],"failed":counts[1],"pending":counts[2]}));
            at = at.saturating_add(step);
        }
    }
    let provider_observations: Vec<_> = observations
        .into_iter()
        .map(|(id, (name, records))| json!({"id":id,"name":name,"records":records}))
        .collect();
    let total = if sum.requests > 0 {
        number(sum.total, &mut overflow)
    } else {
        Value::Null
    };
    let input = if sum.requests > 0 {
        number(sum.input, &mut overflow)
    } else {
        Value::Null
    };
    let output = if sum.requests > 0 {
        number(sum.output, &mut overflow)
    } else {
        Value::Null
    };
    let cache = json!({"complete":sum.requests,"reported":cache_count,"read":if cache_count>0{number(cache_read,&mut overflow)}else{Value::Null},"nonRead":if cache_count>0{number(cache_input-cache_read,&mut overflow)}else{Value::Null},"share":if cache_count>0&&cache_input>0&&cache_input<=SAFE{Some(cache_read as f64/cache_input as f64)}else{None}});
    let mut providers: HashMap<String, String> = HashMap::new();
    let mut callers: HashMap<String, String> = HashMap::new();
    let mut endpoints = BTreeMap::new();
    let mut statuses = BTreeMap::new();
    {
        let mut stmt = tx.prepare("SELECT meta FROM manual_log_search_v1 ORDER BY id DESC")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let m: Meta = serde_json::from_str(&row.get::<_, String>(0)?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            for (id, name) in m.providers {
                if providers.len() < 1000 {
                    providers.entry(id).or_insert(name);
                }
            }
            if callers.len() < 1000 {
                callers.entry(m.caller).or_insert(m.caller_name);
            }
            endpoints.insert(m.endpoint, ());
            statuses.insert(m.http, ());
        }
    }
    let options = |map: HashMap<String, String>| {
        let mut values: Vec<_> = map
            .into_iter()
            .map(|(value, label)| json!({"value":value,"label":label}))
            .collect();
        values.sort_by(|a, b| a["label"].as_str().cmp(&b["label"].as_str()));
        values
    };
    Ok(
        json!({"rows":data,"matched":matched,"page":page,"pageSize":q.page_size,"pages":pages,"anchor":anchor,"newerAvailable":latest>anchor,
      "stats":{"total":matched,"success":success,"failed":failed,"pending":pending},
      "cost":cost.value(price_status),
      "requestTimeline":request_points,"providerObservations":provider_observations,"recentFailures":recent_failures,
      "analytics":{"eligible":eligible,"reported":sum.requests,"pending":waiting,"partial":partial,"unavailable":unavailable,"input":input,"output":output,"total":total,"timeline":points,"intervalMs":step,"bounds":bounds,"intervalLabel":if step<3_600_000{format!("{} 分钟",step/60_000)}else if step<86_400_000{format!("{} 小时",step/3_600_000)}else{format!("{} 天",step/86_400_000)},"rangeLabel":"","invalidTimes":invalid_times,"distribution":distribution,"ranking":ranking_points},
      "cache":cache,"overflow":overflow,"options":{"providers":options(providers),"callers":options(callers),"endpoints":endpoints.keys().collect::<Vec<_>>(),"statuses":statuses.keys().collect::<Vec<_>>()}}),
    )
}

#[cfg(test)]
mod tests;
