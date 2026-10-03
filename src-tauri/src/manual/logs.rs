use super::log_preview::{self, Collector};
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use http_body::Body as HttpBody;
use serde::Serialize;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::Instant,
};
const CAPACITY: usize = 200;
pub mod history;
#[cfg(test)]
mod persistence_tests;
mod storage;

tokio::task_local! { static TRACE: RefCell<Trace>; }
#[derive(Default)]
struct Trace {
    model: Option<String>,
    caller: Caller,
    input: Value,
    route_note: Option<String>,
    providers: Vec<ProviderAttempt>,
    secrets: Vec<String>,
    usage: super::token_usage::Tracker,
}
pub fn usage_tracker() -> Option<super::token_usage::Tracker> {
    TRACE.try_with(|slot| slot.borrow().usage.clone()).ok()
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAttempt {
    pub id: String,
    pub name: String,
    pub outcome: String,
}
#[derive(Clone, Default, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Caller {
    #[default]
    Unknown,
    Local,
    Rejected,
    Key {
        #[serde(rename = "keyId")]
        key_id: String,
        name: String,
    },
}
impl Caller {
    pub fn local() -> Self {
        Self::Local
    }
    pub fn rejected() -> Self {
        Self::Rejected
    }
    pub fn key(identity: super::network::access_keys::Identity) -> Self {
        Self::Key {
            key_id: identity.id,
            name: identity.name,
        }
    }
}
pub fn set_caller(caller: Caller) {
    let _ = TRACE.try_with(|slot| slot.borrow_mut().caller = caller);
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLog {
    pub id: u64,
    pub started_at: String,
    pub endpoint: String,
    pub model: Option<String>,
    pub caller: Caller,
    pub status: u16,
    pub response_ms: u64,
    pub streaming: bool,
    pub providers: Vec<ProviderAttempt>,
    pub route_note: Option<String>,
    pub input: Value,
    pub response: Value,
    pub response_state: String,
    pub usage: Option<super::token_usage::Usage>,
    pub completion: Option<super::completion::Completion>,
}
#[derive(Default)]
pub struct RequestLogs {
    sequence: AtomicU64,
    rows: Mutex<VecDeque<RequestLog>>,
    store: Option<storage::Store>,
    storage_error: Option<String>,
}
impl RequestLogs {
    pub fn open(directory: &std::path::Path) -> Result<Self, String> {
        let (store, next) = storage::Store::open(directory)?;
        Ok(Self {
            sequence: AtomicU64::new(next),
            rows: Mutex::new(VecDeque::new()),
            store: Some(store),
            storage_error: None,
        })
    }
    pub fn unavailable(error: String) -> Self {
        Self {
            storage_error: Some(error),
            ..Self::default()
        }
    }
    pub fn ready(&self) -> bool {
        self.storage_error.is_none() && self.store.as_ref().is_none_or(|s| s.healthy())
    }
    pub fn storage_status(&self) -> Value {
        serde_json::json!({"ready":self.ready(),"path":crate::config::get_app_config_dir().join("logs").join("requests.sqlite3"),
            "message":if self.ready() { "日志存储可用" } else { "日志库不可用：可能被另一实例占用，或存在权限、磁盘空间、文件损坏问题。历史文件未被删除。" }})
    }
    pub fn query_history(&self, query: &history::Query) -> Result<Value, String> {
        if !self.ready() {
            return Err("日志存储不可用，请先处理存储问题".into());
        }
        self.store
            .as_ref()
            .ok_or_else(|| "未配置历史日志库".to_string())?
            .query(query)
    }
    pub fn persisted_snapshot(&self) -> Result<Vec<Value>, String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        match &self.store {
            Some(store) => store.snapshot(),
            None => self
                .snapshot()
                .iter()
                .map(|row| serde_json::to_value(row).map_err(|_| "日志编码失败".to_string()))
                .collect(),
        }
    }
    pub fn flush(&self) -> Result<(), String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        self.store.as_ref().map_or(Ok(()), |store| store.flush())
    }
    fn push(&self, mut row: RequestLog) -> u64 {
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        row.id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let id = row.id;
        if let Some(store) = &self.store {
            store.insert(&row);
        }
        rows.push_front(row);
        rows.truncate(CAPACITY);
        id
    }
    #[cfg(test)]
    fn finish(
        &self,
        id: u64,
        response: Value,
        state: &str,
        usage: Option<super::token_usage::Usage>,
    ) {
        self.finish_observed(id, response, state, usage, None);
    }
    fn finish_observed(
        &self,
        id: u64,
        response: Value,
        state: &str,
        usage: Option<super::token_usage::Usage>,
        completion: Option<super::completion::Completion>,
    ) {
        // Stream termination is not proof of model success. Preserve the actual HTTP code,
        // but persist a failed outcome for explicit protocol errors (also after headers).
        let envelope = response.get("response").unwrap_or(&response);
        let error = envelope.get("error").filter(|error| !error.is_null());
        let code = error
            .and_then(|error| error.get("code").or_else(|| error.get("type")))
            .and_then(Value::as_str);
        let state = if matches!(
            code,
            Some("rate_limit_exceeded" | "rate_limit_error" | "too_many_requests")
        ) {
            "调用失败 · 限流（429）"
        } else if envelope.get("status").and_then(Value::as_str) == Some("incomplete") {
            "调用失败 · 输出未完成"
        } else if error.is_some()
            || matches!(
                envelope.get("status").and_then(Value::as_str),
                Some("failed" | "cancelled")
            )
            || completion == Some(super::completion::Completion::Failed)
        {
            "调用失败"
        } else if completion == Some(super::completion::Completion::Incomplete) {
            "调用失败 · 输出未完成"
        } else {
            state
        };
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(store) = &self.store {
            store.finish(
                id,
                response.clone(),
                state,
                serde_json::to_value(&usage).expect("usage is serializable"),
                serde_json::to_value(completion).expect("completion is serializable"),
            );
        }
        if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
            row.response = response;
            row.response_state = state.into();
            row.usage = usage;
            row.completion = completion;
        }
    }
    pub fn snapshot(&self) -> Vec<RequestLog> {
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}
pub fn identify_model(doc: &super::catalog::Document, body: &Value) {
    let _ = TRACE.try_with(|slot| {
        let mut trace = slot.borrow_mut();
        trace.model = body.get("model").and_then(Value::as_str).and_then(|id| {
            super::catalog::resolve_group(&doc.groups(), id).map(|group| {
                group
                    .model
                    .id
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect()
            })
        });
        trace.input = log_preview::last_turn(body);
    });
}
pub fn route_note(note: &str) {
    let _ = TRACE.try_with(|slot| slot.borrow_mut().route_note = Some(note.into()));
}
pub fn attempt(provider: &crate::provider::Provider) {
    let _ = TRACE.try_with(|slot| {
        if provider
            .settings_config
            .get("manual_upstream_model")
            .is_none()
        {
            return;
        }
        let mut trace = slot.borrow_mut();
        for path in ["/auth/OPENAI_API_KEY", "/env/ANTHROPIC_AUTH_TOKEN"] {
            if let Some(secret) = provider
                .settings_config
                .pointer(path)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                if !trace.secrets.iter().any(|value| value == secret) {
                    trace.secrets.push(secret.into());
                }
            }
        }
        if trace.providers.len() < 8 {
            trace.providers.push(ProviderAttempt {
                id: provider.id.clone(),
                name: provider.name.clone(),
                outcome: "正在尝试".into(),
            });
        }
    });
}
pub fn attempt_succeeded() {
    let _ = TRACE.try_with(|slot| {
        if let Some(provider) = slot.borrow_mut().providers.last_mut() {
            provider.outcome = "已接收响应".into();
        }
    });
}
pub fn attempt_failed(error: &crate::proxy::ProxyError) {
    let outcome = match error {
        crate::proxy::ProxyError::UpstreamError { status, .. } => format!("失败（HTTP {status}）"),
        _ => "尝试失败".into(),
    };
    let _ = TRACE.try_with(|slot| {
        if let Some(provider) = slot.borrow_mut().providers.last_mut() {
            provider.outcome = outcome;
        }
    });
}

struct CaptureBody {
    body: Pin<Box<Body>>,
    logs: Arc<RequestLogs>,
    id: u64,
    collector: Collector,
    usage: super::token_usage::Tracker,
    secrets: Vec<String>,
    finished: bool,
    streaming: bool,
}
impl CaptureBody {
    fn finish(&mut self, state: &str) {
        if !self.finished {
            self.finished = true;
            if state != "已结束" {
                self.collector.interrupted();
            }
            let response = self.collector.finish(&self.secrets);
            // 客户端读到协议结束标记后可能主动关流，不应把完整回答标成中断。
            let state = if state == "已中断" && self.collector.terminal_seen() {
                "已结束"
            } else {
                state
            };
            let observed = self.usage.completion();
            let completion = (self.streaming || observed != super::completion::Completion::Unknown)
                .then_some(observed);
            self.logs.finish_observed(
                self.id,
                response,
                state,
                self.usage.closed_snapshot(),
                completion,
            );
        }
    }
}
impl HttpBody for CaptureBody {
    type Data = bytes::Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let polled = this.body.as_mut().poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.collector.feed(data);
                }
                if this.body.is_end_stream() {
                    this.finish("已结束");
                }
            }
            Poll::Ready(Some(Err(_))) => this.finish("传输错误"),
            Poll::Ready(None) => this.finish("已结束"),
            Poll::Pending => {}
        }
        polled
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}
impl Drop for CaptureBody {
    fn drop(&mut self) {
        self.finish("已中断");
    }
}

pub async fn capture(
    State(logs): State<Arc<RequestLogs>>,
    request: Request,
    next: Next,
) -> Response {
    let endpoint = match request.uri().path() {
        "/v1/models" => "/v1/models",
        "/v1/messages" => "/v1/messages",
        "/v1/chat/completions" => "/v1/chat/completions",
        "/v1/responses" => "/v1/responses",
        "/v1/responses/compact" => "/v1/responses/compact",
        _ => "未知接口",
    }
    .to_owned();
    let mut secrets = Vec::new();
    for name in ["authorization", "x-api-key"] {
        for header in request.headers().get_all(name) {
            if let Ok(value) = header.to_str() {
                secrets.push(value.to_owned());
                if name == "authorization" {
                    if let Some((scheme, value)) = value.split_once(' ') {
                        if scheme.eq_ignore_ascii_case("bearer") {
                            secrets.push(value.to_owned());
                        }
                    }
                }
            }
        }
    }
    let started_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let started = Instant::now();
    TRACE
        .scope(
            RefCell::new(Trace {
                secrets,
                ..Trace::default()
            }),
            async move {
                let response = next.run(request).await;
                let streaming = response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("text/event-stream")
                    });
                let trace = TRACE.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
                let id = logs.push(RequestLog {
                    id: 0,
                    started_at,
                    endpoint,
                    model: trace.model,
                    caller: trace.caller,
                    status: response.status().as_u16(),
                    response_ms: started.elapsed().as_millis() as u64,
                    streaming,
                    input: log_preview::sanitize(&trace.input, &trace.secrets),
                    providers: trace.providers,
                    route_note: trace.route_note,
                    response: Value::Null,
                    response_state: "接收中".into(),
                    usage: trace.usage.snapshot(),
                    completion: None,
                });
                let (parts, body) = response.into_parts();
                // 按原始帧旁路观察，保留字节、背压和 trailers，不等待完整输出才向客户端发送。
                let mut tap = CaptureBody {
                    body: Box::pin(body),
                    logs,
                    id,
                    collector: Collector::new(streaming),
                    streaming,
                    usage: trace.usage,
                    secrets: trace.secrets,
                    finished: false,
                };
                if tap.body.is_end_stream() {
                    tap.finish("已结束");
                }
                Response::from_parts(parts, Body::new(tap))
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn capture_preserves_body_bytes_and_trailers() {
        use http_body_util::{BodyExt, StreamBody};
        let mut trailers = axum::http::HeaderMap::new();
        trailers.insert("x-fixture", "yes".parse().unwrap());
        let frames = futures::stream::iter(vec![
            Ok::<_, std::convert::Infallible>(http_body::Frame::data(bytes::Bytes::from_static(
                b"{\"text\":\"OK\"}",
            ))),
            Ok(http_body::Frame::trailers(trailers)),
        ]);
        let logs = Arc::new(RequestLogs::default());
        let id = logs.push(RequestLog::default());
        let tap = CaptureBody {
            body: Box::pin(Body::new(StreamBody::new(frames))),
            logs: logs.clone(),
            id,
            collector: Collector::new(false),
            streaming: false,
            usage: super::super::token_usage::Tracker::default(),
            secrets: vec![],
            finished: false,
        };
        let collected = tap.collect().await.unwrap();
        assert_eq!(collected.trailers().unwrap()["x-fixture"], "yes");
        assert_eq!(collected.to_bytes(), "{\"text\":\"OK\"}");
        assert_eq!(logs.snapshot()[0].response_state, "已结束");
    }
    #[test]
    fn memory_log_is_bounded_and_newest_first() {
        let logs = RequestLogs::default();
        for index in 0..250 {
            logs.push(RequestLog {
                started_at: index.to_string(),
                endpoint: "/v1/responses".into(),
                status: 400,
                ..RequestLog::default()
            });
        }
        let rows = logs.snapshot();
        assert_eq!(rows.len(), 200);
        assert_eq!(rows[0].started_at, "249");
        assert_eq!(rows[199].started_at, "50");
    }
}
