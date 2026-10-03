use super::*;
use crate::manual::{
    catalog::{Model, Protocol, Source},
    tests::gateway,
};
use axum::{response::IntoResponse, routing::post, Router};
use serde_json::{json, Value};

fn parsed(wire: Wire, raw: Value) -> Usage {
    let mut accumulator = Accumulator::default();
    accumulator.update(serde_json::from_value(raw).unwrap(), wire, false);
    accumulator.result(wire, true)
}
fn sse(wire: Wire, text: &str, clean: bool) -> Usage {
    let mut collector = SseCollector::new(wire);
    for chunk in text.as_bytes().chunks(7) {
        collector.feed(chunk);
    }
    collector.finish(clean)
}
#[test]
fn openai_cache_and_reasoning_are_subsets_not_extra_tokens() {
    let usage = parsed(
        Wire::Chat,
        json!({"prompt_tokens":100,"completion_tokens":25,"total_tokens":125,
        "prompt_tokens_details":{"cached_tokens":40},"completion_tokens_details":{"reasoning_tokens":10}}),
    );
    assert_eq!(usage.state, UsageState::Reported);
    assert_eq!(usage.input_tokens, Some(100));
    assert_eq!(usage.output_tokens, Some(25));
    assert_eq!(usage.total_tokens, Some(125));
    assert_eq!(usage.cache_read_tokens, Some(40));
    assert_eq!(usage.reasoning_tokens, Some(10));
    let responses = parsed(
        Wire::Responses,
        json!({"input_tokens":80,"output_tokens":20,"input_tokens_details":{"cached_tokens":30},"output_tokens_details":{"reasoning_tokens":5}}),
    );
    assert_eq!(responses.total_tokens, Some(100));
}
#[test]
fn anthropic_input_includes_cache_read_and_write_exactly_once() {
    let usage = parsed(
        Wire::Anthropic,
        json!({"input_tokens":100,"output_tokens":25,"cache_read_input_tokens":400,
        "cache_creation_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":20}}),
    );
    assert_eq!(usage.input_tokens, Some(550));
    assert_eq!(usage.total_tokens, Some(575));
    let nested = parsed(
        Wire::Anthropic,
        json!({"input_tokens":100,"output_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":20}}),
    );
    assert_eq!(nested.total_tokens, Some(150));
}
#[test]
fn real_zero_is_known_but_absent_partial_and_inconsistent_usage_are_not_zero() {
    let zero = parsed(
        Wire::Chat,
        json!({"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}),
    );
    assert_eq!(zero.state, UsageState::Reported);
    assert_eq!(zero.total_tokens, Some(0));
    let empty = parsed(Wire::Chat, json!({}));
    assert_eq!(empty.state, UsageState::Unavailable);
    assert!(empty.total_tokens.is_none());
    let partial = parsed(Wire::Responses, json!({"input_tokens":10}));
    assert_eq!(partial.state, UsageState::Partial);
    assert!(partial.output_tokens.is_none());
    assert!(partial.total_tokens.is_none());
    let inconsistent = parsed(
        Wire::Chat,
        json!({"prompt_tokens":5,"completion_tokens":5,"total_tokens":99}),
    );
    assert_eq!(inconsistent.state, UsageState::Unavailable);
    let huge = parsed(
        Wire::Chat,
        json!({"prompt_tokens":u64::MAX,"completion_tokens":1}),
    );
    assert_eq!(huge.state, UsageState::Unavailable);
    assert!(serde_json::from_value::<RawUsage>(json!({"input_tokens":-1})).is_err());
    assert!(serde_json::from_value::<RawUsage>(json!({"input_tokens":1.5})).is_err());
}
#[test]
fn streaming_usage_snapshots_replace_not_accumulate_and_support_multiline_crlf() {
    let chat = "data: {\"choices\":[{}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n\
        data: {\"choices\":[{}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":7}}\n\ndata: [DONE]\n\n";
    assert_eq!(sse(Wire::Chat, chat, true).total_tokens, Some(17));
    let anthropic = "event: message_start\r\ndata: {\"message\":\r\ndata: {\"usage\":{\"input_tokens\":100,\"output_tokens\":0,\"cache_read_input_tokens\":50}}}\r\n\r\n\
        event: message_delta\ndata: {\"usage\":{\"output_tokens\":3}}\n\n\
        event: message_delta\ndata: {\"usage\":{\"output_tokens\":8}}\n\n\
        event: message_stop\ndata: {}\n\n";
    let usage = sse(Wire::Anthropic, anthropic, false);
    assert_eq!(usage.state, UsageState::Reported);
    assert_eq!(usage.input_tokens, Some(150));
    assert_eq!(usage.output_tokens, Some(8));
    assert_eq!(usage.total_tokens, Some(158));
}
#[test]
fn interrupted_usage_is_partial_unless_an_authoritative_final_event_was_seen() {
    let start = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":15,\"output_tokens\":0}}}\n\n";
    let usage = sse(Wire::Anthropic, start, false);
    assert_eq!(usage.state, UsageState::Partial);
    assert!(usage.output_tokens.is_none());
    let final_event = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":15,\"output_tokens\":5}}}\n\n";
    let usage = sse(Wire::Responses, final_event, false);
    assert_eq!(usage.state, UsageState::Reported);
    assert_eq!(usage.total_tokens, Some(20));
    let partial =
        "data: {\"choices\":[{}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3}}\n\n";
    assert_eq!(sse(Wire::Chat, partial, false).state, UsageState::Partial);
    assert_eq!(sse(Wire::Chat, partial, true).state, UsageState::Partial);
    let stopped = format!("{start}data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n");
    assert_eq!(
        sse(Wire::Anthropic, &stopped, false).state,
        UsageState::Reported
    );
}
#[test]
fn long_responses_do_not_share_the_text_preview_limit_and_oversize_events_are_bounded() {
    let body = json!({"type":"response.completed","response":{"output":[{"text":"private-text ".repeat(20000)}],"usage":{"input_tokens":10000,"output_tokens":3000}}});
    let stream = format!("data: {body}\n\n");
    let usage = sse(Wire::Responses, &stream, true);
    assert_eq!(usage.total_tokens, Some(13000));
    assert!(!serde_json::to_string(&usage)
        .unwrap()
        .contains("private-text"));
    let mut collector = SseCollector::new(Wire::Responses);
    collector.feed(b"data: ");
    collector.feed(&vec![b'x'; MAX_EVENT_BYTES + 100]);
    collector.feed(b"\n\n");
    assert!(collector.line.len() <= MAX_EVENT_BYTES);
    assert!(collector.data.len() <= MAX_EVENT_BYTES);
    let limited = collector.finish(true);
    assert_eq!(limited.state, UsageState::Unavailable);
    assert_eq!(limited.reason, Some("event_too_large"));
    let mut stale = SseCollector::new(Wire::Chat);
    stale.feed(
        b"data: {\"choices\":[{}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
    );
    stale.feed(b"data: ");
    stale.feed(&vec![b'x'; MAX_EVENT_BYTES + 100]);
    stale.feed(b"\n\ndata: [DONE]\n\n");
    let result = stale.finish(true);
    assert_eq!(result.state, UsageState::Unavailable);
    assert!(result.total_tokens.is_none());
}

#[test]
fn completion_is_separate_from_usage_and_does_not_promote_a_failed_stream() {
    let cases = [
        ("data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n", Completion::Unknown),
        ("data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n", Completion::Completed),
        ("data: {\"choices\":[{\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n", Completion::Incomplete),
        ("event: error\ndata: {\"message\":\"fixture failure\"}\n\ndata: [DONE]\n\n", Completion::Failed),
        ("data: {\"type\":\"message_stop\"}\n\n", Completion::Completed),
    ];
    for (bytes, expected) in cases {
        let mut collector = SseCollector::new(Wire::Chat);
        collector.feed(bytes.as_bytes());
        collector.finish(true);
        assert_eq!(collector.completion(), expected);
    }
}

async fn integration(
    protocol: Protocol,
    stream: bool,
    body: String,
    content_type: &'static str,
) -> (Value, Vec<crate::manual::logs::RequestLog>) {
    let upstream = Router::new().route(
        &format!("/v1/{}", protocol.endpoint()),
        post(move || {
            let body = body.clone();
            async move { ([("content-type", content_type)], body).into_response() }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let model = Model {
        id: "fixture-model".into(),
        protocol_override: Some(protocol.clone()),
        enabled: true,
        ..Default::default()
    };
    let provider = Source {
        id: "fixture".into(),
        name: "Fixture".into(),
        base_url: format!("http://{address}/v1"),
        key_env: String::new(),
        key_ref: None,
        copilot: None,
        protocol,
        enabled: true,
        models: vec![model],
    };
    let (server, base, doc) = gateway(vec![provider]).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":doc.groups()[0].id,"input":"hello","stream":stream}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let text = response.text().await.unwrap();
    let value = serde_json::from_str(&text).unwrap_or(json!({"text":text}));
    let logs = server.manual_request_logs();
    server.stop().await.unwrap();
    task.abort();
    (value, logs)
}
#[tokio::test]
async fn bridge_reads_actual_upstream_usage_not_synthetic_downstream_zeroes() {
    let mut body = json!({"id":"chat-1","object":"chat.completion","model":"fixture-model","choices":[{"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]});
    let (_, missing) = integration(
        Protocol::OpenaiChat,
        false,
        body.to_string(),
        "application/json",
    )
    .await;
    assert_eq!(
        missing[0].usage.as_ref().unwrap().state,
        UsageState::Unavailable
    );
    body["usage"] = json!({"prompt_tokens":150,"completion_tokens":10,"total_tokens":160,"prompt_tokens_details":{"cached_tokens":50}});
    let (_, rows) = integration(
        Protocol::OpenaiChat,
        false,
        body.to_string(),
        "application/json",
    )
    .await;
    let usage = rows[0].usage.as_ref().unwrap();
    assert_eq!(usage.state, UsageState::Reported);
    assert_eq!(usage.input_tokens, Some(150));
    assert_eq!(usage.output_tokens, Some(10));
    assert_eq!(usage.total_tokens, Some(160));
}
#[tokio::test]
async fn bridged_stream_counts_final_usage_after_large_output_without_changing_response_bytes() {
    let chunk = json!({"id":"chat-1","object":"chat.completion.chunk","model":"fixture-model","choices":[{"index":0,"delta":{"content":"hello ".repeat(15000)},"finish_reason":null}]});
    let usage = json!({"id":"chat-1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":20000,"total_tokens":21000}});
    let body = format!("data: {chunk}\n\ndata: {usage}\n\ndata: [DONE]\n\n");
    let (response, rows) = integration(Protocol::OpenaiChat, true, body, "text/event-stream").await;
    assert!(response["text"].as_str().unwrap().contains("hello"));
    let usage = rows[0].usage.as_ref().unwrap();
    assert_eq!(usage.state, UsageState::Reported);
    assert_eq!(usage.total_tokens, Some(21000));
}
#[tokio::test]
async fn anthropic_bridge_keeps_cache_usage_and_native_responses_usage_is_available() {
    let body = json!({"id":"msg_fixture","type":"message","role":"assistant","model":"fixture-model","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":20}});
    let (_, rows) = integration(
        Protocol::Anthropic,
        false,
        body.to_string(),
        "application/json",
    )
    .await;
    assert_eq!(rows[0].usage.as_ref().unwrap().total_tokens, Some(35));
    let body = json!({"id":"resp_fixture","object":"response","status":"completed","model":"fixture-model","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":40,"output_tokens":10,"total_tokens":50}});
    let (_, rows) = integration(
        Protocol::OpenaiResponses,
        false,
        body.to_string(),
        "application/json",
    )
    .await;
    assert_eq!(rows[0].usage.as_ref().unwrap().total_tokens, Some(50));
    let delta = json!({"type":"response.output_text.delta","delta":"你好 🌍 — not usage: 999999"});
    let completed = json!({"type":"response.completed","response":body});
    let stream = format!("data: {delta}\n\ndata: {completed}\n\ndata: [DONE]\n\n");
    let (_, rows) = integration(Protocol::OpenaiResponses, true, stream, "text/event-stream").await;
    assert_eq!(rows[0].usage.as_ref().unwrap().state, UsageState::Reported);
    assert_eq!(rows[0].usage.as_ref().unwrap().total_tokens, Some(50));
}
