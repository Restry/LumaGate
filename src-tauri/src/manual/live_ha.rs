//! 仅由显式授权触发的真实连通性夹具；普通测试不联网，也不改真实配置。
use super::*;
use axum::{extract::OriginalUri, http::HeaderMap, Router};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const TARGETS: [&str; 3] = ["gpt-6-astra", "FW-Kimi-K3", "Kimi-K3"];
const MAX_CALLS: usize = 18;

struct Relay {
    source: Source,
    fault: AtomicBool,
    budget: Arc<AtomicUsize>,
    max_calls: usize,
    events: Arc<Mutex<Vec<Value>>>,
}

fn output_text(value: &Value) -> String {
    if let Some(text) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        return text.to_string();
    }
    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

async fn relay(
    HttpState(state): HttpState<Arc<Relay>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let id = body.get("model").and_then(Value::as_str).unwrap_or("");
    let Some(model) = state
        .source
        .models
        .iter()
        .find(|m| m.id == id && TARGETS.contains(&id))
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let protocol = endpoints::protocol(model);
    let url = catalog::endpoint(&state.source, protocol.endpoint());
    let expected = url::Url::parse(&url).unwrap();
    let limit = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64);
    let prompt_ok = body.get("input").and_then(Value::as_str) == Some("Reply with OK.")
        || body.pointer("/messages/0/content").and_then(Value::as_str) == Some("Reply with OK.");
    if uri.path() != expected.path()
        || body["stream"] != false
        || !prompt_ok
        || !limit.is_some_and(|limit| (1..=64).contains(&limit))
        || body.get("tools").is_some()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if state.fault.load(Ordering::SeqCst) {
        state
            .events
            .lock()
            .await
            .push(json!({"provider":state.source.name,"model":id,"simulatedFailure":true}));
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"message":"isolated fault fixture"}})),
        )
            .into_response();
    }
    if state
        .budget
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            (count < state.max_calls).then_some(count + 1)
        })
        .is_err()
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":{"message":"live test budget exhausted"}})),
        )
            .into_response();
    }
    let started = Instant::now();
    let mut effective = state.source.clone();
    effective.protocol = protocol;
    let result = async {
        // 核对实际转发的凭据，但绝不把它放进事件或错误文本。
        let key = catalog::credentials(&effective)?;
        let expected_auth = format!("Bearer {key}");
        if !key.is_empty()
            && headers.get("authorization").and_then(|v| v.to_str().ok())
                != Some(expected_auth.as_str())
        {
            return Err("转发鉴权与目标 Provider 不一致".to_string());
        }
        let client = client()?;
        let response = request(
            &client,
            &effective,
            reqwest::Method::POST,
            effective.protocol.endpoint(),
        )
        .await?
        .json(&body)
        .send()
        .await
        .map_err(|_| "网络或 TLS 请求失败".to_string())?;
        let status = response.status();
        let value = bounded_json(response)
            .await
            .unwrap_or_else(|error| json!({"error":{"message":error}}));
        Ok::<_, String>((status, value))
    }
    .await;
    let (status, value) = result.unwrap_or_else(|_| {
        (
            StatusCode::BAD_GATEWAY,
            json!({"error":{"message":"live relay failed"}}),
        )
    });
    let text = output_text(&value);
    let success = status.is_success()
        && !text.trim().is_empty()
        && !value.get("error").is_some_and(|e| !e.is_null())
        && !matches!(
            value.get("status").and_then(Value::as_str),
            Some("failed" | "incomplete" | "cancelled")
        );
    state.events.lock().await.push(json!({
        "provider":state.source.name,"model":id,"url":url,"simulatedFailure":false,
        "httpStatus":status.as_u16(),"success":success,"hasOutput":!text.trim().is_empty(),
        "responseStatus":value.get("status"),"elapsedMs":started.elapsed().as_millis(),
    }));
    (status, Json(value)).into_response()
}

async fn call_gateway(base: &str, doc: &Document, server: &ProxyServer) -> Value {
    let groups = doc.groups();
    if groups.len() != 1 {
        return json!({"success":false,"reason":"Endpoint 不一致，不能组成同一路由","groups":groups.len()});
    }
    let group = &groups[0];
    let payload = match group.protocol {
        catalog::Protocol::OpenaiResponses => {
            json!({"model":group.id,"input":"Reply with OK.","stream":false,"max_output_tokens":64})
        }
        _ => {
            json!({"model":group.id,"messages":[{"role":"user","content":"Reply with OK."}],"stream":false,"max_tokens":64})
        }
    };
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(150))
        .build()
        .unwrap()
        .post(format!("{base}/v1/{}", group.protocol.endpoint()))
        .bearer_auth(LOCAL_TOKEN)
        .json(&payload)
        .send()
        .await;
    let Ok(response) = response else {
        return json!({"success":false,"reason":"网关请求超时或连接失败"});
    };
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let target = server
        .get_status()
        .await
        .active_targets
        .into_iter()
        .next()
        .map(|t| t.provider_name);
    json!({"httpStatus":status,"success":status == 200 && !output_text(&body).trim().is_empty(),"servedBy":target})
}

#[tokio::test]
#[ignore = "需明确授权且设置 CC_SWITCH_MANUAL_LIVE_HA=1；最多 18 次低输出真实调用"]
async fn verify_live_routes() {
    assert_eq!(
        std::env::var("CC_SWITCH_MANUAL_LIVE_HA").as_deref(),
        Ok("1"),
        "缺少真实调用授权开关"
    );
    let output = std::env::var("CC_SWITCH_MANUAL_HA_REPORT").expect("必须指定脱敏报告路径");
    let recheck = std::env::var("CC_SWITCH_MANUAL_HA_CASE").as_deref() == Ok("gpt-eagle-down");
    let max_calls = if recheck { 1 } else { MAX_CALLS };
    let path = crate::config::get_app_config_dir().join("cc-switch.db");
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let text: String = connection
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [catalog::DOCUMENT_KEY],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);
    let original: Document = serde_json::from_str(&text).unwrap();
    let requested = if recheck { &TARGETS[..1] } else { &TARGETS[..] };
    assert!(
        !requested
            .iter()
            .any(|id| original.blocked_models.contains(*id)),
        "验证目标已屏蔽，请先显式恢复；未发送请求"
    );
    let budget = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(vec![]));
    let mut relays = vec![];
    let mut tasks = vec![];
    for mut source in original.providers.into_iter().filter(|s| s.enabled) {
        source
            .models
            .retain(|m| m.enabled && TARGETS.contains(&m.id.as_str()));
        if source.models.is_empty() {
            continue;
        }
        catalog::validate_url(&source.base_url).unwrap();
        let state = Arc::new(Relay {
            source: source.clone(),
            fault: AtomicBool::new(false),
            budget: budget.clone(),
            max_calls,
            events: events.clone(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let path = url::Url::parse(&source.base_url)
            .unwrap()
            .path()
            .to_string();
        source.base_url = format!("http://{address}{path}");
        let app = Router::new().fallback(relay).with_state(state.clone());
        tasks.push(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        relays.push((source, state));
    }
    let mut report = vec![];
    for model_id in TARGETS {
        if recheck {
            if model_id != "gpt-6-astra" {
                continue;
            }
            let primary = relays
                .iter()
                .find(|(s, _)| s.name == "eagle")
                .expect("缺少 eagle");
            let backup = relays
                .iter()
                .find(|(s, _)| s.name == "eaips")
                .expect("缺少 eaips");
            primary.1.fault.store(true, Ordering::SeqCst);
            let sources = [primary.0.clone(), backup.0.clone()]
                .into_iter()
                .map(|mut source| {
                    source.models.retain(|m| m.id == model_id);
                    source
                })
                .collect();
            let (server, base, doc) = super::tests::gateway(sources).await;
            let result = call_gateway(&base, &doc, &server).await;
            let captured = events.lock().await.clone();
            let verified = result["success"] == true
                && captured
                    .first()
                    .is_some_and(|e| e["provider"] == "eagle" && e["simulatedFailure"] == true)
                && captured
                    .get(1)
                    .is_some_and(|e| e["provider"] == "eaips" && e["success"] == true);
            server.stop().await.unwrap();
            primary.1.fault.store(false, Ordering::SeqCst);
            report
                .push(json!({"model":model_id,"recheck":true,"verified":verified,"result":result}));
            continue;
        }
        let candidates: Vec<_> = relays
            .iter()
            .filter(|(s, _)| s.models.iter().any(|m| m.id == model_id))
            .collect();
        let mut independent = vec![];
        let mut healthy = vec![];
        for (source, control) in &candidates {
            let model = source.models.iter().find(|m| m.id == model_id).unwrap();
            let before = events.lock().await.len();
            let result = test_model_once(source, model).await;
            let captured = events.lock().await;
            let success =
                result.is_ok() && captured.get(before).is_some_and(|e| e["success"] == true);
            independent.push(json!({"provider":source.name,"success":success}));
            if success {
                healthy.push(((*source).clone(), (*control).clone()));
            }
        }
        let mut row = json!({"model":model_id,"configuredDeployments":candidates.len(),"independent":independent,"healthyDeployments":healthy.len()});
        let for_model = |sources: Vec<Source>| {
            sources
                .into_iter()
                .map(|mut s| {
                    s.models.retain(|m| m.id == model_id);
                    s
                })
                .collect::<Vec<_>>()
        };
        if candidates.len() > 1 {
            let sources = for_model(candidates.iter().map(|(s, _)| (*s).clone()).collect());
            let (server, base, doc) = super::tests::gateway(sources).await;
            let mut rounds = vec![];
            for _ in 0..candidates.len() {
                rounds.push(call_gateway(&base, &doc, &server).await);
            }
            server.stop().await.unwrap();
            row["roundRobin"] = json!(rounds);
        }
        if healthy.len() > 1 {
            let mut failovers = vec![];
            for index in 0..2 {
                let mut ordered = healthy.clone();
                ordered.rotate_left(index);
                let (primary, control) = &ordered[0];
                control.fault.store(true, Ordering::SeqCst);
                let before = events.lock().await.len();
                let (server, base, doc) = super::tests::gateway(for_model(
                    ordered.iter().map(|(s, _)| s.clone()).collect(),
                ))
                .await;
                let result = call_gateway(&base, &doc, &server).await;
                let captured = events.lock().await;
                let attempts = &captured[before..];
                let verified = result["success"] == true
                    && attempts
                        .first()
                        .is_some_and(|e| e["simulatedFailure"] == true)
                    && attempts
                        .iter()
                        .skip(1)
                        .any(|e| e["success"] == true && e["provider"] != primary.name);
                failovers.push(json!({"injectedFailure":primary.name,"verified":verified,"result":result,"attempts":attempts}));
                drop(captured);
                server.stop().await.unwrap();
                control.fault.store(false, Ordering::SeqCst);
            }
            row["failover"] = json!(failovers);
            for (_, control) in &healthy {
                control.fault.store(true, Ordering::SeqCst);
            }
            let before = budget.load(Ordering::SeqCst);
            let (server, base, doc) =
                super::tests::gateway(for_model(healthy.iter().map(|(s, _)| s.clone()).collect()))
                    .await;
            let result = call_gateway(&base, &doc, &server).await;
            row["allUnavailable"] = json!({"verified":result["success"] == false && budget.load(Ordering::SeqCst) == before,"result":result});
            server.stop().await.unwrap();
            for (_, control) in &healthy {
                control.fault.store(false, Ordering::SeqCst);
            }
        } else {
            row["haStatus"] = json!("不足两路可用部署，不能验证跨 Provider 高可用");
        }
        report.push(row);
    }
    for task in tasks {
        task.abort();
    }
    let passed = report.iter().all(|row| {
        if recheck {
            return row["verified"] == true;
        }
        let healthy = row["healthyDeployments"].as_u64().unwrap_or(0);
        if row["configuredDeployments"].as_u64().unwrap_or(0) < 2 {
            return healthy == 1;
        }
        let rounds = row["roundRobin"].as_array();
        let served: std::collections::HashSet<_> = rounds
            .into_iter()
            .flatten()
            .filter_map(|r| r["servedBy"].as_str())
            .collect();
        healthy >= 2
            && served.len() >= healthy as usize
            && rounds.is_some_and(|rounds| rounds.iter().all(|r| r["success"] == true))
            && row["failover"]
                .as_array()
                .is_some_and(|rows| rows.len() == 2 && rows.iter().all(|r| r["verified"] == true))
            && row["allUnavailable"]["verified"] == true
    });
    let report = json!({"passed":passed,"realRequests":budget.load(Ordering::SeqCst),"maxRealRequests":max_calls,"models":report,"events":events.lock().await.clone()});
    crate::config::atomic_write_private(
        std::path::Path::new(&output),
        serde_json::to_string_pretty(&report).unwrap().as_bytes(),
    )
    .unwrap();
    println!(
        "真实验证已结束：{} 次外发，脱敏报告 {output}",
        budget.load(Ordering::SeqCst)
    );
    assert!(passed, "实际可用线路或高可用验证未全部达标，详见脱敏报告");
}
