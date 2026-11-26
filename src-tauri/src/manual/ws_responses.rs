//! Codex Responses WebSocket bridge (`GET /backend-api/codex/responses` & `GET /v1/responses`).
//!
//! Accepts WebSocket `response.create` frames from Codex Desktop / CLI and executes each turn
//! through LumaGate's standard HTTP POST `/responses` router (preserving auth, logging, drain
//! gating, Copilot/Codex/Gemini protocol transforms, and token accounting), streaming every SSE
//! event back over the WebSocket in real time.

use axum::{
    body::Body,
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension,
    },
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Uri},
    response::Response,
    Router,
};
use futures::StreamExt;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
};
use tower::Service;

const MAX_REMEMBERED_RESPONSES: usize = 64;

#[derive(Clone)]
pub struct WsBridgeState {
    http_router: Router,
}

impl WsBridgeState {
    pub fn new(http_router: Router) -> Self {
        Self { http_router }
    }
}

pub async fn handle_ws(
    Extension(bridge): Extension<WsBridgeState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| drive_socket(socket, bridge, peer, headers, uri))
}

#[derive(Default)]
struct TurnHistory {
    by_id: HashMap<String, Vec<Value>>,
    order: VecDeque<String>,
}

impl TurnHistory {
    fn remember(&mut self, response_id: String, items: Vec<Value>) {
        if response_id.is_empty() {
            return;
        }
        if !self.by_id.contains_key(&response_id) {
            self.order.push_back(response_id.clone());
        }
        self.by_id.insert(response_id, items);
        while self.order.len() > MAX_REMEMBERED_RESPONSES {
            if let Some(evicted) = self.order.pop_front() {
                self.by_id.remove(&evicted);
            }
        }
    }

    fn get(&self, response_id: &str) -> Option<&Vec<Value>> {
        self.by_id.get(response_id)
    }
}

fn input_value_to_items(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(_)) => value.cloned().into_iter().collect(),
        Some(Value::String(text)) => vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        })],
        _ => Vec::new(),
    }
}

fn prepare_http_payload(
    mut payload: Value,
    history: &TurnHistory,
) -> Result<(Value, Vec<Value>), Value> {
    let Some(obj) = payload.as_object_mut() else {
        return Err(ws_error_event(
            400,
            "invalid_request_error",
            "WebSocket frame must be a JSON object",
        ));
    };

    obj.remove("type");
    obj.remove("generate");
    obj.insert("stream".into(), Value::Bool(true));

    let current_items = input_value_to_items(obj.get("input"));
    let full_input = if let Some(prev_id) = obj
        .get("previous_response_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    {
        if let Some(prev_items) = history.get(&prev_id) {
            let mut combined = prev_items.clone();
            combined.extend(current_items);
            obj.insert("input".into(), Value::Array(combined.clone()));
            obj.remove("previous_response_id");
            combined
        } else {
            current_items
        }
    } else {
        current_items
    };

    Ok((payload, full_input))
}

fn ws_error_event(status: u16, error_type: &str, message: &str) -> Value {
    json!({
        "type": "error",
        "status": status,
        "error": {
            "type": error_type,
            "message": message
        }
    })
}

fn http_error_body_to_ws_event(status: u16, bytes: &[u8]) -> Value {
    if let Ok(parsed) = serde_json::from_slice::<Value>(bytes) {
        if let Some(err) = parsed.get("error") {
            return json!({
                "type": "error",
                "status": status,
                "error": err
            });
        }
        if let Some(msg) = parsed.get("message").and_then(Value::as_str) {
            return ws_error_event(status, "api_error", msg);
        }
    }
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    let msg = if text.is_empty() {
        format!("HTTP {status}")
    } else {
        text
    };
    ws_error_event(status, "api_error", &msg)
}

fn build_internal_post_request(
    uri: &Uri,
    handshake_headers: &HeaderMap,
    peer: SocketAddr,
    body_bytes: Vec<u8>,
) -> Result<Request<Body>, String> {
    let mut builder = Request::builder().method(Method::POST).uri(uri.clone());
    for (name, value) in handshake_headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "upgrade"
                | "connection"
                | "content-length"
                | "content-type"
                | "accept"
                | "accept-encoding"
        ) || lower.starts_with("sec-websocket-")
        {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }
    builder = builder
        .header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        )
        .header(
            HeaderName::from_static("accept"),
            HeaderValue::from_static("text/event-stream"),
        );

    let mut req = builder
        .body(Body::from(body_bytes))
        .map_err(|e| format!("构造内部请求失败: {e}"))?;
    req.extensions_mut().insert(ConnectInfo(peer));
    Ok(req)
}

fn extract_sse_data(block: &str) -> Option<String> {
    let mut data_lines = Vec::new();
    for raw_line in block.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let joined = data_lines.join("\n");
    let trimmed = joined.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return None;
    }
    Some(trimmed.to_string())
}

fn record_completed_turn(
    event_json: &Value,
    turn_input: &[Value],
    history: &mut TurnHistory,
) {
    let event_type = event_json.get("type").and_then(Value::as_str);
    if !matches!(event_type, Some("response.completed" | "response.incomplete")) {
        return;
    }
    let Some(response) = event_json.get("response") else {
        return;
    };
    let Some(resp_id) = response
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let mut full = turn_input.to_vec();
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        full.extend(output.iter().cloned());
    }
    history.remember(resp_id.to_string(), full);
}

async fn send_json(socket: &mut WebSocket, value: &Value) -> bool {
    match serde_json::to_string(value) {
        Ok(text) => socket.send(Message::Text(text)).await.is_ok(),
        Err(_) => false,
    }
}

async fn drive_socket(
    mut socket: WebSocket,
    bridge: WsBridgeState,
    peer: SocketAddr,
    handshake_headers: HeaderMap,
    uri: Uri,
) {
    let mut history = TurnHistory::default();

    while let Some(incoming) = socket.next().await {
        let msg = match incoming {
            Ok(msg) => msg,
            Err(_) => break,
        };
        match msg {
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => break,
            Message::Binary(_) => {
                let err = ws_error_event(
                    400,
                    "invalid_request_error",
                    "Binary WebSocket frames are not supported",
                );
                if !send_json(&mut socket, &err).await {
                    break;
                }
            }
            Message::Text(text) => {
                let payload: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        let err = ws_error_event(
                            400,
                            "invalid_request_error",
                            &format!("Invalid JSON payload: {e}"),
                        );
                        if !send_json(&mut socket, &err).await {
                            break;
                        }
                        continue;
                    }
                };

                let event_type = payload.get("type").and_then(Value::as_str);
                match event_type {
                    Some("response.cancel") => continue,
                    Some("response.create") | None if payload.get("model").is_some() => {}
                    Some(other) => {
                        let err = ws_error_event(
                            400,
                            "invalid_request_error",
                            &format!("Unsupported WebSocket event type: {other}"),
                        );
                        if !send_json(&mut socket, &err).await {
                            break;
                        }
                        continue;
                    }
                    None => {
                        let err = ws_error_event(
                            400,
                            "invalid_request_error",
                            "Missing event type or model in WebSocket frame",
                        );
                        if !send_json(&mut socket, &err).await {
                            break;
                        }
                        continue;
                    }
                }

                // Codex responses_websockets_v2 preconnect / warm-up (`generate: false`)
                if payload.get("generate").and_then(Value::as_bool) == Some(false) {
                    let warm_id = format!("resp_ws_{}", uuid::Uuid::new_v4().simple());
                    let model = payload
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let initial_items = input_value_to_items(payload.get("input"));
                    history.remember(warm_id.clone(), initial_items);
                    let created = json!({
                        "type": "response.created",
                        "sequence_number": 0,
                        "response": {
                            "id": warm_id,
                            "object": "response",
                            "status": "in_progress",
                            "model": model,
                            "output": []
                        }
                    });
                    let completed = json!({
                        "type": "response.completed",
                        "sequence_number": 1,
                        "response": {
                            "id": warm_id,
                            "object": "response",
                            "status": "completed",
                            "model": model,
                            "output": []
                        }
                    });
                    if !send_json(&mut socket, &created).await
                        || !send_json(&mut socket, &completed).await
                    {
                        break;
                    }
                    continue;
                }

                let (http_payload, turn_input) = match prepare_http_payload(payload, &history) {
                    Ok(pair) => pair,
                    Err(err_event) => {
                        if !send_json(&mut socket, &err_event).await {
                            break;
                        }
                        continue;
                    }
                };

                let body_bytes = match serde_json::to_vec(&http_payload) {
                    Ok(b) => b,
                    Err(e) => {
                        let err = ws_error_event(
                            500,
                            "server_error",
                            &format!("Failed to serialize request: {e}"),
                        );
                        if !send_json(&mut socket, &err).await {
                            break;
                        }
                        continue;
                    }
                };

                let req = match build_internal_post_request(
                    &uri,
                    &handshake_headers,
                    peer,
                    body_bytes,
                ) {
                    Ok(r) => r,
                    Err(msg) => {
                        let err = ws_error_event(500, "server_error", &msg);
                        if !send_json(&mut socket, &err).await {
                            break;
                        }
                        continue;
                    }
                };

                let mut svc = bridge.http_router.clone();
                let _ = std::future::poll_fn(|cx| {
                    <Router as Service<Request<Body>>>::poll_ready(&mut svc, cx)
                })
                .await;
                let response = match svc.call(req).await {
                    Ok(resp) => resp,
                    Err(infallible) => match infallible {},
                };

                let status = response.status();
                let is_sse = response
                    .headers()
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| {
                        v.split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("text/event-stream")
                    });

                if !status.is_success() {
                    let bytes = response
                        .into_body()
                        .collect()
                        .await
                        .map(|c| c.to_bytes())
                        .unwrap_or_default();
                    let err_event = http_error_body_to_ws_event(status.as_u16(), &bytes);
                    if !send_json(&mut socket, &err_event).await {
                        break;
                    }
                    continue;
                }

                if is_sse {
                    let mut body = response.into_body();
                    let mut buf = String::new();
                    let mut utf8_remainder: Vec<u8> = Vec::new();
                    let mut send_failed = false;

                    while let Some(frame_res) = body.frame().await {
                        let frame = match frame_res {
                            Ok(f) => f,
                            Err(e) => {
                                let err = ws_error_event(
                                    502,
                                    "upstream_error",
                                    &format!("Upstream stream interrupted: {e}"),
                                );
                                let _ = send_json(&mut socket, &err).await;
                                send_failed = true;
                                break;
                            }
                        };
                        if let Ok(data) = frame.into_data() {
                            crate::proxy::sse::append_utf8_safe(
                                &mut buf,
                                &mut utf8_remainder,
                                &data,
                            );
                            while let Some(block) = crate::proxy::sse::take_sse_block(&mut buf) {
                                if let Some(json_text) = extract_sse_data(&block) {
                                    if let Ok(parsed) = serde_json::from_str::<Value>(&json_text) {
                                        record_completed_turn(&parsed, &turn_input, &mut history);
                                    }
                                    if socket.send(Message::Text(json_text)).await.is_err() {
                                        send_failed = true;
                                        break;
                                    }
                                }
                            }
                            if send_failed {
                                break;
                            }
                        }
                    }

                    if !send_failed && !buf.trim().is_empty() {
                        if let Some(json_text) = extract_sse_data(&buf) {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&json_text) {
                                record_completed_turn(&parsed, &turn_input, &mut history);
                            }
                            if socket.send(Message::Text(json_text)).await.is_err() {
                                send_failed = true;
                            }
                        }
                    }

                    if send_failed {
                        break;
                    }
                } else {
                    // Non-streaming JSON fallback wrapped into Responses WS lifecycle events
                    let bytes = response
                        .into_body()
                        .collect()
                        .await
                        .map(|c| c.to_bytes())
                        .unwrap_or_default();
                    match serde_json::from_slice::<Value>(&bytes) {
                        Ok(resp_obj) => {
                            let created = json!({
                                "type": "response.created",
                                "sequence_number": 0,
                                "response": {
                                    "id": resp_obj.get("id").cloned().unwrap_or_else(|| json!("")),
                                    "object": "response",
                                    "status": "in_progress",
                                    "model": resp_obj.get("model").cloned().unwrap_or_else(|| json!("")),
                                    "output": []
                                }
                            });
                            let completed = json!({
                                "type": "response.completed",
                                "sequence_number": 1,
                                "response": resp_obj
                            });
                            record_completed_turn(&completed, &turn_input, &mut history);
                            if !send_json(&mut socket, &created).await
                                || !send_json(&mut socket, &completed).await
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            let err = ws_error_event(
                                502,
                                "upstream_error",
                                &format!("Invalid JSON response from upstream: {e}"),
                            );
                            if !send_json(&mut socket, &err).await {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_response_id_chains_prior_items_for_stateless_upstreams() {
        let mut history = TurnHistory::default();
        let turn1_input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        })];
        let completed = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "hi"}]
                }]
            }
        });
        record_completed_turn(&completed, &turn1_input, &mut history);

        let turn2_frame = json!({
            "type": "response.create",
            "model": "copilot/gemini-4-neon",
            "previous_response_id": "resp_1",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "next"}]
            }]
        });
        let (http_payload, full_input) = prepare_http_payload(turn2_frame, &history).unwrap();
        assert!(http_payload.get("type").is_none());
        assert!(http_payload.get("previous_response_id").is_none());
        assert_eq!(http_payload["stream"], true);
        assert_eq!(full_input.len(), 3);
        assert_eq!(full_input[0]["role"], "user");
        assert_eq!(full_input[1]["role"], "assistant");
        assert_eq!(full_input[2]["role"], "user");
    }

    #[test]
    fn extract_sse_data_skips_done_and_joins_multiline_data() {
        assert_eq!(
            extract_sse_data("event: response.completed\ndata: {\"type\":\"response.completed\"}\n"),
            Some("{\"type\":\"response.completed\"}".to_string())
        );
        assert_eq!(extract_sse_data("data: [DONE]\n"), None);
        assert_eq!(extract_sse_data(": keepalive\n"), None);
    }

    #[tokio::test]
    async fn live_websocket_bridge_upgrades_streams_chains_and_logs_clean_endpoint() {
        use axum::routing::post;
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let captured_bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_ref = captured_bodies.clone();

        let sse_handler = move |body: axum::body::Bytes| {
            let captured = captured_ref.clone();
            async move {
                let parsed: Value = serde_json::from_slice(&body).unwrap();
                let turn_no = {
                    let mut guard = captured.lock().unwrap();
                    guard.push(parsed.clone());
                    guard.len()
                };
                let resp_id = format!("resp_turn_{turn_no}");
                let sse = format!(
                    "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{resp_id}\",\"status\":\"in_progress\",\"output\":[]}}}}\n\n\
                     event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"reply-{turn_no}\"}}\n\n\
                     event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{resp_id}\",\"status\":\"completed\",\"output\":[{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"reply-{turn_no}\"}}]}}]}}}}\n\n\
                     data: [DONE]\n\n"
                );
                axum::http::Response::builder()
                    .status(200)
                    .header("content-type", "text/event-stream")
                    .body(Body::from(sse))
                    .unwrap()
            }
        };

        let logs = Arc::new(crate::manual::logs::RequestLogs::default());
        let drain = Arc::new(crate::manual::drain::Gate::default());

        let internal_http = Router::new()
            .route("/backend-api/codex/responses", post(sse_handler.clone()))
            .layer(axum::middleware::from_fn_with_state(
                logs.clone(),
                crate::manual::logs::capture,
            ))
            .layer(axum::middleware::from_fn_with_state(
                drain.clone(),
                crate::manual::drain::admission,
            ));

        let bridge = WsBridgeState::new(internal_http);
        let app = Router::new()
            .route(
                "/backend-api/codex/responses",
                post(sse_handler).get(handle_ws),
            )
            .layer(axum::Extension(bridge))
            .layer(axum::middleware::from_fn_with_state(
                logs.clone(),
                crate::manual::logs::capture,
            ))
            .layer(axum::middleware::from_fn_with_state(
                drain.clone(),
                crate::manual::drain::admission,
            ));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let handshake = format!(
            "GET /backend-api/codex/responses HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(handshake.as_bytes()).await.unwrap();

        let mut hdr_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !hdr_buf.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            hdr_buf.push(byte[0]);
        }
        let hdr_text = String::from_utf8_lossy(&hdr_buf);
        assert!(
            hdr_text.starts_with("HTTP/1.1 101"),
            "expected 101 upgrade, got: {hdr_text}"
        );

        async fn send_client_text(stream: &mut tokio::net::TcpStream, text: &str) {
            let payload = text.as_bytes();
            let mask = [0x12u8, 0x34, 0x56, 0x78];
            let mut frame = vec![0x81u8]; // FIN + text
            if payload.len() < 126 {
                frame.push(0x80 | (payload.len() as u8));
            } else {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            frame.extend_from_slice(&mask);
            for (i, b) in payload.iter().enumerate() {
                frame.push(b ^ mask[i % 4]);
            }
            stream.write_all(&frame).await.unwrap();
        }

        async fn read_server_text(stream: &mut tokio::net::TcpStream) -> Value {
            let mut head = [0u8; 2];
            stream.read_exact(&mut head).await.unwrap();
            let mut len = (head[1] & 0x7f) as usize;
            if len == 126 {
                let mut ext = [0u8; 2];
                stream.read_exact(&mut ext).await.unwrap();
                len = u16::from_be_bytes(ext) as usize;
            }
            let mut data = vec![0u8; len];
            stream.read_exact(&mut data).await.unwrap();
            serde_json::from_slice(&data).unwrap()
        }

        // Turn 1
        send_client_text(
            &mut stream,
            &json!({
                "type": "response.create",
                "model": "copilot/gpt-6.2-sol",
                "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}]
            })
            .to_string(),
        )
        .await;
        let e1 = read_server_text(&mut stream).await;
        let e2 = read_server_text(&mut stream).await;
        let e3 = read_server_text(&mut stream).await;
        assert_eq!(e1["type"], "response.created");
        assert_eq!(e2["delta"], "reply-1");
        assert_eq!(e3["type"], "response.completed");
        assert_eq!(e3["response"]["id"], "resp_turn_1");

        // Turn 2 chaining via previous_response_id
        send_client_text(
            &mut stream,
            &json!({
                "type": "response.create",
                "model": "copilot/gpt-6.2-sol",
                "previous_response_id": "resp_turn_1",
                "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "follow-up"}]}]
            })
            .to_string(),
        )
        .await;
        let _ = read_server_text(&mut stream).await;
        let e5 = read_server_text(&mut stream).await;
        let e6 = read_server_text(&mut stream).await;
        assert_eq!(e5["delta"], "reply-2");
        assert_eq!(e6["response"]["id"], "resp_turn_2");

        // Verify Turn 2 received chained 3-item history without previous_response_id
        let bodies = captured_bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[1].get("previous_response_id").is_none());
        assert_eq!(bodies[1]["input"].as_array().unwrap().len(), 3);

        // Verify logs recorded 2 clean `/v1/responses` turns and 0 bogus 101/`未知接口` entries!
        let records = logs.snapshot();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.endpoint == "/v1/responses"));
        assert!(records.iter().all(|r| r.status == 200));

        drop(stream);
        server.abort();
    }
}
