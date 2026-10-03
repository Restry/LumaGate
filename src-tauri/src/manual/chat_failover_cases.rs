use super::*;

fn chat_body() -> Value {
    json!({"model":"model-a","messages":[{"role":"user","content":"fixture question"}],"stream":false})
}

#[tokio::test]
async fn bound_plain_chat_retries_other_provider_on_429_and_remembers_success() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    let (server, base, _) = gateway(vec![source("a", &a), source("b", &b)]).await;
    let client = reqwest::Client::new();
    let body = chat_body();
    let url = format!("{base}/v1/chat/completions");
    let first_response = client
        .post(&url)
        .header("x-session-id", "plain-chat-session")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);
    first_response.bytes().await.unwrap();
    assert_eq!(first.calls.lock().await.len(), 1);
    assert!(second.calls.lock().await.is_empty());
    first.status.store(429, Ordering::SeqCst);
    let fallback = client
        .post(&url)
        .header("x-session-id", "plain-chat-session")
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = fallback.status();
    fallback.bytes().await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "a bound replayable Chat must try B when A is rate limited"
    );
    assert_eq!(second.calls.lock().await.len(), 1);
    let rows = server.manual_request_logs();
    assert_eq!(
        rows[0]
            .providers
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(rows[0].providers[0].outcome, "失败（HTTP 429）");
    assert!(rows[0]
        .route_note
        .as_deref()
        .unwrap()
        .contains("同模型回退"));
    let third = client
        .post(&url)
        .header("x-session-id", "plain-chat-session")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(third.status(), StatusCode::OK);
    third.bytes().await.unwrap();
    assert_eq!(
        first.calls.lock().await.len(),
        2,
        "B must become the preference; do not hit throttled A again"
    );
    assert_eq!(second.calls.lock().await.len(), 2);
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn signed_chat_stays_bound_and_failover_off_remains_respected() {
    let db = Arc::new(Database::memory().unwrap());
    let router = crate::proxy::provider_router::ProviderRouter::new(db);
    let mut doc = Document {
        providers: vec![
            source("a", "http://127.0.0.1:1234/v1"),
            source("b", "http://127.0.0.1:1235/v1"),
        ],
        ..Document::default()
    };
    let version = catalog::deployment_version(&doc.providers[0], &doc.providers[0].models[0]);
    router.manual.bind("codex", "session", "a", &version).await;
    for field in [
        "encrypted_content",
        "signature",
        "thought_signature",
        "thoughtSignature",
    ] {
        let mut body = chat_body();
        body["messages"] = json!([{"role":"assistant","content":[{"type":"thinking",(field):"private-state"}]},{"role":"user","content":"next"}]);
        let chain = router
            .manual
            .select(&router, &doc, "codex", &body, Some("session"))
            .await
            .unwrap();
        assert_eq!(
            chain.len(),
            1,
            "{field} must prohibit cross-account retries"
        );
        assert_eq!(chain[0].id, "a");
        assert!(router
            .manual
            .select(&router, &doc, "codex", &body, None)
            .await
            .is_err());
    }
    for field in ["previous_response_id", "conversation", "cached_content"] {
        let mut body = chat_body();
        body[field] = json!("server-state");
        let chain = router
            .manual
            .select(&router, &doc, "codex", &body, Some("session"))
            .await
            .unwrap();
        assert_eq!(chain.len(), 1);
    }
    // A Chat-shaped body never loosens a Responses model's account binding.
    let mut responses_doc = doc.clone();
    for provider in &mut responses_doc.providers {
        provider.models[0].protocol_override = Some(Protocol::OpenaiResponses);
    }
    let responses_version = catalog::deployment_version(
        &responses_doc.providers[0],
        &responses_doc.providers[0].models[0],
    );
    router
        .manual
        .bind("codex", "responses-session", "a", &responses_version)
        .await;
    let chain = router
        .manual
        .select(
            &router,
            &responses_doc,
            "codex",
            &chat_body(),
            Some("responses-session"),
        )
        .await
        .unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].id, "a");
    let id = doc.groups()[0].id.clone();
    doc.policies.insert(
        id,
        catalog::Policy {
            failover: false,
            ..catalog::Policy::default()
        },
    );
    let chain = router
        .manual
        .select(&router, &doc, "codex", &chat_body(), Some("session"))
        .await
        .unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].id, "a");
}

#[tokio::test]
async fn replayable_chat_skips_an_open_circuit_and_keeps_preference_after_reload() {
    let db = Arc::new(Database::memory().unwrap());
    let router = crate::proxy::provider_router::ProviderRouter::new(db.clone());
    let doc = Document {
        providers: vec![
            source("a", "http://127.0.0.1:1234/v1"),
            source("b", "http://127.0.0.1:1235/v1"),
        ],
        ..Document::default()
    };
    let version = catalog::deployment_version(&doc.providers[0], &doc.providers[0].models[0]);
    router.manual.bind("codex", "session", "a", &version).await;
    for _ in 0..20 {
        let _ = router
            .record_result("a", "codex", false, false, Some("fixture 429".into()))
            .await;
    }
    assert!(!router.manual_available("a", "codex").await);
    let chain = router
        .manual
        .select(&router, &doc, "codex", &chat_body(), Some("session"))
        .await
        .unwrap();
    assert_eq!(chain[0].id, "b");
    // Preference persistence is exercised through the same completion seam as the forwarder.
    router
        .manual
        .remember_success(
            "codex",
            "session",
            "b",
            &catalog::deployment_version(&doc.providers[1], &doc.providers[1].models[0]),
            true,
        )
        .await;
    let restored = crate::proxy::provider_router::ProviderRouter::new(db);
    let chain = restored
        .manual
        .select(&restored, &doc, "codex", &chat_body(), Some("session"))
        .await
        .unwrap();
    assert_eq!(chain[0].id, "b");
}
