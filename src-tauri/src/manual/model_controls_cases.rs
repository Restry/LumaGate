use super::*;

#[tokio::test]
async fn explicit_provider_tests_are_isolated_and_persisted() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    let mut doc = Document {
        providers: vec![source("a", &a), source("b", &b)],
        ..Document::default()
    };
    let id = doc.groups()[0].id.clone();
    let result = controls::test_deployment(&doc, &id, "a").await.unwrap();
    assert!(result.success);
    assert_eq!(result.provider_id.as_deref(), Some("a"));
    controls::record_test(&mut doc, &id, "a", result);
    second.status.store(503, Ordering::SeqCst);
    let failed = controls::test_deployment(&doc, &id, "b").await.unwrap();
    assert!(!failed.success);
    assert_eq!(failed.provider_id.as_deref(), Some("b"));
    controls::record_test(&mut doc, &id, "b", failed);
    assert_eq!(first.calls.lock().await.len(), 1, "B 测试失败不得借用 A");
    assert_eq!(second.calls.lock().await.len(), 1);
    let db = Database::memory().unwrap();
    doc.save(&db).unwrap();
    let loaded = Document::load(&db).unwrap();
    assert!(loaded.tests[&catalog::test_key(&id, "a")].success);
    assert!(!loaded.tests[&catalog::test_key(&id, "b")].success);
    let public = catalog::public_models(&loaded);
    assert_eq!(
        public["data"][0]["deployment_tests"][0]["result"]["providerId"],
        "a"
    );
    assert_eq!(
        public["data"][0]["deployment_tests"][1]["result"]["success"],
        false
    );
    assert!(controls::test_deployment(&doc, &id, "unrelated")
        .await
        .is_err());
    doc.providers[1].models[0].enabled = false;
    assert!(controls::test_deployment(&doc, &id, "b").await.is_err());
    controls::set_blocked(&mut doc, "model-a", true).unwrap();
    assert!(controls::test_deployment(&doc, &id, "a").await.is_err());
    controls::set_blocked(&mut doc, "model-a", false).unwrap();
    assert!(doc.providers[0].models[0].enabled);
    assert!(!doc.providers[1].models[0].enabled);
    assert_eq!(first.calls.lock().await.len(), 1);
    assert_eq!(second.calls.lock().await.len(), 1);
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn blocked_fallback_is_skipped_without_breaking_primary() {
    let (a, first, task_a) = mock_upstream().await;
    let (b, second, task_b) = mock_upstream().await;
    let mut fallback = source("b", &b);
    fallback.models[0].id = "model-b".into();
    let mut doc = Document {
        providers: vec![source("a", &a), fallback],
        ..Document::default()
    };
    let groups = doc.groups();
    let primary_id = groups
        .iter()
        .find(|g| g.model.id == "model-a")
        .unwrap()
        .id
        .clone();
    let fallback_id = groups
        .iter()
        .find(|g| g.model.id == "model-b")
        .unwrap()
        .id
        .clone();
    doc.policies.insert(
        primary_id.clone(),
        catalog::Policy {
            fallbacks: vec![fallback_id.clone()],
            ..catalog::Policy::default()
        },
    );
    controls::set_blocked(&mut doc, "model-b", true).unwrap();
    let db = Database::memory().unwrap();
    doc.save(&db).unwrap();
    doc = Document::load(&db).unwrap();
    assert!(doc.blocked_models.contains("model-b"));
    assert_eq!(
        catalog::public_models(&doc)["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut stale_form = doc.clone();
    stale_form.blocked_models.clear();
    keys::save_with_keys(
        &doc,
        stale_form,
        None,
        &keys::FileKeyStore::default(),
        |saved| {
            assert!(saved.blocked_models.contains("model-b"));
            Ok(())
        },
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    // macOS 的 /var 是系统别名，夹具用真实路径，不能放宽生产符号链接保护。
    let home_path = home.path().canonicalize().unwrap();
    for agent in [sync::Agent::Pi, sync::Agent::Claude, sync::Agent::Codex] {
        let preview = sync::Plan::build(&home_path, &doc, agent, BASE)
            .unwrap()
            .preview();
        let text = serde_json::to_string(&preview).unwrap();
        assert!(text.contains("model-a"));
        assert!(!text.contains("model-b"));
    }
    assert!(std::fs::read_dir(&home_path).unwrap().next().is_none());
    let (server, base, _) = gateway_document(doc.clone()).await;
    let client = reqwest::Client::new();
    let body = |id: &str| json!({"model":id,"messages":[{"role":"user","content":"Hello"}],"stream":false});
    assert_eq!(
        client
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(LOCAL_TOKEN)
            .json(&body(&primary_id))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(!client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&body(&fallback_id))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    first.status.store(503, Ordering::SeqCst);
    assert!(!client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&body(&primary_id))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    assert!(second.calls.lock().await.is_empty());
    server.stop().await.unwrap();
    controls::set_blocked(&mut doc, "model-b", false).unwrap();
    let (server, base, _) = gateway_document(doc).await;
    assert_eq!(
        client
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(LOCAL_TOKEN)
            .json(&body(&primary_id))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(second.calls.lock().await.len(), 1);
    server.stop().await.unwrap();
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn image_requests_only_use_declared_capable_sources() {
    let (a, unknown, task_a) = mock_upstream().await;
    let (b, vision, task_b) = mock_upstream().await;
    let mut capable = source("vision", &b);
    capable.models[0].input_modalities = Some(vec!["text".into(), "image".into()]);
    capable.models[0].output_modalities = Some(vec!["text".into()]);
    capable.models[0].image = true;
    let (server, base, doc) = gateway(vec![source("unknown", &a), capable]).await;
    let group = &doc.groups()[0];
    assert!(group.model.image);
    assert!(group
        .model
        .input_modalities
        .as_ref()
        .unwrap()
        .contains(&"image".into()));
    let pi = metadata::pi_model(&group.model, &group.id);
    assert!(pi["input"].as_array().unwrap().contains(&json!("image")));
    let mut codex = json!({});
    metadata::enrich_codex_entry(&mut codex, &group.model);
    assert!(codex["input_modalities"]
        .as_array()
        .unwrap()
        .contains(&json!("image")));
    let client = reqwest::Client::new();
    let text =
        json!({"model":group.id,"messages":[{"role":"user","content":"Hello"}],"stream":false});
    for _ in 0..2 {
        assert_eq!(
            client
                .post(format!("{base}/v1/chat/completions"))
                .bearer_auth(LOCAL_TOKEN)
                .json(&text)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(unknown.calls.lock().await.len(), 1);
    assert_eq!(vision.calls.lock().await.len(), 1);
    let image = json!({"model":group.id,"messages":[{"role":"user","content":[{"type":"text","text":"Describe"},{"type":"image_url","image_url":{"url":"https://example.invalid/fixture.png"}}]}],"stream":false});
    assert_eq!(
        client
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(LOCAL_TOKEN)
            .json(&image)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(unknown.calls.lock().await.len(), 1);
    assert_eq!(vision.calls.lock().await.len(), 2);
    vision.status.store(503, Ordering::SeqCst);
    assert!(!client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&image)
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    assert_eq!(
        unknown.calls.lock().await.len(),
        1,
        "不能为了成功把图片转给能力未知的来源"
    );
    server.stop().await.unwrap();
    let router =
        crate::proxy::provider_router::ProviderRouter::new(Arc::new(Database::memory().unwrap()));
    let routes = routing::ManualRoutes::default();
    let mut no_failover = doc.clone();
    no_failover.policies.insert(
        group.id.clone(),
        catalog::Policy {
            failover: false,
            ..catalog::Policy::default()
        },
    );
    assert_eq!(
        routes
            .select(&router, &no_failover, "codex", &image, None)
            .await
            .unwrap()[0]
            .id,
        "vision"
    );
    routes
        .bind(
            "codex",
            "session",
            "unknown",
            &catalog::deployment_version(&doc.providers[0], &doc.providers[0].models[0]),
        )
        .await;
    let mut opaque = image.clone();
    opaque["previous_response_id"] = json!("fixture-response");
    assert!(routes
        .select(&router, &doc, "codex", &opaque, Some("session"))
        .await
        .is_err());
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn image_rejection_is_not_retried_without_the_image() {
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let seen = calls.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
        let calls = seen.clone();
        async move {
            calls.lock().await.push(body.clone());
            if body.to_string().contains("image_url") {
                (StatusCode::BAD_REQUEST, Json(json!({"error":{"message":"image input is not supported"}}))).into_response()
            } else {
                Json(json!({"id":"fixture","object":"chat.completion","model":"model-a","choices":[{"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]})).into_response()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut provider = source("vision", &format!("http://{address}/v1"));
    provider.models[0].input_modalities = Some(vec!["text".into(), "image".into()]);
    let (server, base, doc) = gateway(vec![provider]).await;
    let response = reqwest::Client::new().post(format!("{base}/v1/chat/completions"))
        .bearer_auth(LOCAL_TOKEN)
        .json(&json!({"model":doc.groups()[0].id,"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png"}}]}],"stream":false}))
        .send().await.unwrap();
    assert!(!response.status().is_success());
    assert_eq!(
        calls.lock().await.len(),
        1,
        "不得去图后再次请求，伪造图像处理成功"
    );
    server.stop().await.unwrap();
    task.abort();
}

#[test]
fn modality_schema_aliases_match_pi_and_codex_formats() {
    for row in [
        json!({"id":"m","input":["text","image"],"output":["text"]}),
        json!({"id":"m","input_modalities":["text","image"],"output_modalities":["text"]}),
        json!({"id":"m","inputModalities":["text","image"],"outputModalities":["text"]}),
        json!({"id":"m","capabilities":{"input":["text","image"],"output":["text"]}}),
    ] {
        let model = metadata::parse_model(&row, true).unwrap();
        assert!(model.image);
        assert_eq!(model.output_modalities, Some(vec!["text".into()]));
        assert!(model.input_modalities.unwrap().contains(&"image".into()));
    }
    let model = metadata::parse_model(&json!({"id":"gpt-6-unannounced"}), true).unwrap();
    assert!(model.input_modalities.is_none());
    let mut entry = json!({"input_modalities":["text","image"]});
    metadata::enrich_codex_entry(&mut entry, &model);
    assert_eq!(
        entry["input_modalities"],
        json!(["text"]),
        "未知来源不能继承模板的图片能力"
    );
    assert_eq!(
        metadata::pi_model(&model, "unknown")["input"],
        json!(["text"])
    );
    let known = metadata::parse_model(
        &json!({"id":"known","input":["text","image"],"output":["text"]}),
        true,
    )
    .unwrap();
    for request in [
        json!({"input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.invalid/image.png"}]}]}),
        json!({"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","data":"fixture"}}]}]}),
    ] {
        assert!(modalities::supports_request(&known, &request));
        assert!(!modalities::supports_request(&model, &request));
    }
    let body = json!({"messages":[{"role":"user","content":"Hello"}],"tools":[{"type":"function","function":{"parameters":{"type":"object","properties":{"type":{"const":"image"}}}}}]});
    assert!(
        modalities::supports_request(&model, &body),
        "工具 schema 不应误判成图片输入"
    );
}
