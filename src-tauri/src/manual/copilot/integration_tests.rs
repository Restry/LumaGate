use super::*;
use crate::{database::Database, manual::catalog::Document};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn credential() -> Credential {
    Credential {
        version: 1,
        account_id: "123".into(),
        login: "fixture-user".into(),
        github_token: "fixture-private-oauth-token".into(),
        grant_id: uuid::Uuid::new_v4().to_string(),
    }
}
async fn ready(manager: &Manager, revision: u64) -> Authorized {
    *manager.device.lock().await = Some(Device {
        id: "flow-fixture".into(),
        code: "device-private".into(),
        expires: Instant::now() + Duration::from_secs(600),
        interval: 5,
        next_poll: Instant::now(),
        polling: false,
        revision,
    });
    Authorized {
        credential: credential(),
        access: Access {
            token: "fixture-session-token".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            endpoint: API_ROOT.into(),
        },
        revision,
    }
}
#[tokio::test]
async fn commit_adds_only_one_isolated_provider_and_never_returns_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("copilot"));
    let db = Database::memory().unwrap();
    let mut original:Document=serde_json::from_value(json!({"revision":0,"providers":[{"id":"original","name":"Existing","baseUrl":"https://existing.example/v1","protocol":"openai_chat","models":[{"id":"gpt-6"}]}],"policies":{},"tests":{}})).unwrap();
    original.save(&db).unwrap();
    let before = serde_json::to_value(&original.providers[0]).unwrap();
    let ready = ready(&manager, original.revision).await;
    let result = commands::link(&manager, &db, "flow-fixture", ready)
        .await
        .unwrap();
    assert!(!result.to_string().contains("token"));
    let doc = Document::load(&db).unwrap();
    assert_eq!(doc.providers.len(), 2);
    assert_eq!(serde_json::to_value(&doc.providers[0]).unwrap(), before);
    assert_eq!(doc.providers[1].id, PROVIDER_ID);
    assert!(doc.providers[1].key_ref.is_none());
    assert!(doc.providers[1].models.is_empty());
    assert!(!serde_json::to_string(&doc)
        .unwrap()
        .contains("private-oauth"));
    assert!(
        manager
            .status(doc.providers[1].copilot.as_ref())
            .await
            .connected
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&manager.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn stale_or_cancelled_authorizations_do_not_write_credentials_or_provider_data() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("copilot"));
    let db = Database::memory().unwrap();
    let mut doc = Document::default();
    doc.save(&db).unwrap();
    let auth = ready(&manager, doc.revision).await;
    manager.cancel("flow-fixture").await;
    assert!(commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .is_err());
    assert!(!manager.path.exists());
    assert!(Document::load(&db).unwrap().providers.is_empty());
    let auth = ready(&manager, doc.revision).await;
    doc.save(&db).unwrap();
    assert!(commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .is_err());
    assert!(!manager.path.exists());
}
#[tokio::test]
async fn account_switch_is_explicit_and_file_write_failure_leaves_document_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("copilot"));
    let db = Database::memory().unwrap();
    let mut doc = Document::default();
    doc.save(&db).unwrap();
    let auth = ready(&manager, doc.revision).await;
    commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .unwrap();
    let doc = Document::load(&db).unwrap();
    let before = serde_json::to_value(&doc).unwrap();
    let mut auth = ready(&manager, doc.revision).await;
    auth.credential.account_id = "456".into();
    assert!(commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .is_err());
    assert_eq!(
        serde_json::to_value(Document::load(&db).unwrap()).unwrap(),
        before
    );
    let broken = dir.path().join("not-a-directory");
    fs::write(&broken, "fixture").unwrap();
    let manager = Manager::new(broken);
    let auth = ready(&manager, doc.revision).await;
    assert!(commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .is_err());
    assert_eq!(
        serde_json::to_value(Document::load(&db).unwrap()).unwrap(),
        before
    );
}
#[tokio::test]
async fn cancellation_during_oauth_io_cannot_late_commit_a_login() {
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use tokio::sync::Notify;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let e = entered.clone();
    let r = release.clone();
    let app=Router::new().route("/login/device/code",post(||async{Json(json!({"device_code":"private-device","user_code":"TEST-CODE","expires_in":900,"interval":5}))})).route("/login/oauth/access_token",post(move||{let e=e.clone();let r=r.clone();async move{e.notify_one();r.notified().await;Json(json!({"access_token":"fixture-github-token"}))}})).route("/user",get(||async{Json(json!({"id":123,"login":"fixture-user"}))})).route("/copilot_internal/v2/token",get(||async{Json(json!({"token":"fixture-copilot-token","expires_at":chrono::Utc::now().timestamp()+3600,"endpoints":{"api":API_ROOT}}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(Manager::at(dir.path().into(), base.clone(), base));
    let c = manager.start(1).await.unwrap();
    manager.device.lock().await.as_mut().unwrap().next_poll = Instant::now();
    let m = manager.clone();
    let id = c.flow_id.clone();
    let task = tokio::spawn(async move { m.poll(&id).await });
    entered.notified().await;
    manager.cancel(&c.flow_id).await;
    release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(!manager.path.exists());
    assert!(manager.state.lock().await.credential.is_none());
    server.abort();
}
#[tokio::test]
async fn catalog_refresh_commits_initial_and_subsequent_discovery_but_not_failures() {
    use axum::{response::IntoResponse, routing::get, Json, Router};
    let mode = Arc::new(AtomicUsize::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let (m, c) = (mode.clone(), count.clone());
    let app = Router::new().route("/models", get(move || {
        let (m, c) = (m.clone(), c.clone());
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            let phase = m.load(Ordering::SeqCst);
            if phase == 2 { return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            let mut data = vec![json!({"id":"retained","model_picker_enabled":true,"supported_endpoints":["/responses"],"capabilities":{"limits":{"max_context_window_tokens":if phase == 0 {128000} else {256000}}}})];
            if phase == 1 { data.push(json!({"id":"new","model_picker_enabled":true,"supported_endpoints":["/chat/completions"]})); }
            Json(json!({"data":data})).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(Manager::at(dir.path().into(), base.clone(), base.clone()));
    let db = Arc::new(Database::memory().unwrap());
    let auth = ready(&manager, 0).await;
    commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .unwrap();
    manager.state.lock().await.access.as_mut().unwrap().endpoint = base;
    let credential_bytes = fs::read(&manager.path).unwrap();
    let state = crate::manual::ManualState {
        db: db.clone(),
        copilot: manager,
        logs: std::sync::Mutex::new(Arc::new(crate::manual::logs::RequestLogs::default())),
        mutation: Mutex::new(()),
        server: Mutex::new(None),
        plans: Mutex::new(std::collections::HashMap::new()),
    };
    assert_eq!(
        crate::manual::discover_catalog(&state, PROVIDER_ID)
            .await
            .unwrap(),
        1
    );
    let mut doc = Document::load(&db).unwrap();
    let stable = doc.groups()[0].id.clone();
    doc.providers[0].models[0].enabled = false;
    doc.save(&db).unwrap();
    mode.store(1, Ordering::SeqCst);
    assert_eq!(
        crate::manual::discover_catalog(&state, PROVIDER_ID)
            .await
            .unwrap(),
        2
    );
    let doc = Document::load(&db).unwrap();
    assert!(!doc.providers[0].models[0].enabled);
    assert!(doc.providers[0].models[1].enabled);
    assert_eq!(doc.providers[0].models[0].context_window, Some(256000));
    assert_eq!(
        crate::manual::catalog::group_id(&Protocol::OpenaiResponses, &doc.providers[0].models[0]),
        stable
    );
    let before = serde_json::to_value(&doc).unwrap();
    mode.store(2, Ordering::SeqCst);
    assert!(crate::manual::discover_catalog(&state, PROVIDER_ID)
        .await
        .is_err());
    assert_eq!(
        serde_json::to_value(Document::load(&db).unwrap()).unwrap(),
        before
    );
    let held = state.mutation.lock().await;
    assert!(crate::manual::discover_catalog(&state, PROVIDER_ID)
        .await
        .is_err());
    drop(held);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    state
        .set_provider_enabled(PROVIDER_ID, false, doc.revision)
        .await
        .unwrap();
    let disabled = Document::load(&db).unwrap();
    assert!(crate::manual::catalog::public_models(&disabled)["data"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(disabled.providers[0].models, doc.providers[0].models);
    assert!(state
        .set_provider_enabled(PROVIDER_ID, true, doc.revision)
        .await
        .is_err());
    state
        .set_provider_enabled(PROVIDER_ID, true, disabled.revision)
        .await
        .unwrap();
    let restored = Document::load(&db).unwrap();
    assert_eq!(
        serde_json::to_value(&restored.providers).unwrap(),
        serde_json::to_value(&doc.providers).unwrap()
    );
    assert_eq!(fs::read(&state.copilot.path).unwrap(), credential_bytes);
    server.abort();
}

#[tokio::test]
async fn rejected_saved_login_is_reported_without_erasing_credentials_or_catalog() {
    use axum::{routing::get, Router};
    let app = Router::new().route(
        "/copilot_internal/v2/token",
        get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::at(dir.path().into(), base.clone(), base);
    let db = Database::memory().unwrap();
    let auth = ready(&manager, 0).await;
    commands::link(&manager, &db, "flow-fixture", auth)
        .await
        .unwrap();
    manager.state.lock().await.access = None;
    let before = fs::read(&manager.path).unwrap();
    let doc = Document::load(&db).unwrap();
    let source = &doc.providers[0];
    assert!(manager
        .discover(source)
        .await
        .unwrap_err()
        .contains("重新登录"));
    let status = manager.status(source.copilot.as_ref()).await;
    assert!(
        !status.connected,
        "rejected OAuth must not remain presented as connected"
    );
    assert!(status.message.unwrap().contains("重新登录"));
    assert_eq!(fs::read(&manager.path).unwrap(), before);
    assert_eq!(
        serde_json::to_value(Document::load(&db).unwrap()).unwrap(),
        serde_json::to_value(doc).unwrap()
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_refresh_is_single_flight_and_routes_keep_stable_account_identity() {
    use axum::{routing::get, Json, Router};
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app=Router::new().route("/copilot_internal/v2/token",get(move||{let seen=seen.clone();async move{seen.fetch_add(1,Ordering::SeqCst);Json(json!({"token":"fixture-session","expires_at":chrono::Utc::now().timestamp()+3600,"endpoints":{"api":"https://api.individual.githubcopilot.com"}}))}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::at(dir.path().into(), base.clone(), base);
    let c = credential();
    let binding = Binding {
        account_id: c.account_id.clone(),
        grant_id: c.grant_id.clone(),
    };
    manager.state.lock().await.credential = Some(c);
    let (a, b) = tokio::join!(manager.resolve(&binding), manager.resolve(&binding));
    assert_eq!(a.unwrap().0, "fixture-session");
    assert_eq!(b.unwrap().0, "fixture-session");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let model = Model {
        id: "copilot/exact-model".into(),
        protocol_override: Some(Protocol::OpenaiResponses),
        enabled: true,
        ..Model::default()
    };
    let s = Source {
        id: PROVIDER_ID.into(),
        name: "Copilot".into(),
        base_url: API_ROOT.into(),
        key_env: String::new(),
        key_ref: None,
        copilot: Some(binding),
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models: vec![model.clone()],
    };
    for app in ["claude", "codex"] {
        let runtime = manager.provider(&s, &model, app).await.unwrap();
        assert_eq!(
            runtime.settings_config["manual_upstream_model"],
            "exact-model"
        );
        assert_eq!(
            runtime.settings_config["manual_credential_version"],
            crate::manual::catalog::deployment_version(&s, &model)
        );
        assert_eq!(runtime.settings_config["api_format"], "openai_responses");
    }
    assert!(!serde_json::to_string(&s)
        .unwrap()
        .contains("fixture-session"));
    server.abort();
}
#[tokio::test]
async fn bare_model_names_reach_the_real_router_without_login_or_network_calls() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Arc::new(Manager::new(directory.path().join("copilot")));
    let credential = credential();
    let binding = Binding {
        account_id: credential.account_id.clone(),
        grant_id: credential.grant_id.clone(),
    };
    {
        let mut state = manager.state.lock().await;
        state.credential = Some(credential);
        state.access = Some(Access {
            token: "fixture-session-only".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            endpoint: API_ROOT.into(),
        });
    }
    let model = Model {
        id: "copilot/gpt-5.4".into(),
        protocol_override: Some(Protocol::OpenaiResponses),
        enabled: true,
        ..Model::default()
    };
    let source = Source {
        id: PROVIDER_ID.into(),
        name: "Copilot".into(),
        base_url: API_ROOT.into(),
        key_env: String::new(),
        key_ref: None,
        copilot: Some(binding),
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models: vec![model],
    };
    let mut doc = Document {
        providers: vec![source],
        ..Document::default()
    };
    doc.validate().unwrap();
    let before = serde_json::to_value(&doc).unwrap();
    let groups = doc.groups();
    let stable = groups[0].id.clone();
    let catalog = crate::manual::catalog::public_models(&doc);
    assert_eq!(catalog["data"][0]["id"], "gpt-5.4");
    assert_eq!(catalog["data"][0]["route_id"], stable);
    assert!(catalog["data"][0]["aliases"]
        .as_array()
        .unwrap()
        .contains(&json!(stable)));
    let db = Arc::new(Database::memory().unwrap());
    let router = crate::proxy::provider_router::ProviderRouter::new(db);
    router.manual.attach_copilot(manager.clone());
    for id in ["gpt-5.4", "copilot/gpt-5.4", stable.as_str()] {
        let candidates = router
            .manual
            .select(
                &router,
                &doc,
                "codex",
                &json!({"model":id,"input":"fixture"}),
                None,
            )
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, PROVIDER_ID);
        assert_eq!(
            candidates[0].settings_config
                [crate::proxy::providers::native_responses_identity::ENABLED],
            true
        );
        assert_eq!(
            candidates[0].settings_config["manual_upstream_model"],
            "gpt-5.4"
        );
    }
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    assert!(!manager.path.exists());
    doc.blocked_models.insert("copilot/gpt-5.4".into());
    assert!(crate::manual::catalog::public_models(&doc)["data"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(router
        .manual
        .select(
            &router,
            &doc,
            "codex",
            &json!({"model":"gpt-5.4","input":"fixture"}),
            None
        )
        .await
        .is_err());
}

#[test]
fn existing_api_names_are_not_hijacked_even_when_blocked_and_config_resolution_stays_explicit() {
    let normal = Model {
        id: "gpt-5.4".into(),
        protocol_override: Some(Protocol::OpenaiResponses),
        enabled: true,
        ..Model::default()
    };
    let mut cp = normal.clone();
    cp.id = "copilot/gpt-5.4".into();
    let api = Source {
        id: "api".into(),
        name: "API".into(),
        base_url: "https://example.com".into(),
        key_env: String::new(),
        key_ref: None,
        copilot: None,
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models: vec![normal],
    };
    let copilot = Source {
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
        models: vec![cp],
    };
    let mut doc = Document {
        providers: vec![api, copilot],
        ..Default::default()
    };
    let groups = doc.groups();
    let normal = crate::manual::catalog::resolve_public_group(&groups, "gpt-5.4").unwrap();
    assert_eq!(normal.provider_ids, vec!["api"]);
    let cp = groups
        .iter()
        .find(|g| g.model.id.starts_with("copilot/"))
        .unwrap();
    assert_eq!(cp.public_id, "copilot/gpt-5.4");
    doc.blocked_models.insert("gpt-5.4".into());
    let groups = doc.groups();
    assert!(
        crate::manual::catalog::resolve_public_group(&groups, "gpt-5.4")
            .unwrap()
            .blocked
    );
    doc.providers.remove(0);
    let groups = doc.groups();
    assert!(crate::manual::catalog::resolve_group(&groups, "gpt-5.4").is_none());
    assert!(crate::manual::catalog::resolve_public_group(&groups, "gpt-5.4").is_some());
}

#[test]
fn native_messages_are_preserved_and_ordinary_sources_cannot_claim_the_copilot_namespace() {
    let models=parse_models(&json!({"data":[{"id":"native-claude","model_picker_enabled":true,"supported_endpoints":["/v1/messages"],"capabilities":{"type":"chat"}}]})).unwrap();
    assert_eq!(models[0].protocol_override, Some(Protocol::Anthropic));
    assert_eq!(models[0].context_window, None);
    assert_eq!(models[0].tools, None);
    assert_eq!(
        inference_url(API_ROOT, "/v1/messages").unwrap(),
        format!("{API_ROOT}/v1/messages")
    );
    let s = Source {
        id: "ordinary".into(),
        name: "Existing".into(),
        base_url: "https://example.com".into(),
        key_env: String::new(),
        key_ref: None,
        copilot: None,
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models,
    };
    assert!(Document {
        providers: vec![s],
        ..Document::default()
    }
    .validate()
    .is_err());
}
