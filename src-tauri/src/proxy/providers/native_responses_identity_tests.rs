use super::*;
use serde_json::json;

#[test]
fn only_opted_in_responses_are_rewritten_and_compaction_restores_input_handles() {
    let mut provider = Provider::with_id("fixture".into(), "fixture".into(), json!({}), None);
    assert!(!enabled(&provider, "/v1/responses"));
    provider.settings_config[ENABLED] = json!(true);
    assert!(enabled(&provider, "/responses?beta=1"));
    assert!(!enabled(&provider, "/responses/compact"));
    assert!(request_enabled(&provider, "/responses/compact"));
    assert!(!request_enabled(&provider, "/chat/completions"));
    assert!(!request_enabled(&provider, "/alpha/search"));
}

#[test]
fn failure_status_and_usage_survive_id_repair() {
    for terminal in ["response.failed", "response.incomplete"] {
        let mut state = StreamIdentity::new(store(), "session-a".into());
        state
            .rewrite(&mut json!({"type":"response.created","response":{"id":"start","output":[]}}))
            .unwrap();
        let mut event = json!({"type":terminal,"response":{"id":"final","status":"failed","error":{"code":"fixture"},"usage":{"total_tokens":2},"output":[]}});
        let mut expected = event.clone();
        expected["response"]["id"] = json!("start");
        state.rewrite(&mut event).unwrap();
        assert_eq!(event, expected);
    }
}

fn store() -> Arc<IdentityStore> {
    Arc::new(IdentityStore::new(Arc::new(Database::memory().unwrap())))
}

fn fixture() -> Vec<Value> {
    vec![
        json!({"type":"response.created","response":{"id":"resp-start","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"msg-start","type":"message","role":"assistant","phase":"final_answer","content":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"msg-part","content_index":0,"part":{"type":"output_text","text":""}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg-delta-1","delta":"你好 "}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg-delta-2","delta":"world"}),
        json!({"type":"response.output_text.done","output_index":0,"item_id":"msg-text-done","text":"你好 world"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"msg-done","type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"你好 world"}]}}),
        json!({"type":"response.completed","response":{"id":"resp-final","status":"completed","output":[{"id":"msg-final","type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"你好 world"}]}],"usage":{"total_tokens":7}}}),
    ]
}

fn wire(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .map(|e| {
            format!(
                "event: {}\r\ndata: {e}\r\n\r\n",
                e["type"].as_str().unwrap()
            )
        })
        .collect::<String>()
        .into_bytes()
}

fn events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

async fn replay(raw: Vec<u8>, store: Arc<IdentityStore>) -> Vec<u8> {
    // Every byte boundary, including inside UTF-8 and CRLF, may be a transport boundary.
    let chunks = raw
        .into_iter()
        .map(|b| Ok(Bytes::from(vec![b])))
        .collect::<Vec<_>>();
    stream(futures::stream::iter(chunks), store, "session-a".into())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flat_map(|r| r.unwrap().to_vec())
        .collect()
}

fn assembled(events: &[Value]) -> HashMap<(String, String), String> {
    let mut response = String::new();
    let mut messages = HashMap::new();
    for e in events {
        if e["type"] == "response.created" {
            response = e["response"]["id"].as_str().unwrap().into();
        }
        if e["type"] == "response.output_text.delta" {
            messages
                .entry((response.clone(), e["item_id"].as_str().unwrap().to_owned()))
                .or_insert_with(String::new)
                .push_str(e["delta"].as_str().unwrap());
        }
        if e["type"] == "response.completed" {
            for item in e["response"]["output"].as_array().unwrap() {
                if item["type"] == "message" {
                    messages.insert(
                        (
                            e["response"]["id"].as_str().unwrap().into(),
                            item["id"].as_str().unwrap().into(),
                        ),
                        item["content"][0]["text"].as_str().unwrap().into(),
                    );
                }
            }
        }
    }
    messages
}

#[tokio::test]
async fn drifting_native_ids_assemble_one_answer_and_restore_authoritative_handles() {
    let original = fixture();
    assert!(
        assembled(&original).len() > 1,
        "fixture must reproduce the client's duplicate assembly"
    );
    let store = store();
    let repaired = events(&replay(wire(&original), store.clone()).await);
    assert_eq!(assembled(&repaired).len(), 1);
    assert_eq!(assembled(&repaired).values().next().unwrap(), "你好 world");
    for e in &repaired {
        if let Some(id) = e.get("item_id") {
            assert_eq!(id, "msg-start");
        }
        if let Some(item) = e.get("item") {
            assert_eq!(item["id"], "msg-start");
        }
        if let Some(response) = e.get("response") {
            assert_eq!(response["id"], "resp-start");
        }
    }
    assert_eq!(
        repaired.last().unwrap()["response"]["usage"]["total_tokens"],
        7
    );
    let mut next = json!({"previous_response_id":"resp-start","input":[{"id":"msg-start","type":"message","content":"fixture"}],"metadata":{"id":"msg-start"}});
    store.restore_request("session-a", &mut next).unwrap();
    assert_eq!(next["previous_response_id"], "resp-final");
    assert_eq!(next["input"][0]["id"], "msg-final");
    assert_eq!(next["metadata"]["id"], "msg-start");
}

#[tokio::test]
async fn stable_native_sse_is_byte_identical_including_heartbeats_and_unknown_fields() {
    let mut original = fixture();
    for e in &mut original {
        if let Some(r) = e.get_mut("response") {
            r["id"] = json!("r");
            if let Some(a) = r["output"].as_array_mut() {
                for i in a {
                    i["id"] = json!("m");
                }
            }
        }
        if let Some(i) = e.get_mut("item") {
            i["id"] = json!("m");
        }
        if let Some(id) = e.get_mut("item_id") {
            *id = json!("m");
        }
        e["extension"] = json!({"kept":true});
    }
    let mut raw = b": keepalive\r\n\r\n".to_vec();
    raw.extend(wire(&original));
    raw.extend(b"data: [DONE]\n\n");
    assert_eq!(replay(raw.clone(), store()).await, raw);
}

#[test]
fn different_items_tools_and_opaque_reasoning_keep_their_own_identity() {
    let store = store();
    let mut state = StreamIdentity::new(store.clone(), "session-a".into());
    let items = [
        json!({"id":"r-a","type":"reasoning","encrypted_content":"opaque-state","summary":[]}),
        json!({"id":"m-a","type":"message","phase":"commentary","content":[]}),
        json!({"id":"m-b","type":"message","phase":"final_answer","content":[]}),
        json!({"id":"f-a","type":"function_call","call_id":"call-a","name":"lookup","arguments":"{}"}),
        json!({"id":"f-b","type":"custom_tool_call","call_id":"call-b","name":"exec","input":"fixture"}),
    ];
    for (index, item) in items.iter().enumerate() {
        let mut added =
            json!({"type":"response.output_item.added","output_index":index,"item":item});
        state.rewrite(&mut added).unwrap();
        let mut done = added.clone();
        done["type"] = json!("response.output_item.done");
        done["item"]["id"] = json!(format!("upstream-done-{index}"));
        state.rewrite(&mut done).unwrap();
        assert_eq!(added["item"], done["item"]);
    }
    let mut arguments = json!({"type":"response.function_call_arguments.delta","output_index":3,"item_id":"f-drift","delta":"{}"});
    state.rewrite(&mut arguments).unwrap();
    assert_eq!(arguments["item_id"], "f-a");
    let mut next = json!({"input":items});
    store.restore_request("session-a", &mut next).unwrap();
    assert_eq!(next["input"][0]["encrypted_content"], "opaque-state");
    assert_eq!(next["input"][3]["call_id"], "call-a");
    assert_eq!(next["input"][4]["call_id"], "call-b");
    assert_eq!(next["input"][4]["id"], "upstream-done-4");
    let mut isolated = json!({"previous_response_id":"resp-start","input":[{"id":"f-a"}]});
    store
        .restore_request("another-account-or-session", &mut isolated)
        .unwrap();
    assert_eq!(isolated["input"][0]["id"], "f-a");
}

#[test]
fn reverse_map_survives_reopening_and_terminal_snapshot_can_restore_original_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let db = Arc::new(Database {
        conn: Mutex::new(Connection::open(&path).unwrap()),
    });
    let first = IdentityStore::new(db.clone());
    first
        .remember("scope", "item", "client", "upstream")
        .unwrap();
    drop(first);
    let restored = IdentityStore::new(db);
    let mut input = json!({"input":[{"type":"item_reference","id":"client"}]});
    restored.restore_request("scope", &mut input).unwrap();
    assert_eq!(input["input"][0]["id"], "upstream");
    restored
        .remember("scope", "item", "client", "client")
        .unwrap();
    let mut input = json!({"input":[{"id":"client"}]});
    restored.restore_request("scope", &mut input).unwrap();
    assert_eq!(input["input"][0]["id"], "client");
}

#[tokio::test]
async fn concurrent_idless_streams_are_distinct_and_eof_does_not_forge_completion() {
    let frames = vec![
        json!({"type":"response.created","response":{"output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":null,"delta":"partial"}),
    ];
    let store = store();
    let (a, b) = tokio::join!(
        replay(wire(&frames), store.clone()),
        replay(wire(&frames), store)
    );
    let a = events(&a);
    let b = events(&b);
    assert_ne!(a[0]["response"]["id"], b[0]["response"]["id"]);
    assert_ne!(a[1]["item"]["id"], b[1]["item"]["id"]);
    assert_eq!(a.len(), 3);
    assert_eq!(a[1]["item"]["id"], a[2]["item_id"]);
}

#[tokio::test]
async fn streams_immediately_and_drop_cancels_without_replay() {
    struct Guard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = Guard(dropped.clone());
    let upstream = async_stream::stream! {
        let _guard=guard;
        yield Ok(Bytes::from(wire(&fixture()[..2])));
        std::future::pending::<()>().await;
    };
    let mut output = Box::pin(stream(upstream, store(), "s".into()));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), output.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    drop(output);
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
}
