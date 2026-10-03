use super::*;
fn item(id: &str, endpoints: Value) -> Value {
    json!({"id":id,"name":id,"model_picker_enabled":true,"supported_endpoints":endpoints,"capabilities":{"type":"chat","limits":{"max_context_window_tokens":128000,"max_output_tokens":16000},"supports":{"tool_calls":true,"vision":true}}})
}
fn source() -> Source {
    Source {
        id: PROVIDER_ID.into(),
        name: "Copilot".into(),
        base_url: API_ROOT.into(),
        key_env: String::new(),
        key_ref: None,
        copilot: Some(Binding {
            account_id: "123".into(),
            grant_id: uuid::Uuid::new_v4().to_string(),
        }),
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models: parse_models(&json!({"data":[item("gpt-6",json!(["/responses"]))]})).unwrap(),
    }
}
#[test]
fn catalog_is_namespaced_capability_driven_and_does_not_guess_limits() {
    let mut blocked = item("policy-disabled", json!(["/chat/completions"]));
    blocked["policy"] = json!({"state":"disabled"});
    let mut utility = item("utility", json!(["/chat/completions"]));
    utility["model_picker_enabled"] = json!(false);
    let items=parse_models(&json!({"data":[item("gpt-6",json!(["/responses"])),item("chat",json!(["/chat/completions"])),item("embedding",json!(["/embeddings"])),blocked,utility]})).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "copilot/gpt-6");
    assert_eq!(items[0].protocol_override, Some(Protocol::OpenaiResponses));
    assert_eq!(items[0].context_window, Some(128000));
    let mut effective = items[0].clone();
    crate::manual::limits::apply(&mut effective);
    assert_eq!(effective.context_window, Some(128000));
    assert!(effective.limits_source.is_none());
    assert_eq!(upstream_id(&items[0]).unwrap(), "gpt-6");
}
#[test]
fn refreshing_copilot_does_not_erase_other_provider_or_legacy_test_records() {
    let mut doc = crate::manual::catalog::Document::default();
    let result = crate::manual::catalog::TestResult {
        success: true,
        latency_ms: 1,
        checked_at: "fixture".into(),
        detail: "OK".into(),
        provider_id: None,
        provider_name: None,
        model_id: None,
    };
    doc.tests.insert("legacy-api-group".into(), result.clone());
    doc.tests.insert(
        crate::manual::catalog::test_key("api-model", "api-source"),
        result.clone(),
    );
    doc.tests.insert(
        crate::manual::catalog::test_key("copilot/model", PROVIDER_ID),
        result,
    );
    crate::manual::controls::clear_source_tests(&mut doc, PROVIDER_ID);
    assert_eq!(doc.tests.len(), 2);
    assert!(doc.tests.contains_key("legacy-api-group"));
    assert!(doc
        .tests
        .contains_key(&crate::manual::catalog::test_key("api-model", "api-source")));
}
#[test]
fn a_test_requires_protocol_completion_not_just_http_or_partial_output() {
    assert!(validate_test_response(
        &json!({"status":"incomplete","output":[{"type":"reasoning"}]})
    )
    .is_err());
    assert!(validate_test_response(
        &json!({"choices":[{"finish_reason":"length","message":{"content":"partial"}}]})
    )
    .is_err());
    assert!(
        validate_test_response(&json!({"status":"completed","output":[{"type":"message"}]}))
            .is_ok()
    );
    assert!(validate_test_response(
        &json!({"stop_reason":"end_turn","content":[{"type":"text","text":"OK"}]})
    )
    .is_ok());
}
#[test]
fn endpoint_seam_cannot_send_tokens_to_arbitrary_hosts_or_paths() {
    for url in [
        "https://api.githubcopilot.com",
        "https://api.individual.githubcopilot.com/",
    ] {
        assert!(validate_endpoint(url).is_ok());
    }
    for url in [
        "http://api.githubcopilot.com",
        "https://githubcopilot.com.evil.test",
        "https://evil.test/githubcopilot.com",
        "https://user@api.githubcopilot.com",
        "https://api.githubcopilot.com/v1",
        "https://api.githubcopilot.com?token=x",
    ] {
        assert!(validate_endpoint(url).is_err());
    }
    assert_eq!(
        inference_url("https://api.githubcopilot.com/v1", "/v1/responses").unwrap(),
        "https://api.githubcopilot.com/responses"
    );
    assert_eq!(
        inference_url(
            "https://api.githubcopilot.com",
            "/chat/completions?beta=true"
        )
        .unwrap(),
        "https://api.githubcopilot.com/chat/completions"
    );
    assert!(inference_url(API_ROOT, "/v1/responses/compact").is_err());
    assert_eq!(
        headers("fixture-not-a-real-token").unwrap()["x-initiator"],
        "user"
    );
    assert!(headers("bad\nvalue").is_err());
}
#[test]
fn api_credentials_and_existing_affinity_hashes_are_unchanged() {
    let mut s = source();
    s.copilot = None;
    s.id = "api".into();
    s.models.clear();
    let expected = crate::manual::catalog::hash(
        &json!([s.base_url, s.protocol, s.key_env, s.key_ref, null]).to_string(),
    );
    assert_eq!(crate::manual::keys::credential_version(&s), expected);
    let first = source();
    let mut next = first.clone();
    next.copilot.as_mut().unwrap().grant_id = uuid::Uuid::new_v4().to_string();
    assert_ne!(
        crate::manual::keys::credential_version(&first),
        crate::manual::keys::credential_version(&next)
    );
}
#[test]
fn ordinary_save_can_toggle_models_but_cannot_forge_oauth_bindings_or_catalogs() {
    let current = crate::manual::catalog::Document {
        providers: vec![source()],
        ..Default::default()
    };
    let mut next = current.clone();
    next.providers[0].models[0].enabled = false;
    assert!(validate_document_edit(&current, &next).is_ok());
    next.providers[0].models[0].context_window = Some(9999999);
    assert!(validate_document_edit(&current, &next).is_err());
    let mut next = current.clone();
    next.providers[0].copilot.as_mut().unwrap().account_id = "999".into();
    assert!(validate_document_edit(&current, &next).is_err());
    assert!(validate_document_edit(&current, &Default::default()).is_err());
    assert!(validate_document_edit(&Default::default(), &current).is_err());
}
#[tokio::test]
async fn persisted_secret_stays_private_and_snapshot_only_exposes_account_identity() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("copilot"));
    let c = Credential {
        version: 1,
        account_id: "123".into(),
        login: "fixture-user".into(),
        github_token: "fixture-private-oauth-material".into(),
        grant_id: uuid::Uuid::new_v4().to_string(),
    };
    manager.persist(&c).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&manager.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(manager.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let loaded = Manager::new(dir.path().join("copilot"));
    let status = loaded
        .status(Some(&Binding {
            account_id: c.account_id.clone(),
            grant_id: c.grant_id.clone(),
        }))
        .await;
    assert!(status.connected);
    assert!(!serde_json::to_string(&status)
        .unwrap()
        .contains("private-oauth"));
    assert!(loaded
        .resolve(&Binding {
            account_id: "another".into(),
            grant_id: c.grant_id
        })
        .await
        .is_err());
}
#[tokio::test]
async fn device_flow_honors_poll_interval_slowdown_expiry_and_cancel() {
    use axum::{routing::post, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let app=Router::new().route("/login/device/code",post(||async{Json(json!({"device_code":"private-device","user_code":"TEST-CODE","expires_in":900,"interval":5}))})).route("/login/oauth/access_token",post(move||{let calls=calls.clone();async move{calls.fetch_add(1,Ordering::SeqCst);Json(json!({"error":"slow_down"}))}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::at(dir.path().into(), base.clone(), base);
    let c = manager.start(7).await.unwrap();
    assert!(!serde_json::to_string(&c)
        .unwrap()
        .contains("private-device"));
    assert!(matches!(
        manager.poll(&c.flow_id).await.unwrap(),
        Poll::Pending(5)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    manager.device.lock().await.as_mut().unwrap().next_poll = Instant::now();
    assert!(matches!(
        manager.poll(&c.flow_id).await.unwrap(),
        Poll::Pending(10)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(matches!(
        manager.poll(&c.flow_id).await.unwrap(),
        Poll::Pending(10)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    manager.cancel(&c.flow_id).await;
    assert!(manager.poll(&c.flow_id).await.is_err());
    assert!(!manager.path.exists());
    let next = manager.start(7).await.unwrap();
    manager.device.lock().await.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
    match manager.poll(&next.flow_id).await {
        Err(error) => assert!(error.contains("过期")),
        Ok(_) => panic!("expired flow accepted"),
    };
    task.abort();
}
