use super::*;
use axum::body::Body;
use std::sync::atomic::AtomicUsize;

async fn forward_fixture(
    body: String,
    content_type: &'static str,
    streaming: bool,
) -> (u16, String, Vec<logs::RequestLog>, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let body = body.clone();
            let calls = count.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let chunks: Vec<_> = body
                    .as_bytes()
                    .chunks(19)
                    .map(|v| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(v)))
                    .collect();
                (
                    [("content-type", content_type)],
                    Body::from_stream(futures::stream::iter(chunks)),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut provider = source("outcome-fixture", &format!("http://{address}/v1"));
    provider.protocol = Protocol::OpenaiResponses;
    provider.models[0].protocol_override = Some(Protocol::OpenaiResponses);
    let (server, base, doc) = gateway(vec![provider]).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":doc.groups()[0].id,"input":"fixture","stream":streaming}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    server.stop().await.unwrap();
    let rows = server.manual_request_logs();
    task.abort();
    (status, text, rows, calls.load(Ordering::SeqCst))
}
fn failed() -> Value {
    json!({"id":"resp-fixture","model":"model-a","status":"failed","output":[],"usage":null,
        "error":{"code":"rate_limit_exceeded","message":"fixture rate limit"}})
}
#[tokio::test]
async fn native_responses_json_200_rate_limit_becomes_429() {
    let (status, body, rows, calls) =
        forward_fixture(failed().to_string(), "application/json", false).await;
    assert_eq!(status, 429);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"]["code"],
        "rate_limit_exceeded"
    );
    assert_eq!(rows[0].status, 429);
    assert!(rows[0].response_state.contains("限流"));
    assert_eq!(calls, 1);
}
#[tokio::test]
async fn native_responses_early_stream_failure_becomes_429_before_headers() {
    let stream = format!("data: {{\"type\":\"response.created\",\"response\":{{\"status\":\"in_progress\"}}}}\n\nevent: response.failed\ndata: {}\n\n", json!({"type":"response.failed","response":failed()}));
    let (status, _, rows, calls) = forward_fixture(stream, "text/event-stream", true).await;
    assert_eq!(status, 429);
    assert!(rows[0].response_state.contains("限流"));
    assert_eq!(calls, 1);
}
#[tokio::test]
async fn late_stream_failure_keeps_http_and_bytes_but_records_failed_outcome() {
    let stream = format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"你好 🌍\"}}\n\ndata: {}\n\n",
        json!({"type":"response.failed","response":failed()})
    );
    let (status, body, rows, calls) =
        forward_fixture(stream.clone(), "text/event-stream", true).await;
    assert_eq!(status, 200);
    assert_eq!(body, stream);
    assert_eq!(rows[0].response["status"], "failed");
    assert_eq!(rows[0].response_state, "调用失败 · 限流（429）");
    assert_eq!(calls, 1, "never retry a stream after productive output");
}
#[tokio::test]
async fn stream_eof_without_protocol_end_is_unknown_but_explicit_end_is_completed() {
    use crate::manual::completion::Completion;
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    for (suffix, expected) in [
        ("", Completion::Unknown),
        ("data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"error\":null}}\n\n", Completion::Completed),
    ] {
        let body = format!("{partial}{suffix}");
        let (status, actual, rows, _) = forward_fixture(body.clone(), "text/event-stream", true).await;
        assert_eq!(status, 200);
        assert_eq!(actual, body);
        assert_eq!(rows[0].completion, Some(expected));
    }
}

#[tokio::test]
async fn responses_null_error_and_quoted_error_text_are_not_failures() {
    let body = json!({"id":"resp-ok","status":"completed","error":null,"output":[{"type":"message","content":[{"type":"output_text","text":"rate_limit_exceeded"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}).to_string();
    let (status, actual, rows, _) = forward_fixture(body.clone(), "application/json", false).await;
    assert_eq!(status, 200);
    assert_eq!(actual, body);
    assert_eq!(rows[0].response_state, "已结束");
}
