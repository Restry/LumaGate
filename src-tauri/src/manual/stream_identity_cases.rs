use super::*;
use std::collections::BTreeMap;

fn chat_chunk(id: Option<&str>, delta: Value, finish: Option<&str>) -> Value {
    let mut chunk = json!({"object":"chat.completion.chunk","model":"model-a","choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
    if let Some(id) = id {
        chunk["id"] = json!(id);
    }
    chunk
}

fn chat_wire(chunks: Vec<Value>) -> String {
    let mut wire = String::new();
    for chunk in chunks {
        wire.push_str(&format!("data: {chunk}\n\n"));
    }
    wire.push_str("data: [DONE]\n\n");
    wire
}

async fn replay(wire: String) -> (ProxyServer, String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let wire = wire.clone();
            async move {
                let chunks: Vec<_> = wire
                    .as_bytes()
                    .chunks(17)
                    .map(|chunk| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(chunk)))
                    .collect();
                (
                    [("content-type", "text/event-stream")],
                    axum::body::Body::from_stream(futures::stream::iter(chunks)),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (server, base, _) =
        gateway(vec![source("identity", &format!("http://{address}/v1"))]).await;
    (server, base, task)
}

async fn response_events(base: &str) -> Vec<Value> {
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":"model-a","input":"fixture","stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let wire = response.text().await.unwrap();
    wire.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}

// A downstream-like assembler keys each message by response and item identities.
// Completion replaces the snapshot at the same key; it never deduplicates text.
fn assembled_messages(events: &[Value]) -> BTreeMap<(String, String), String> {
    let mut current = String::new();
    let mut messages = BTreeMap::new();
    for event in events {
        match event["type"].as_str().unwrap() {
            "response.created" => current = event["response"]["id"].as_str().unwrap().into(),
            "response.output_text.delta" => {
                let key = (current.clone(), event["item_id"].as_str().unwrap().into());
                messages
                    .entry(key)
                    .or_insert_with(String::new)
                    .push_str(event["delta"].as_str().unwrap());
            }
            "response.completed" => {
                for item in event["response"]["output"].as_array().unwrap() {
                    if item["type"] == "message" {
                        let key = (
                            event["response"]["id"].as_str().unwrap().into(),
                            item["id"].as_str().unwrap().into(),
                        );
                        messages.insert(key, item["content"][0]["text"].as_str().unwrap().into());
                    }
                }
            }
            _ => {}
        }
    }
    messages
}

#[tokio::test]
async fn chat_bridge_late_id_reconstructs_one_response() {
    let (server, base, task) = replay(chat_wire(vec![
        chat_chunk(None, json!({"role":"assistant"}), None),
        chat_chunk(
            Some("chat-authoritative"),
            json!({"content":"fixture answer"}),
            Some("stop"),
        ),
    ]))
    .await;
    let events = response_events(&base).await;
    server.stop().await.unwrap();
    task.abort();
    for event in &events {
        if let Some(id) = event["response"]["id"].as_str() {
            println!("{} response.id={id}", event["type"].as_str().unwrap());
        }
    }
    assert_eq!(
        assembled_messages(&events).len(),
        1,
        "one reply must not become two response identities"
    );
    let id = &events[0]["response"]["id"];
    assert!(events
        .iter()
        .filter(|e| e.get("response").is_some())
        .all(|e| &e["response"]["id"] == id));
    assert_eq!(
        assembled_messages(&events).values().next().unwrap(),
        "fixture answer"
    );
}

#[tokio::test]
async fn native_responses_preserves_item_scopes_and_opaque_continuation() {
    let items = json!([
        {"id":"rs_native","type":"reasoning","encrypted_content":"opaque-fixture-state","summary":[]},
        {"id":"msg_first","type":"message","role":"assistant","content":[{"type":"output_text","text":"first"}]},
        {"id":"msg_second","type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]},
        {"id":"fc_native","type":"function_call","call_id":"call_native","name":"lookup","arguments":"{}"}
    ]);
    let mut frames = vec![
        json!({"type":"response.created","response":{"id":"resp_native","status":"in_progress","output":[]}}),
        json!({"type":"response.in_progress","response":{"id":"resp_native","status":"in_progress","output":[]}}),
    ];
    for (index, item) in items.as_array().unwrap().iter().enumerate() {
        frames.push(json!({"type":"response.output_item.added","output_index":index,"item":item}));
        if item["type"] == "message" {
            frames.push(json!({"type":"response.output_text.delta","output_index":index,"content_index":0,"item_id":item["id"],"delta":item["content"][0]["text"]}));
        }
        frames.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    frames.push(json!({"type":"response.completed","response":{"id":"resp_native","status":"completed","output":items}}));
    let wire: String = frames
        .iter()
        .map(|frame| {
            format!(
                "event: {}\ndata: {frame}\n\n",
                frame["type"].as_str().unwrap()
            )
        })
        .collect();
    let expected = wire.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |Json(body): Json<Value>| {
            let wire = wire.clone();
            async move {
                if body.get("previous_response_id").is_none() {
                    return Json(json!({"id":"resp_previous_server","object":"response","status":"completed","output":[{"id":"msg_previous","type":"message","role":"assistant","content":[{"type":"output_text","text":"fixture"}]}]})).into_response();
                }
                assert_eq!(body["previous_response_id"], "resp_previous_server");
                assert_eq!(
                    body["input"][0]["encrypted_content"],
                    "opaque-fixture-input"
                );
                ([("content-type", "text/event-stream")], wire).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut provider = source("native", &format!("http://{address}/v1"));
    provider.models[0].protocol_override = Some(Protocol::OpenaiResponses);
    let (server, base, _) = gateway(vec![provider]).await;
    let client = reqwest::Client::new();
    let initial = client
        .post(format!("{base}/v1/responses"))
        .header("x-session-id", "native-continuation")
        .json(&json!({"model":"model-a","stream":false,"input":"fixture"}))
        .send()
        .await
        .unwrap();
    assert_eq!(initial.status(), StatusCode::OK);
    let previous: Value = initial.json().await.unwrap();
    assert_eq!(previous["id"], "resp_previous_server");
    let response = client.post(format!("{base}/v1/responses"))
        .header("x-session-id", "native-continuation")
        .json(&json!({"model":"model-a","stream":true,"previous_response_id":previous["id"],"input":[{"type":"reasoning","encrypted_content":"opaque-fixture-input","summary":[]}]}))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let actual = response.text().await.unwrap();
    server.stop().await.unwrap();
    task.abort();
    assert_eq!(
        actual, expected,
        "native SSE must not replace server IDs or opaque state"
    );
    let messages = assembled_messages(&frames);
    assert_eq!(
        messages.len(),
        2,
        "distinct native messages must not collapse"
    );
    assert_eq!(
        messages[&("resp_native".into(), "msg_first".into())],
        "first"
    );
    assert_eq!(
        messages[&("resp_native".into(), "msg_second".into())],
        "second"
    );
}

fn assert_item_identity(events: &[Value]) {
    let response_id = &events[0]["response"]["id"];
    let mut items = BTreeMap::new();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "response.created")
            .count(),
        1
    );
    for event in events {
        if let Some(response) = event.get("response") {
            assert_eq!(&response["id"], response_id);
            for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
                assert_eq!(&items[&(index as u64)], &item["id"]);
            }
        }
        if event["type"] == "response.output_item.added" {
            let id = event["item"]["id"].clone();
            assert_ne!(&id, response_id);
            assert!(!items.values().any(|existing| existing == &id));
            assert!(items
                .insert(event["output_index"].as_u64().unwrap(), id)
                .is_none());
        } else if event["type"] == "response.output_item.done" {
            assert_eq!(
                items[&event["output_index"].as_u64().unwrap()],
                event["item"]["id"]
            );
        }
        if let Some(item_id) = event.get("item_id") {
            assert_eq!(&items[&event["output_index"].as_u64().unwrap()], item_id);
        }
        if let Some(content_index) = event.get("content_index") {
            assert_eq!(content_index, 0);
        }
    }
}

#[tokio::test]
async fn chat_bridge_initial_id_survives_changed_chunks_and_terminal_usage() {
    let (server, base, task) = replay(chat_wire(vec![
        chat_chunk(Some("chat-first"), json!({"content":"Hello "}), None),
        chat_chunk(Some("chat-later"), json!({"content":"world"}), Some("stop")),
        json!({"id":"","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}),
    ])).await;
    let events = response_events(&base).await;
    server.stop().await.unwrap();
    task.abort();
    assert_item_identity(&events);
    let messages = assembled_messages(&events);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages.values().next().unwrap(), "Hello world");
    let completed = events.last().unwrap();
    assert_eq!(completed["response"]["id"], "resp_chat-first");
    assert_eq!(completed["response"]["usage"]["total_tokens"], 5);
}

#[tokio::test]
async fn concurrent_idless_chat_streams_have_distinct_response_and_message_ids() {
    let (server, base, task) = replay(chat_wire(vec![
        chat_chunk(Some(""), json!({"role":"assistant"}), None),
        chat_chunk(None, json!({"content":"same fixture text"}), Some("stop")),
    ]))
    .await;
    let (first, second) = tokio::join!(response_events(&base), response_events(&base));
    server.stop().await.unwrap();
    task.abort();
    assert_item_identity(&first);
    assert_item_identity(&second);
    assert_ne!(first[0]["response"]["id"], second[0]["response"]["id"]);
    let mut messages = assembled_messages(&first);
    messages.extend(assembled_messages(&second));
    assert_eq!(
        messages.len(),
        2,
        "identical text from distinct requests is not duplicate output"
    );
    let ids: std::collections::BTreeSet<_> = messages.keys().map(|(_, item)| item).collect();
    assert_eq!(ids.len(), 2);
}

#[tokio::test]
async fn chat_bridge_keeps_reasoning_text_and_parallel_tool_items_linked() {
    let (server, base, task) = replay(chat_wire(vec![
        chat_chunk(Some("chat-tools"), json!({"reasoning_content":"fixture reasoning"}), None),
        chat_chunk(Some("chat-tools-late"), json!({"content":"fixture answer"}), None),
        chat_chunk(Some("chat-tools-late"), json!({"tool_calls":[
            {"index":0,"id":"call_first","type":"function","function":{"name":"first","arguments":"{\"value\":"}},
            {"index":1,"id":"call_second","type":"function","function":{"name":"second","arguments":"{\"value\":"}}
        ]}), None),
        chat_chunk(Some("chat-tools-late"), json!({"tool_calls":[
            {"index":1,"function":{"arguments":"2}"}},
            {"index":0,"function":{"arguments":"1}"}}
        ]}), Some("tool_calls")),
    ])).await;
    let events = response_events(&base).await;
    server.stop().await.unwrap();
    task.abort();
    assert_item_identity(&events);
    let items = events.last().unwrap()["response"]["output"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["type"], "reasoning");
    assert_eq!(items[1]["type"], "message");
    for (item, call, args) in [
        (&items[2], "call_first", "{\"value\":1}"),
        (&items[3], "call_second", "{\"value\":2}"),
    ] {
        assert_eq!(item["call_id"], call);
        assert_ne!(item["id"], item["call_id"]);
        assert_eq!(item["arguments"], args);
        let added = events
            .iter()
            .find(|e| e["type"] == "response.output_item.added" && e["item"]["id"] == item["id"])
            .unwrap();
        assert_eq!(added["item"]["call_id"], item["call_id"]);
        let arguments: String = events
            .iter()
            .filter(|e| {
                e["type"] == "response.function_call_arguments.delta" && e["item_id"] == item["id"]
            })
            .map(|e| e["delta"].as_str().unwrap())
            .collect();
        assert_eq!(arguments, args);
    }
    assert_eq!(assembled_messages(&events).len(), 1);
}

#[tokio::test]
async fn interrupted_chat_stream_keeps_identity_without_success_or_restart() {
    for (suffix, expected) in [
        ("", "incomplete"),
        ("event: error\ndata: {\"error\":{\"message\":\"fixture failure\",\"type\":\"stream_error\"}}\n\n", "failed"),
    ] {
        let chunk = chat_chunk(None, json!({"content":"partial"}), None);
        let (server, base, task) = replay(format!("data: {chunk}\n\n{suffix}")).await;
        let events = response_events(&base).await;
        server.stop().await.unwrap();
        task.abort();
        assert_item_identity(&events);
        let terminal: Vec<_> = events.iter().filter(|e| matches!(e["type"].as_str(), Some("response.completed" | "response.failed"))).collect();
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0]["response"]["status"], expected);
        assert_eq!(assembled_messages(&events).len(), 1);
    }
}

#[tokio::test]
async fn native_chat_usage_and_anthropic_message_start_keep_protocol_identity() {
    let wire = chat_wire(vec![
        chat_chunk(
            Some("chat-valid"),
            json!({"content":"fixture"}),
            Some("stop"),
        ),
        json!({"id":"chat-valid","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
    ]);
    let (server, base, task) = replay(wire.clone()).await;
    let client = reqwest::Client::new();
    let native = client.post(format!("{base}/v1/chat/completions"))
        .json(&json!({"model":"model-a","stream":true,"messages":[{"role":"user","content":"fixture"}]}))
        .send().await.unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    assert_eq!(native.text().await.unwrap(), wire);
    let anthropic = client.post(format!("{base}/v1/messages"))
        .json(&json!({"model":"model-a","stream":true,"max_tokens":32,"messages":[{"role":"user","content":"fixture"}]}))
        .send().await.unwrap();
    assert_eq!(anthropic.status(), StatusCode::OK);
    let output = anthropic.text().await.unwrap();
    server.stop().await.unwrap();
    task.abort();
    let events: Vec<Value> = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "message_start")
            .count(),
        1
    );
    assert_eq!(events[0]["message"]["id"], "chat-valid");
    assert_eq!(events.last().unwrap()["type"], "message_stop");
    assert!(events
        .iter()
        .skip(1)
        .all(|e| e.get("id").is_none() && e.get("message").is_none()));
}

#[tokio::test]
async fn chat_bridge_streams_before_eof_and_cancels_upstream_without_retry() {
    use std::sync::atomic::AtomicUsize;
    struct Dropped(Arc<tokio::sync::Notify>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let dropped = Arc::new(tokio::sync::Notify::new());
    let notify = dropped.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let guard = Dropped(notify.clone());
            async move {
                let body = async_stream::stream! {
                    let _guard = guard;
                    let chunk = chat_chunk(None, json!({"content":"partial"}), None);
                    yield Ok::<_, std::io::Error>(bytes::Bytes::from(format!("data: {chunk}\n\n")));
                    std::future::pending::<()>().await;
                };
                (
                    [("content-type", "text/event-stream")],
                    axum::body::Body::from_stream(body),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (server, base, _) = gateway(vec![source("cancel", &format!("http://{address}/v1"))]).await;
    let mut response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":"model-a","input":"fixture","stream":true}))
        .send()
        .await
        .unwrap();
    let mut received = String::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !received.contains("response.output_text.delta") {
            received
                .push_str(std::str::from_utf8(&response.chunk().await.unwrap().unwrap()).unwrap());
        }
    })
    .await
    .expect("partial output must arrive while upstream is still open");
    assert!(!received.contains("response.completed"));
    drop(response);
    tokio::time::timeout(Duration::from_secs(3), dropped.notified())
        .await
        .expect("cancel must close upstream body");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.stop().await.unwrap();
    task.abort();
}
