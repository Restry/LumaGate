use super::*;
use crate::manual::{catalog::Document, keys::FileKeyStore, ManualState};
use access_keys::{Operation, Registry, REGISTRY_KEY};
use axum::{body::Body, routing::get, Json, Router};
use http_body_util::BodyExt;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tower::Service;
const A: &str = "fixture-network-key-a-0123456789abcdef0123456789";
const B: &str = "fixture-network-key-b-0123456789abcdef0123456789";

struct Fixture {
    db: Arc<Database>,
    directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        Self {
            db: Arc::new(Database::memory().unwrap()),
            directory: tempfile::tempdir().unwrap(),
        }
    }
    fn change(&self, operation: Operation) -> access_keys::KeyList {
        let current = Registry::load(&self.db).unwrap();
        let revision = current.view().revision;
        current
            .change(
                revision,
                operation,
                &FileKeyStore::at_directory(self.directory.path().join("access-keys")),
                |value| {
                    self.db
                        .set_setting(REGISTRY_KEY, value)
                        .map_err(|e| e.to_string())
                },
            )
            .unwrap()
            .keys
    }
    fn add(&self, name: &str, value: &str) -> String {
        self.change(Operation::Add {
            name: name.into(),
            value: value.into(),
        })
        .items
        .into_iter()
        .find(|key| key.name == name)
        .unwrap()
        .id
    }
    fn policy(&self, lan: bool) -> Policy {
        Policy::new(
            Settings {
                listen_address: if lan { "0.0.0.0" } else { "127.0.0.1" }.into(),
                ..Default::default()
            },
            self.db.clone(),
        )
    }
}
fn update(address: &str, port: u16, revision: u64) -> Update {
    Update {
        revision,
        listen_address: address.into(),
        listen_port: port,
    }
}
async fn request(
    policy: Policy,
    logs: Arc<logs::RequestLogs>,
    peer: Option<&str>,
    host: &str,
    headers: &[(&str, &str)],
) -> StatusCode {
    let mut router = Router::new()
        .route(
            "/v1/models",
            get(|request: Request| async move {
                // Echo only the supplied credential to prove log redaction, not response rewriting.
                let token = request
                    .headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.split_once(' '))
                    .map(|(_, v)| v)
                    .or_else(|| {
                        request
                            .headers()
                            .get("x-api-key")
                            .and_then(|h| h.to_str().ok())
                    })
                    .unwrap_or("");
                let response = json!({"data":[], "echo": token});
                tokio::task::yield_now().await;
                Json(response)
            }),
        )
        .layer(axum::middleware::from_fn_with_state(policy, guard))
        .layer(axum::middleware::from_fn_with_state(logs, logs::capture));
    let mut builder = Request::builder().uri("/v1/models").header("host", host);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    }
    let response = router.call(request).await.unwrap();
    let status = response.status();
    response.into_body().collect().await.unwrap();
    status
}
async fn call(
    f: &Fixture,
    lan: bool,
    peer: Option<&str>,
    host: &str,
    headers: &[(&str, &str)],
) -> StatusCode {
    request(
        f.policy(lan),
        Arc::new(logs::RequestLogs::default()),
        peer,
        host,
        headers,
    )
    .await
}

#[test]
fn default_and_validated_listener_settings_are_independent_of_named_keys() {
    let f = Fixture::new();
    assert_eq!(
        Settings::load(&f.db).unwrap().base_url(),
        "http://127.0.0.1:15722"
    );
    for value in [
        update("0.0.0.0", 15722, 0),
        update("127.0.0.1", 0, 0),
        update("192.168.1.4", 15722, 0),
        update("example.com", 15722, 0),
    ] {
        assert!(Settings::save_update(&f.db, value).is_err());
        assert!(f.db.get_setting(SETTINGS_KEY).unwrap().is_none());
    }
    f.add("A", A);
    let saved = Settings::save_update(&f.db, update("0.0.0.0", 18080, 0)).unwrap();
    assert!(saved.allow_lan());
    assert_eq!(saved.base_url(), "http://127.0.0.1:18080");
    assert!(Settings::save_update(&f.db, update("127.0.0.1", 18080, 0)).is_err());
    let snapshot =
        serde_json::to_string(&saved.snapshot(None, &Registry::load(&f.db).unwrap())).unwrap();
    for secret_field in [A, "secretRef", "accessKeyTag", "\"tag\""] {
        assert!(!snapshot.contains(secret_field));
    }
    f.db.set_setting(SETTINGS_KEY, "invalid").unwrap();
    assert!(Settings::load(&f.db).is_err());
}

#[tokio::test]
async fn loopback_compatibility_host_origin_and_real_peer_restrictions_remain() {
    let f = Fixture::new();
    f.add("A", A);
    for lan in [false, true] {
        assert_eq!(
            call(&f, lan, Some("127.0.0.1:4444"), "localhost:15722", &[]).await,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &f,
                lan,
                Some("127.0.0.1:4444"),
                "127.0.0.1:15722",
                &[("authorization", "Bearer cc-switch-manual-local")]
            )
            .await,
            StatusCode::OK
        );
        for origin in ["https://example.com", "null", "http://127.0.0.1:15722"] {
            assert_eq!(
                call(
                    &f,
                    lan,
                    Some("127.0.0.1:4444"),
                    "127.0.0.1:15722",
                    &[("origin", origin)]
                )
                .await,
                StatusCode::FORBIDDEN
            );
        }
        for host in [
            "evil.example",
            "localhost.evil.example",
            "evil@127.0.0.1:15722",
        ] {
            assert_eq!(
                call(&f, lan, Some("127.0.0.1:4444"), host, &[]).await,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            call(&f, lan, None, "127.0.0.1", &[]).await,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            &f,
            false,
            Some("192.168.1.4:4444"),
            "localhost",
            &[("x-api-key", A)]
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn private_peers_need_one_unambiguous_enabled_identity_public_and_spoofed_peers_are_rejected()
{
    let f = Fixture::new();
    f.add("A", A);
    f.add("B", B);
    let bearer = format!("Bearer {A}");
    for peer in [
        "192.168.2.10:4444",
        "10.2.3.4:4444",
        "172.16.0.2:4444",
        "169.254.1.2:4444",
    ] {
        assert_eq!(
            call(&f, true, Some(peer), "192.168.1.4", &[]).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &f,
                true,
                Some(peer),
                "192.168.1.4",
                &[("authorization", &bearer)]
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            call(&f, true, Some(peer), "192.168.1.4", &[("x-api-key", B)]).await,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &f,
                true,
                Some(peer),
                "localhost",
                &[
                    ("x-forwarded-for", "127.0.0.1"),
                    ("authorization", "Bearer cc-switch-manual-local")
                ]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &f,
                true,
                Some(peer),
                "localhost",
                &[("x-api-key", A), ("x-api-key", A)]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &f,
                true,
                Some(peer),
                "localhost",
                &[("authorization", &bearer), ("x-api-key", B)]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }
    for peer in ["8.8.8.8:4444", "172.32.0.1:4444", "[2001:db8::1]:4444"] {
        assert_eq!(
            call(&f, true, Some(peer), "localhost", &[("x-api-key", A)]).await,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn logs_snapshot_key_identity_without_secrets_and_keep_local_and_rejected_distinct() {
    let f = Fixture::new();
    let a = f.add("Alice", A);
    let b = f.add("Build", B);
    let logs = Arc::new(logs::RequestLogs::default());
    request(
        f.policy(true),
        logs.clone(),
        Some("192.168.1.4:44"),
        "localhost",
        &[("authorization", &format!("bEaReR {A}"))],
    )
    .await;
    request(
        f.policy(true),
        logs.clone(),
        Some("127.0.0.1:44"),
        "localhost",
        &[("x-api-key", B)],
    )
    .await;
    request(
        f.policy(true),
        logs.clone(),
        Some("127.0.0.1:44"),
        "localhost",
        &[],
    )
    .await;
    request(
        f.policy(true),
        logs.clone(),
        Some("192.168.1.4:44"),
        "localhost",
        &[("x-api-key", "invalid")],
    )
    .await;
    f.change(Operation::Rename {
        key_id: a.clone(),
        name: "Alice renamed".into(),
    });
    let rows = serde_json::to_value(logs.snapshot()).unwrap();
    assert_eq!(rows[0]["caller"]["kind"], "rejected");
    assert_eq!(rows[1]["caller"]["kind"], "local");
    assert_eq!(rows[2]["caller"]["keyId"], b);
    assert_eq!(rows[2]["caller"]["name"], "Build");
    assert_eq!(rows[3]["caller"]["keyId"], a);
    assert_eq!(rows[3]["caller"]["name"], "Alice");
    let text = rows.to_string();
    assert!(!text.contains(A));
    assert!(!text.contains(B));
    f.db.set_setting(REGISTRY_KEY, "broken").unwrap();
    assert_eq!(
        call(
            &f,
            true,
            Some("192.168.1.4:44"),
            "localhost",
            &[("x-api-key", A)]
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn concurrent_requests_keep_key_identities_separate() {
    let f = Fixture::new();
    f.add("Alice", A);
    f.add("Build", B);
    let logs = Arc::new(logs::RequestLogs::default());
    let futures = (0..16).map(|index| {
        let policy = f.policy(true);
        let logs = logs.clone();
        async move {
            request(
                policy,
                logs,
                Some("192.168.1.4:4444"),
                "localhost",
                &[("x-api-key", if index % 2 == 0 { A } else { B })],
            )
            .await
        }
    });
    for status in futures::future::join_all(futures).await {
        assert_eq!(status, StatusCode::OK);
    }
    let rows = serde_json::to_value(logs.snapshot()).unwrap();
    let entries = rows.as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|row| row["caller"]["name"] == "Alice")
            .count(),
        8
    );
    assert_eq!(
        entries
            .iter()
            .filter(|row| row["caller"]["name"] == "Build")
            .count(),
        8
    );
    assert!(!rows.to_string().contains(A));
    assert!(!rows.to_string().contains(B));
}

#[tokio::test]
async fn real_private_socket_applies_key_changes_without_restart_and_stop_revokes_keepalive() {
    let ip = if_addrs::get_if_addrs()
        .unwrap()
        .into_iter()
        .find_map(|interface| match interface.ip() {
            IpAddr::V4(ip) if private_ipv4(ip) => Some(ip),
            _ => None,
        });
    let Some(ip) = ip else {
        eprintln!("NOT VERIFIED: no private IPv4 for socket test");
        return;
    };
    let f = Fixture::new();
    Document::default().save(&f.db).unwrap();
    let a = f.add("A", A);
    Settings::save_update(&f.db, update("0.0.0.0", 15722, 0)).unwrap();
    let server = crate::proxy::server::ProxyServer::new(
        crate::proxy::types::ProxyConfig {
            listen_address: "0.0.0.0".into(),
            listen_port: 0,
            ..Default::default()
        },
        f.db.clone(),
        None,
    );
    let info = server.start().await.unwrap();
    let url = format!("http://{ip}:{}/v1/models", info.port);
    let client = reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(1)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    async fn drain(request: reqwest::RequestBuilder) -> StatusCode {
        let response = request.send().await.unwrap();
        let status = response.status();
        response.bytes().await.unwrap(); // Return the established connection to the keep-alive pool.
        status
    }
    assert_eq!(drain(client.get(&url)).await, StatusCode::UNAUTHORIZED);
    assert_eq!(drain(client.get(&url).bearer_auth(A)).await, StatusCode::OK);
    let b = f.add("B", B);
    assert_eq!(drain(client.get(&url).bearer_auth(B)).await, StatusCode::OK);
    f.change(Operation::SetEnabled {
        key_id: a,
        enabled: false,
    });
    assert_eq!(
        drain(client.get(&url).bearer_auth(A)).await,
        StatusCode::UNAUTHORIZED
    );
    f.change(Operation::Remove { key_id: b });
    assert_eq!(
        drain(client.get(&url).bearer_auth(B)).await,
        StatusCode::UNAUTHORIZED
    );
    server.stop().await.unwrap();
    assert!(client.get(&url).send().await.is_err());
    eprintln!("VERIFIED private socket: add=200 immediately, disable/delete=401 immediately, stop closes keep-alive; isolated empty catalog");
}

#[tokio::test]
async fn listener_changes_remain_manual_and_occupied_ports_do_not_report_running() {
    let f = Fixture::new();
    Document::default().save(&f.db).unwrap();
    let state = ManualState {
        db: f.db.clone(),
        copilot: Arc::new(crate::manual::copilot::Manager::new(
            f.directory.path().join("copilot"),
        )),
        logs: std::sync::Mutex::new(Arc::new(
            crate::manual::logs::RequestLogs::open(&f.directory.path().join("logs")).unwrap(),
        )),
        mutation: Mutex::new(()),
        server: Mutex::new(None),
        plans: Mutex::new(HashMap::new()),
    };
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    state
        .save_network(update("127.0.0.1", port, 0))
        .await
        .unwrap();
    assert!(state.set_gateway(true).await.is_err());
    assert!(state.server.lock().await.is_none());
    drop(occupied);
    state.set_gateway(true).await.unwrap();
    let response = reqwest::get(format!("http://127.0.0.1:{port}/v1/models"))
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.bytes().await.unwrap();
    let next_port = if port == 65535 { port - 1 } else { port + 1 };
    state
        .save_network(update("127.0.0.1", next_port, 1))
        .await
        .unwrap();
    {
        let server = state.server.lock().await;
        let active = server.as_ref().unwrap();
        let saved = Settings::load(&state.db).unwrap();
        assert!(saved.restart_required(&active.settings));
        assert!(saved.sync_base(Some(&active.settings)).is_err());
    }
    state.set_gateway(false).await.unwrap();
    assert!(state.server.lock().await.is_none());
    let rows = state.current_logs().persisted_snapshot().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["endpoint"], "/v1/models");
    drop(state);
    let reopened =
        crate::manual::logs::RequestLogs::open(&f.directory.path().join("logs")).unwrap();
    assert_eq!(reopened.persisted_snapshot().unwrap(), rows);
}

#[tokio::test]
async fn unavailable_log_store_keeps_state_inspectable_but_does_not_start_a_silent_gateway() {
    let f = Fixture::new();
    Document::default().save(&f.db).unwrap();
    let state = ManualState {
        db: f.db.clone(),
        copilot: Arc::new(crate::manual::copilot::Manager::new(
            f.directory.path().join("copilot"),
        )),
        logs: std::sync::Mutex::new(Arc::new(crate::manual::logs::RequestLogs::unavailable(
            "fixture corrupt database".into(),
        ))),
        mutation: Mutex::new(()),
        server: Mutex::new(None),
        plans: Mutex::new(HashMap::new()),
    };
    assert_eq!(state.current_logs().storage_status()["ready"], false);
    assert!(Document::load(&state.db).is_ok());
    assert!(state
        .set_gateway(true)
        .await
        .unwrap_err()
        .contains("日志库不可用"));
    assert!(state.server.lock().await.is_none());
}

#[tokio::test]
async fn network_changes_invalidate_sync_plans_but_key_changes_do_not_modify_agent_files() {
    let f = Fixture::new();
    let mut doc: Document = serde_json::from_value(json!({"revision":0,"providers":[{"id":"fixture","name":"Fixture","baseUrl":"https://example.invalid/v1","keyEnv":"","protocol":"openai_chat","models":[{"id":"demo"}]}],"policies":{},"tests":{}})).unwrap();
    doc.save(&f.db).unwrap();
    let state = ManualState {
        db: f.db.clone(),
        copilot: Arc::new(crate::manual::copilot::Manager::new(
            f.directory.path().join("copilot"),
        )),
        logs: std::sync::Mutex::new(Arc::new(crate::manual::logs::RequestLogs::default())),
        mutation: Mutex::new(()),
        server: Mutex::new(None),
        plans: Mutex::new(HashMap::new()),
    };
    state
        .save_network(update("127.0.0.1", 18080, 0))
        .await
        .unwrap();
    let saved = Settings::load(&state.db).unwrap();
    let home = f.directory.path().canonicalize().unwrap();
    let plan = crate::manual::sync::Plan::build(
        &home,
        &doc,
        crate::manual::sync::Agent::Pi,
        &saved.sync_base(None).unwrap(),
    )
    .unwrap();
    assert!(serde_json::to_string(&plan.preview())
        .unwrap()
        .contains("http://127.0.0.1:18080/v1"));
    let stale = crate::manual::PendingSync {
        plan,
        network_revision: saved.revision,
    };
    f.add("A", A);
    assert_eq!(Settings::load(&state.db).unwrap().revision, saved.revision);
    state
        .save_network(update("127.0.0.1", 18081, 1))
        .await
        .unwrap();
    assert!(stale.apply(&state.db).is_err());
    assert!(!home.join(".pi/agent/models.json").exists());
    assert_eq!(Document::load(&state.db).unwrap().revision, doc.revision);
}

#[test]
fn wildcard_bind_still_requires_explicit_permission() {
    assert!(Settings::default()
        .validate_listener("0.0.0.0".parse().unwrap())
        .is_err());
    let lan = Settings {
        listen_address: "0.0.0.0".into(),
        ..Default::default()
    };
    assert!(lan.validate_listener("0.0.0.0".parse().unwrap()).is_ok());
    assert!(lan.validate_listener("8.8.8.8".parse().unwrap()).is_err());
}
