use super::*;

#[tokio::test]
async fn manual_gateway_refuses_non_loopback_listener() {
    let db = Arc::new(Database::memory().unwrap());
    Document::default().save(&db).unwrap();
    let server = ProxyServer::new(
        ProxyConfig {
            listen_address: "0.0.0.0".into(),
            listen_port: 0,
            ..ProxyConfig::default()
        },
        db,
        None,
    );
    assert!(server.start().await.is_err());
}

#[tokio::test]
async fn manual_logs_capture_http_outcomes_without_content_or_credentials() {
    let (upstream, mock, task) = mock_upstream().await;
    let (server, base, _) = gateway(vec![source("a", &upstream)]).await;
    let client = reqwest::Client::new();
    let body = json!({"model":"model-a","messages":[{"role":"user","content":"PRIVATE_PROMPT_SENTINEL"}],"metadata":{"private":"PRIVATE_METADATA_SENTINEL"}});
    let reply = client
        .post(format!(
            "{base}/v1/chat/completions?api_key=PRIVATE_QUERY_SENTINEL"
        ))
        .bearer_auth("PRIVATE_AUTH_SENTINEL")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let unknown = client
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":"PRIVATE_UNKNOWN_MODEL_SENTINEL","input":"PRIVATE_INPUT_SENTINEL"}))
        .send()
        .await
        .unwrap();
    assert!(!unknown.status().is_success());
    assert_eq!(
        client
            .post(format!("{base}/v1/responses"))
            .header("origin", "https://example.com")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let mut stream_body = body.clone();
    stream_body["stream"] = json!(true);
    let stream = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&stream_body)
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert!(stream.text().await.unwrap().contains("[DONE]"));
    mock.status.store(503, Ordering::SeqCst);
    let failed = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(!failed.status().is_success());
    let rows = server.manual_request_logs();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].status, failed.status().as_u16());
    assert_eq!(rows[0].model.as_deref(), Some("model-a"));
    assert!(rows[1].streaming);
    assert_eq!(rows[1].status, 200);
    assert_eq!(rows[2].endpoint, "/v1/models");
    assert_eq!(rows[3].status, 403);
    assert!(rows[3].model.is_none());
    assert!(rows[4].model.is_none());
    assert_eq!(rows[5].endpoint, "/v1/chat/completions");
    assert_eq!(rows[5].model.as_deref(), Some("model-a"));
    let serialized = serde_json::to_string(&rows).unwrap();
    assert!(serialized.contains("PRIVATE_PROMPT_SENTINEL"));
    for hidden in [
        "PRIVATE_AUTH_SENTINEL",
        "PRIVATE_QUERY_SENTINEL",
        "PRIVATE_METADATA_SENTINEL",
    ] {
        assert!(!serialized.contains(hidden));
    }
    assert!(serialized.contains("choices"));
    assert_eq!(rows[5].providers[0].name, "a");
    assert_eq!(rows[1].response_state, "已结束");
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("authorization"));
    assert_eq!(mock.calls.lock().await.len(), 3);
    server.stop().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn manual_gateway_uses_provider_key_not_client_credentials() {
    struct KeyEnv(String);
    impl Drop for KeyEnv {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
        }
    }
    let key_env = KeyEnv(format!("MANUAL_UNIT_KEY_{}", uuid::Uuid::new_v4().simple()));
    std::env::set_var(&key_env.0, "upstream-fixture-only");
    async fn upstream(headers: axum::http::HeaderMap, Json(body): Json<Value>) -> Response {
        assert_eq!(
            headers.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer upstream-fixture-only")
        );
        assert!(!headers.contains_key("x-api-key"));
        Json(json!({"id":"fixture","object":"chat.completion","model":body["model"],"choices":[{"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]})).into_response()
    }
    let app = Router::new().route("/v1/chat/completions", post(upstream));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut provider = source("a", &format!("http://{address}/v1"));
    provider.key_env = key_env.0.clone();
    let (server, base, _) = gateway(vec![provider]).await;
    let reply = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .bearer_auth("irrelevant-client-value")
        .header("x-api-key", "irrelevant-client-value")
        .json(&json!({"model":"model-a","input":"Hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    server.stop().await.unwrap();
    task.abort();
}
