use super::*;

#[tokio::test]
async fn empty_continuation_fields_are_not_account_bound_history() {
    let router =
        crate::proxy::provider_router::ProviderRouter::new(Arc::new(Database::memory().unwrap()));
    let doc = Document {
        providers: vec![source("a", "http://127.0.0.1:1234/v1")],
        ..Document::default()
    };
    for body in [
        json!({"model":"model-a","input":"Hello","previous_response_id":null}),
        json!({"model":"model-a","input":[{"type":"reasoning","encrypted_content":null}]}),
        json!({"model":"model-a","input":"Hello","tools":[{"parameters":{"properties":{"encrypted_content":{"type":"string"}}}}]}),
    ] {
        assert!(router
            .manual
            .select(&router, &doc, "codex", &body, None)
            .await
            .is_ok());
    }
}

#[tokio::test]
async fn bound_session_never_uses_a_backup_even_with_plain_history() {
    let router =
        crate::proxy::provider_router::ProviderRouter::new(Arc::new(Database::memory().unwrap()));
    let doc = Document {
        providers: vec![
            source("a", "http://127.0.0.1:1234/v1"),
            source("b", "http://127.0.0.1:1235/v1"),
        ],
        ..Document::default()
    };
    let version = catalog::deployment_version(&doc.providers[0], &doc.providers[0].models[0]);
    router.manual.bind("codex", "session", "a", &version).await;
    let chain = router
        .manual
        .select(
            &router,
            &doc,
            "codex",
            &json!({"model":"model-a","input":"Hello"}),
            Some("session"),
        )
        .await
        .unwrap();
    assert_eq!(
        chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["a"]
    );
}

#[tokio::test]
async fn session_initial_failover_is_logged_and_later_failure_stays_on_same_provider() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    first.status.store(503, Ordering::SeqCst);
    let (server, base, _) = gateway(vec![source("a", &a), source("b", &b)]).await;
    let client = reqwest::Client::new();
    let mut body = json!({"model":"model-a","prompt_cache_key":"session-fixture","input":[{"role":"user","content":"old question"},{"role":"assistant","content":"old answer"},{"role":"user","content":"latest question"}]});
    let reply = client
        .post(format!("{base}/v1/responses"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let _ = reply.text().await.unwrap();
    let rows = server.manual_request_logs();
    assert_eq!(
        rows[0]
            .providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(rows[0].providers[1].outcome, "已接收响应");
    let input = rows[0].input.to_string();
    assert!(input.contains("latest question"));
    assert!(!input.contains("old question"));
    first.status.store(200, Ordering::SeqCst);
    second.status.store(503, Ordering::SeqCst);
    body["previous_response_id"] = Value::Null;
    let failed = client
        .post(format!("{base}/v1/responses"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(!failed.status().is_success());
    let _ = failed.text().await.unwrap();
    assert_eq!(
        first.calls.lock().await.len(),
        1,
        "已绑定会话不能为了成功切回 A"
    );
    assert_eq!(second.calls.lock().await.len(), 2);
    let rows = server.manual_request_logs();
    assert_eq!(rows[0].providers.len(), 1);
    assert_eq!(rows[0].providers[0].id, "b");
    assert!(rows[0].route_note.as_ref().unwrap().contains("固定会话"));
    assert!(!rows[0].response.is_null());
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn concurrent_http_first_turns_do_not_split_providers() {
    async fn slow(HttpState(mock): HttpState<Mock>, Json(body): Json<Value>) -> Response {
        tokio::time::sleep(Duration::from_millis(50)).await;
        mock_chat(HttpState(mock), Json(body)).await
    }
    let first = Mock {
        calls: Arc::new(Mutex::new(vec![])),
        status: Arc::new(AtomicU16::new(200)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/v1/chat/completions", post(slow))
        .with_state(first.clone());
    let task_a = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (b, second, task_b) = mock_upstream().await;
    let (server, base, _) = gateway(vec![
        source("a", &format!("http://{address}/v1")),
        source("b", &b),
    ])
    .await;
    let client = reqwest::Client::new();
    let body = json!({"model":"model-a","prompt_cache_key":"parallel-session","input":"Hello"});
    let (one, two) = tokio::join!(
        client
            .post(format!("{base}/v1/responses"))
            .json(&body)
            .send(),
        client
            .post(format!("{base}/v1/responses"))
            .json(&body)
            .send(),
    );
    assert_eq!(one.unwrap().status(), StatusCode::OK);
    assert_eq!(two.unwrap().status(), StatusCode::OK);
    assert_eq!(first.calls.lock().await.len(), 2);
    assert!(second.calls.lock().await.is_empty());
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn corrupt_binding_store_does_not_silently_reassign_sessions() {
    let db = Arc::new(Database::memory().unwrap());
    db.set_setting("manual_gateway_affinity_v1", "broken-json")
        .unwrap();
    let router = crate::proxy::provider_router::ProviderRouter::new(db);
    let doc = Document {
        providers: vec![source("a", "http://127.0.0.1:1234/v1")],
        ..Document::default()
    };
    assert!(router
        .manual
        .select(
            &router,
            &doc,
            "codex",
            &json!({"model":"model-a","input":"Hello"}),
            Some("session")
        )
        .await
        .is_err());
}

#[tokio::test]
async fn changed_environment_credential_cannot_resume_a_bound_session() {
    struct Env(String);
    impl Drop for Env {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
        }
    }
    let env = Env(format!(
        "MANUAL_SESSION_KEY_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::env::set_var(&env.0, "first-fixture-key");
    let mut provider = source("a", "http://127.0.0.1:1234/v1");
    provider.key_env = env.0.clone();
    let version = catalog::deployment_version(&provider, &provider.models[0]);
    let doc = Document {
        providers: vec![provider],
        ..Document::default()
    };
    let router =
        crate::proxy::provider_router::ProviderRouter::new(Arc::new(Database::memory().unwrap()));
    router.manual.bind("codex", "session", "a", &version).await;
    std::env::set_var(&env.0, "second-fixture-key");
    assert!(router
        .manual
        .select(
            &router,
            &doc,
            "codex",
            &json!({"model":"model-a","input":"Hello"}),
            Some("session")
        )
        .await
        .is_err());
}

#[tokio::test]
async fn concurrent_first_turns_share_one_session_lease() {
    let routes = Arc::new(routing::ManualRoutes::default());
    let first = routes
        .session_guard("codex", "concurrent")
        .await
        .unwrap()
        .unwrap();
    let pending = routes.session_guard("codex", "concurrent");
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    routes.bind("codex", "concurrent", "a", "version").await;
    drop(first);
    assert!(pending.await.unwrap().is_some());
    assert!(routes
        .session_guard("codex", "concurrent")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn successful_session_binding_survives_router_recreation() {
    let db = Arc::new(Database::memory().unwrap());
    let doc = Document {
        providers: vec![
            source("a", "http://127.0.0.1:1234/v1"),
            source("b", "http://127.0.0.1:1235/v1"),
        ],
        ..Document::default()
    };
    let router = crate::proxy::provider_router::ProviderRouter::new(db.clone());
    let version = catalog::deployment_version(&doc.providers[1], &doc.providers[1].models[0]);
    router
        .manual
        .bind("codex", "private-session-id", "b", &version)
        .await;
    let restored = crate::proxy::provider_router::ProviderRouter::new(db.clone());
    let chain = restored.manual.select(&restored, &doc, "codex", &json!({"model":"model-a","input":[{"type":"reasoning","encrypted_content":"opaque-fixture"}]}), Some("private-session-id")).await.unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].id, "b");
    let stored = db
        .get_setting("manual_gateway_affinity_v1")
        .unwrap()
        .unwrap();
    assert!(!stored.contains("private-session-id"));
    assert!(!stored.contains("opaque-fixture"));
}
