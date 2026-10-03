use super::*;
use axum::{routing::post, Router};
use catalog::{Model, Protocol, Source};
use std::sync::atomic::{AtomicU16, Ordering};
#[path = "chat_failover_cases.rs"]
mod chat_failover_cases;
#[path = "model_controls_cases.rs"]
mod controls_cases;
#[path = "local_access_cases.rs"]
mod local_access_cases;
#[path = "outcome_cases.rs"]
mod outcome_cases;
#[path = "session_cases.rs"]
mod session_cases;

#[derive(Clone)]
struct Mock {
    calls: Arc<Mutex<Vec<Value>>>,
    status: Arc<AtomicU16>,
}
async fn mock_chat(HttpState(state): HttpState<Mock>, Json(body): Json<Value>) -> Response {
    state.calls.lock().await.push(body.clone());
    let status = StatusCode::from_u16(state.status.load(Ordering::SeqCst)).unwrap();
    if !status.is_success() {
        return (
            status,
            Json(json!({"error":{"message":"mock unavailable"}})),
        )
            .into_response();
    }
    if body["stream"] == true {
        let chunk = json!({"id":"chat-mock","object":"chat.completion.chunk","model":body["model"],"choices":[{"index":0,"delta":{"content":"OK"},"finish_reason":null}]});
        return (
            [("content-type", "text/event-stream")],
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        )
            .into_response();
    }
    Json(json!({"id":"chat-mock","object":"chat.completion","model":body["model"],"choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})).into_response()
}
async fn mock_upstream() -> (String, Mock, tokio::task::JoinHandle<()>) {
    let mock = Mock {
        calls: Arc::new(Mutex::new(vec![])),
        status: Arc::new(AtomicU16::new(200)),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/v1"), mock, task)
}
fn source(id: &str, base: &str) -> Source {
    Source {
        id: id.into(),
        name: id.into(),
        base_url: base.into(),
        key_env: String::new(),
        key_ref: None,
        copilot: None,
        enabled: true,
        protocol: Protocol::OpenaiChat,
        models: vec![Model {
            id: "model-a".into(),
            image: false,
            tools: None,
            context_window: None,
            enabled: true,
            ..Model::default()
        }],
    }
}
pub(super) async fn gateway(providers: Vec<Source>) -> (ProxyServer, String, Document) {
    gateway_document(Document {
        providers,
        ..Document::default()
    })
    .await
}
async fn gateway_document(mut doc: Document) -> (ProxyServer, String, Document) {
    let db = Arc::new(Database::memory().unwrap());
    doc.save(&db).unwrap();
    mirror_sources(&db, &doc).unwrap();
    for name in ["claude", "codex"] {
        let mut config = db.get_proxy_config_for_app(name).await.unwrap();
        config.auto_failover_enabled = true;
        config.enabled = false;
        config.max_retries = 3;
        db.update_proxy_config_for_app(config).await.unwrap();
    }
    let server = ProxyServer::new(
        ProxyConfig {
            listen_port: 0,
            ..ProxyConfig::default()
        },
        db,
        None,
    );
    let info = server.start().await.unwrap();
    (server, format!("http://127.0.0.1:{}", info.port), doc)
}

#[tokio::test]
async fn manual_responses_test_accepts_null_error_and_preserves_base_path() {
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let reply = Arc::new(Mutex::new(json!({
        "id":"resp-fixture", "object":"response", "model":"gpt-6-astra",
        "status":"completed", "error":null,
        "output":[{"id":"msg-fixture","type":"message","role":"assistant",
            "content":[{"type":"output_text","text":"OK"}]}],
        "usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}
    })));
    let captured = calls.clone();
    let response_body = reply.clone();
    let app = Router::new().route(
        "/openai/v1/responses",
        post(
            move |axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
                  Json(body): Json<Value>| {
                let calls = captured.clone();
                let reply = response_body.clone();
                async move {
                    calls
                        .lock()
                        .await
                        .push(json!({"path":uri.path(),"body":body}));
                    Json(reply.lock().await.clone())
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut provider = source("prefix-fixture", &format!("http://{address}/openai/v1"));
    provider.models[0].id = "gpt-6-astra".into();

    // 夹具保留真实 Responses 成功响应中的 error:null，不能把可空字段当成失败。
    let result = test_model_once(&provider, &provider.models[0]).await;
    assert_eq!(calls.lock().await.len(), 1);
    assert_eq!(calls.lock().await[0]["path"], "/openai/v1/responses");
    assert_eq!(calls.lock().await[0]["body"]["model"], "gpt-6-astra");
    assert_eq!(calls.lock().await[0]["body"]["stream"], false);
    assert!(result.is_ok(), "成功响应不应被判为失败：{result:?}");

    // 同时经过原有转发器，避免只有测试按钮正确、实际 Agent 转发却丢掉前缀。
    let (server, base, doc) = gateway(vec![provider.clone()]).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":doc.groups()[0].id,"input":"Reply with OK.","stream":false,"max_output_tokens":64}))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"][0]["content"][0]["text"], "OK");
    assert_eq!(calls.lock().await.len(), 2);
    assert_eq!(calls.lock().await[1]["path"], "/openai/v1/responses");
    assert_eq!(calls.lock().await[1]["body"]["model"], "gpt-6-astra");
    server.stop().await.unwrap();

    for invalid in [
        json!({"error":{"message":"fixture failure"},"output":[{}]}),
        json!({"status":"failed","error":null,"output":[{}]}),
        json!({"status":"completed","error":null,"output":[]}),
    ] {
        *reply.lock().await = invalid;
        let before = calls.lock().await.len();
        assert!(test_model_once(&provider, &provider.models[0])
            .await
            .is_err());
        assert_eq!(calls.lock().await.len(), before + 1);
    }
    task.abort();
}

#[tokio::test]
async fn disabled_provider_model_is_not_forwarded() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    let current = Document {
        providers: vec![source("a", &a), source("b", &b)],
        ..Document::default()
    };
    let mut proposed = current.clone();
    proposed.providers[0].models[0].enabled = false;
    let db = Database::memory().unwrap();
    keys::save_with_keys(
        &current,
        proposed,
        None,
        &keys::FileKeyStore::default(),
        |document| document.save(&db),
    )
    .unwrap();
    let saved = Document::load(&db).unwrap();
    let (server, base, doc) = gateway(saved.providers).await;
    let groups = doc.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].provider_ids, ["b"]);
    for _ in 0..3 {
        let response = reqwest::Client::new().post(format!("{base}/v1/chat/completions"))
            .bearer_auth(LOCAL_TOKEN)
            .json(&json!({"model":groups[0].id,"messages":[{"role":"user","content":"Hello"}],"stream":false}))
            .send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert!(first.calls.lock().await.is_empty());
    assert_eq!(second.calls.lock().await.len(), 3);
    second.status.store(503, Ordering::SeqCst);
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":groups[0].id,"messages":[{"role":"user","content":"Hello"}],"stream":false}))
        .send().await.unwrap();
    assert!(!response.status().is_success());
    assert!(
        first.calls.lock().await.is_empty(),
        "其他线路失败也不能重新启用停用的部署"
    );
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn manual_gateway_balances_and_maps_real_model_without_config_writes() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    let (server, base, doc) = gateway(vec![source("a", &a), source("b", &b)]).await;
    let client = reqwest::Client::new();
    let id = doc.groups()[0].id.clone();
    for _ in 0..4 {
        let response = client
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(LOCAL_TOKEN)
            .json(
                &json!({"model":id,"messages":[{"role":"user","content":"Hello"}],"stream":false}),
            )
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{status}: {text}");
    }
    assert_eq!(first.calls.lock().await.len(), 2);
    assert_eq!(second.calls.lock().await.len(), 2);
    assert_eq!(first.calls.lock().await[0]["model"], "model-a");
    let models: Value = client
        .get(format!("{base}/v1/models"))
        .bearer_auth(LOCAL_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["data"].as_array().unwrap().len(), 1);
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn manual_gateway_fails_over_before_response_and_preserves_sse() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    first.status.store(503, Ordering::SeqCst);
    let (server, base, doc) = gateway(vec![source("a", &a), source("b", &b)]).await;
    let response = reqwest::Client::new().post(format!("{base}/v1/chat/completions")).bearer_auth(LOCAL_TOKEN).json(&json!({"model":doc.groups()[0].id,"messages":[{"role":"user","content":"Hello"}],"stream":true})).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    assert!(text.contains("[DONE]"));
    assert_eq!(first.calls.lock().await.len(), 1);
    assert_eq!(second.calls.lock().await.len(), 1);
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn manual_models_read_without_auth_returns_json() {
    let (server, base, _) = gateway(vec![]).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("application/json"));
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({"object":"list","data":[]}));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn manual_gateway_allows_local_clients_without_marker() {
    let (upstream, mock, task) = mock_upstream().await;
    let (server, base, _) = gateway(vec![source("a", &upstream)]).await;
    let client = reqwest::Client::new();
    for (path, body) in [
        ("responses", json!({"model":"model-a","input":"test"})),
        (
            "chat/completions",
            json!({"model":"model-a","messages":[{"role":"user","content":"test"}]}),
        ),
        (
            "messages",
            json!({"model":"model-a","max_tokens":32,"messages":[{"role":"user","content":"test"}]}),
        ),
    ] {
        let response = client
            .post(format!("{base}/v1/{path}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "本机调用不应要求标记：{path}"
        );
    }
    assert_eq!(
        client
            .post(format!("{base}/v1/responses"))
            .bearer_auth("unused-client-placeholder")
            .json(&json!({"model":"model-a","input":"test"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for path in ["models", "responses"] {
        assert_eq!(
            client
                .post(format!("{base}/v1/{path}"))
                .header("origin", "https://example.com")
                .json(&json!({"model":"model-a","input":"test"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(format!("{base}/v1/{path}"))
                .header("host", "example.com")
                .json(&json!({"model":"model-a","input":"test"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(mock.calls.lock().await.len(), 4);
    server.stop().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn manual_endpoint_selection_controls_real_upstream_path() {
    async fn responses(HttpState(mock): HttpState<Mock>, Json(body): Json<Value>) -> Response {
        let mut recorded = body.clone();
        recorded["_endpoint"] = json!("responses");
        mock.calls.lock().await.push(recorded);
        Json(json!({"id":"resp-mock","object":"response","status":"completed","model":body["model"],"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}],"usage":{"input_tokens":1,"output_tokens":1}})).into_response()
    }
    let mock = Mock {
        calls: Arc::new(Mutex::new(vec![])),
        status: Arc::new(AtomicU16::new(200)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat))
        .route("/v1/responses", post(responses))
        .with_state(mock.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut provider = source("a", &format!("http://{address}/v1"));
    provider.models[0].id = "gpt-5.5".into();
    provider.models.push(Model {
        id: "gemini-3-pro".into(),
        ..provider.models[0].clone()
    });
    let (server, base, doc) = gateway(vec![provider]).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/v1/responses"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":"gpt-5.5","input":"Hi"}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    assert_eq!(mock.calls.lock().await[0]["_endpoint"], "responses");
    let reply = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":"gemini-3-pro","messages":[{"role":"user","content":"Hi"}]}))
        .send()
        .await
        .unwrap();
    assert!(reply.status().is_success());
    let wrong = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":"gpt-5.5","messages":[{"role":"user","content":"Hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 400);
    assert_eq!(mock.calls.lock().await.len(), 2);
    let catalog = catalog::public_models(&doc);
    assert_eq!(
        catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == "gpt-5.5")
            .unwrap()["endpoint"],
        "/v1/responses"
    );
    server.stop().await.unwrap();
    task.abort();
}

#[test]
fn manual_endpoint_override_preserves_policy_and_legacy_alias() {
    let mut provider = source("a", "https://example.com/v1");
    provider.models[0].id = "gpt-5.5".into();
    let legacy = catalog::group_id(&Protocol::OpenaiChat, &provider.models[0]);
    let mut doc = Document {
        providers: vec![provider],
        ..Document::default()
    };
    doc.policies.insert(
        legacy.clone(),
        catalog::Policy {
            balance: catalog::Balance::Priority,
            failover: false,
            fallbacks: vec![],
        },
    );
    let group = doc.groups().remove(0);
    assert_eq!(group.protocol, Protocol::OpenaiResponses);
    assert!(!group.policy.failover);
    assert!(catalog::resolve_group(&doc.groups(), &legacy).is_some());
    endpoints::set_override(&mut doc, &group.id, Some(Protocol::OpenaiChat)).unwrap();
    let group = doc.groups().remove(0);
    assert_eq!(group.protocol, Protocol::OpenaiChat);
    assert_eq!(group.protocol_mode, "manual");
    assert!(!group.policy.failover);
    endpoints::set_override(&mut doc, &group.id, None).unwrap();
    assert_eq!(doc.groups()[0].protocol, Protocol::OpenaiResponses);
}

#[tokio::test]
async fn manual_gateway_reuses_anthropic_bridge() {
    let (a, mock, task) = mock_upstream().await;
    let (server, base, doc) = gateway(vec![source("a", &a)]).await;
    let response=reqwest::Client::new().post(format!("{base}/v1/messages")).header("x-api-key",LOCAL_TOKEN).header("anthropic-version","2023-06-01").json(&json!({"model":doc.groups()[0].id,"max_tokens":64,"messages":[{"role":"user","content":"Hello"}]})).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["content"][0]["text"], "OK");
    assert_eq!(mock.calls.lock().await[0]["model"], "model-a");
    server.stop().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn manual_gateway_reuses_responses_bridge() {
    let (a, mock, task) = mock_upstream().await;
    let (server, base, doc) = gateway(vec![source("a", &a)]).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":doc.groups()[0].id,"input":"Hello","stream":false}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    assert!(body["output"].as_array().is_some_and(|v| !v.is_empty()));
    assert_eq!(mock.calls.lock().await[0]["model"], "model-a");
    server.stop().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn manual_route_counters_are_per_model_and_opaque_turns_are_pinned() {
    let db = Arc::new(Database::memory().unwrap());
    let router = crate::proxy::provider_router::ProviderRouter::new(db);
    let mut a = source("a", "http://127.0.0.1:1234/v1");
    a.models.push(Model {
        id: "model-b".into(),
        ..a.models[0].clone()
    });
    let mut b = a.clone();
    b.id = "b".into();
    let doc = Document {
        providers: vec![a, b],
        ..Document::default()
    };
    for expected in ["a", "b"] {
        for model in ["model-a", "model-b"] {
            let chain = router
                .manual
                .select(&router, &doc, "codex", &json!({"model":model}), None)
                .await
                .unwrap();
            assert_eq!(chain[0].id, expected);
        }
    }
    let turn = json!({"model":"model-a","previous_response_id":"resp-1"});
    assert!(router
        .manual
        .select(&router, &doc, "codex", &turn, None)
        .await
        .is_err());
    let version = catalog::deployment_version(&doc.providers[1], &doc.providers[1].models[0]);
    router
        .manual
        .bind("codex", "session-1", "b", &version)
        .await;
    let chain = router
        .manual
        .select(&router, &doc, "codex", &turn, Some("session-1"))
        .await
        .unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].id, "b");
    let mut rotated = doc.clone();
    rotated.providers[1].key_ref = Some(uuid::Uuid::new_v4().to_string());
    assert!(router
        .manual
        .select(&router, &rotated, "codex", &turn, Some("session-1"))
        .await
        .is_err());
}

#[tokio::test]
async fn manual_route_fallback_order_and_disabled_retry() {
    let db = Arc::new(Database::memory().unwrap());
    let router = crate::proxy::provider_router::ProviderRouter::new(db);
    let a = source("a", "http://127.0.0.1:1234/v1");
    let mut b = a.clone();
    b.id = "b".into();
    b.models[0].id = "model-b".into();
    let mut doc = Document {
        providers: vec![a, b],
        ..Document::default()
    };
    let groups = doc.groups();
    let root = &groups.iter().find(|g| g.model.id == "model-a").unwrap().id;
    let target = &groups.iter().find(|g| g.model.id == "model-b").unwrap().id;
    doc.policies.insert(
        root.to_string(),
        catalog::Policy {
            balance: catalog::Balance::Priority,
            failover: true,
            fallbacks: vec![target.to_string()],
        },
    );
    doc.validate().unwrap();
    let chain = router
        .manual
        .select(&router, &doc, "codex", &json!({"model":root}), None)
        .await
        .unwrap();
    assert_eq!(
        chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(chain[1].settings_config["manual_upstream_model"], "model-b");
    doc.policies.get_mut(root).unwrap().failover = false;
    assert_eq!(
        router
            .manual
            .select(&router, &doc, "codex", &json!({"model":root}), None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn manual_discovery_reads_pages_without_inference() {
    async fn page(
        axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
    ) -> Json<Value> {
        if query.contains_key("after_id") {
            Json(json!({"data":[{"id":"b","context_window":32000}],"has_more":false}))
        } else {
            Json(
                json!({"data":[{"id":"a","architecture":{"input_modalities":["text","image"]},"capabilities":{"tools":true}}],"last_id":"a","has_more":true}),
            )
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/models", axum::routing::get(page));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut provider = source("a", &format!("http://{addr}/v1"));
    provider.protocol = Protocol::Anthropic;
    provider.models[0].id = "a".into();
    provider.models[0].enabled = false;
    provider.models[0].reasoning = Some(true);
    provider.models[0].protocol_override = Some(Protocol::OpenaiResponses);
    provider.models[0].max_output_tokens = Some(128000);
    let models = discover_models(&provider).await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(models[0].image);
    assert_eq!(models[0].tools, Some(true));
    assert!(!models[0].enabled);
    assert_eq!(models[0].reasoning, None);
    assert_eq!(models[0].protocol_override, Some(Protocol::OpenaiResponses));
    assert_eq!(models[0].max_output_tokens, None);
    assert_eq!(models[0].metadata["id"], "a");
    assert_eq!(models[1].context_window, Some(32000));
    task.abort();
}
