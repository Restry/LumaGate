mod aggregation;
pub mod catalog;
pub mod completion;
mod controls;
pub mod copilot;
pub mod drain;
pub mod endpoints;
pub mod keys;
mod limits;
#[cfg(test)]
mod live_ha;
mod log_preview;
pub mod logs;
pub mod metadata;
pub mod migration;
mod modalities;
mod names;
pub mod network;
pub mod pricing;
pub mod routing;
pub mod sync;
#[cfg(test)]
mod tests;
pub mod token_usage;
mod updater;

use crate::{
    database::Database,
    proxy::{
        server::{ProxyServer, ProxyState},
        types::ProxyConfig,
    },
};
use axum::{
    extract::State as HttpState,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use catalog::{Document, Model, Source, TestResult};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::{Manager, State};
use tokio::sync::Mutex;

pub const PORT: u16 = 15722;
#[cfg(test)]
pub const BASE: &str = "http://127.0.0.1:15722";
// 兼容要求填写 API Key 的客户端；占位值不参与网关鉴权。
pub const LOCAL_TOKEN: &str = "cc-switch-manual-local";

pub struct ManualState {
    pub db: Arc<Database>,
    copilot: Arc<copilot::Manager>,
    logs: std::sync::Mutex<Arc<logs::RequestLogs>>,
    drain: Arc<drain::Gate>,
    mutation: Mutex<()>,
    server: Mutex<Option<RunningGateway>>,
    plans: Mutex<HashMap<String, PendingSync>>,
}

struct PendingSync {
    plan: sync::Plan,
    network_revision: u64,
}

impl PendingSync {
    fn apply(self, db: &Database) -> Result<Vec<sync::Applied>, String> {
        if network::Settings::load(db)?.revision != self.network_revision {
            return Err("网络设置已变化，请重新预览 Agent 同步".into());
        }
        self.plan.apply(Document::load(db)?.revision)
    }
}

struct RunningGateway {
    server: ProxyServer,
    settings: network::Settings,
}

impl ManualState {
    fn current_logs(&self) -> Arc<logs::RequestLogs> {
        self.logs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    async fn save_network(&self, update: network::Update) -> Result<network::View, String> {
        let _guard = self.mutation.lock().await;
        if self.drain.closed() {
            return Err("更新排空期间不能更改监听设置".into());
        }
        let saved = network::Settings::save_update(&self.db, update)?;
        // Connection previews must never silently apply a stale endpoint.
        self.plans.lock().await.clear();
        Ok(saved.view())
    }

    async fn set_gateway(&self, running: bool) -> Result<(), String> {
        let _guard = self.mutation.lock().await;
        if self.drain.closed() {
            return Err("更新排空期间不能启停网关，请先取消更新".into());
        }
        let mut server = self.server.lock().await;
        if running && server.is_none() {
            let settings = network::Settings::load(&self.db)?;
            *server = Some(self.start_server(settings).await?);
            self.plans.lock().await.clear();
        } else if !running {
            if let Some(active) = server.as_ref() {
                active.server.stop().await.map_err(|e| e.to_string())?;
            }
            *server = None;
            self.plans.lock().await.clear();
        }
        Ok(())
    }

    async fn start_gateway_with(&self, settings: network::Settings) -> Result<(), String> {
        let _guard = self.mutation.lock().await;
        let mut server = self.server.lock().await;
        if server.is_none() {
            *server = Some(self.start_server(settings).await?);
        }
        Ok(())
    }

    async fn start_server(&self, settings: network::Settings) -> Result<RunningGateway, String> {
        let logs = self.current_logs();
        if !logs.ready() {
            return Err(
                "日志库不可用，请先在调用日志页面处理；未启动网关，也未静默丢弃日志".into(),
            );
        }
        let config = ProxyConfig {
            listen_address: settings.listen_address.clone(),
            listen_port: settings.listen_port,
            ..ProxyConfig::default()
        };
        // No AppHandle: the upstream failover UI must not write live client configurations.
        let server = ProxyServer::new(config, self.db.clone(), None)
            .with_manual_logs(logs)
            .with_manual_drain(self.drain.clone())
            .with_manual_network(settings.clone())
            .with_manual_copilot(self.copilot.clone());
        server.start().await.map_err(|e| e.to_string())?;
        Ok(RunningGateway { server, settings })
    }
}

impl ManualState {
    async fn set_provider_enabled(
        &self,
        provider_id: &str,
        enabled: bool,
        revision: u64,
    ) -> Result<(), String> {
        let _guard = self.mutation.lock().await;
        let mut doc = Document::load(&self.db)?;
        if doc.revision != revision {
            return Err("配置已变化，请刷新后重试".into());
        }
        let source = doc
            .providers
            .iter_mut()
            .find(|s| s.id == provider_id)
            .ok_or("Provider 不存在")?;
        source.enabled = enabled;
        // A routing control does not delete catalogs, credentials, policies or test history.
        doc.save(&self.db)
    }
}

#[tauri::command]
async fn manual_set_provider_enabled(
    state: State<'_, ManualState>,
    provider_id: String,
    enabled: bool,
    revision: u64,
) -> Result<(), String> {
    state
        .set_provider_enabled(&provider_id, enabled, revision)
        .await
}

#[tauri::command]
async fn manual_set_provider_cost_estimation(
    state: State<'_, ManualState>,
    provider_id: String,
    enabled: bool,
    revision: u64,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let mut doc = Document::load(&state.db)?;
    controls::set_cost_estimation(&mut doc, &provider_id, enabled, revision)?;
    doc.save(&state.db)
}

fn mirror_sources(db: &Database, doc: &Document) -> Result<(), String> {
    for app in ["claude", "codex"] {
        for source in &doc.providers {
            let provider = crate::provider::Provider::with_id(
                source.id.clone(),
                source.name.clone(),
                json!({"manual_source": source.id}),
                None,
            );
            db.save_provider(app, &provider)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn manual_snapshot(state: State<'_, ManualState>) -> Result<Value, String> {
    let _guard = state.mutation.lock().await;
    let doc = Document::load(&state.db)?;
    let settings = network::Settings::load(&state.db)?;
    let server = state.server.lock().await;
    let active = server.as_ref().map(|server| &server.settings);
    let status = match server.as_ref() {
        Some(server) => {
            serde_json::to_value(server.server.get_status().await).map_err(|e| e.to_string())?
        }
        None => json!({"running":false}),
    };
    let copilot_status = state
        .copilot
        .status(doc.providers.iter().find_map(|p| p.copilot.as_ref()))
        .await;
    Ok(
        json!({"document":doc,"groups":doc.groups(),"status":status,"copilotAuth":copilot_status,"logStorage":state.current_logs().storage_status(),
        "baseUrl":active.unwrap_or(&settings).base_url(),"network":settings.snapshot(active, &network::access_keys::Registry::load(&state.db)?)}),
    )
}

#[tauri::command]
async fn manual_request_logs(state: State<'_, ManualState>) -> Result<Vec<Value>, String> {
    let logs = state.current_logs();
    tauri::async_runtime::spawn_blocking(move || logs.persisted_snapshot())
        .await
        .map_err(|_| "日志读取任务失败".to_string())?
}

#[tauri::command]
async fn manual_query_logs(
    state: State<'_, ManualState>,
    mut query: logs::history::Query,
) -> Result<Value, String> {
    query.validate()?;
    query.apply_cost_settings(&Document::load(&state.db)?);
    let logs = state.current_logs();
    if !logs.ready() {
        return Ok(json!({"storage":logs.storage_status()}));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = logs.query_history(&query)?;
        result["storage"] = logs.storage_status();
        Ok(result)
    })
    .await
    .map_err(|_| "历史查询任务失败".to_string())?
}

#[tauri::command]
async fn manual_refresh_default_prices() -> Result<Value, String> {
    pricing::SERVICE.refresh(&pricing::cache_path()).await
}

#[tauri::command]
async fn manual_retry_log_storage(state: State<'_, ManualState>) -> Result<Value, String> {
    let _guard = state.mutation.lock().await;
    if state.server.lock().await.is_some() {
        return Err("请先手动停止网关，再重试打开日志库".into());
    }
    let old = {
        let mut slot = state.logs.lock().unwrap_or_else(|e| e.into_inner());
        if Arc::strong_count(&slot) > 1 {
            return Err("仍有日志读取正在完成，请稍后重试".into());
        }
        std::mem::replace(
            &mut *slot,
            Arc::new(logs::RequestLogs::unavailable("正在重新打开日志库".into())),
        )
    };
    let fresh = tauri::async_runtime::spawn_blocking(move || {
        drop(old);
        logs::RequestLogs::open(&crate::config::get_app_config_dir().join("logs"))
            .unwrap_or_else(logs::RequestLogs::unavailable)
    })
    .await
    .map_err(|_| "打开日志库的任务失败".to_string())?;
    let status = fresh.storage_status();
    *state.logs.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(fresh);
    Ok(status)
}

#[tauri::command]
async fn manual_open_log_directory() -> Result<(), String> {
    let directory = crate::config::get_app_config_dir().join("logs");
    let directory = if directory.is_dir() {
        directory
    } else {
        crate::config::get_app_config_dir()
    };
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "windows")]
    let command = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = "xdg-open";
    std::process::Command::new(command)
        .arg(directory)
        .spawn()
        .map_err(|_| "无法打开日志目录，请手动在文件管理器中打开显示的路径".to_string())?;
    Ok(())
}

#[tauri::command]
async fn manual_save(
    state: State<'_, ManualState>,
    document: Document,
    key_update: Option<keys::KeyUpdate>,
) -> Result<Vec<String>, String> {
    let _guard = state.mutation.lock().await;
    let current = Document::load(&state.db)?;
    copilot::validate_document_edit(&current, &document)?;
    if key_update.as_ref().is_some_and(|u| match u {
        keys::KeyUpdate::Set { provider_id, .. } | keys::KeyUpdate::Clear { provider_id } => {
            provider_id == copilot::PROVIDER_ID
        }
    }) {
        return Err("Copilot 使用 GitHub 登录，不接受 API Key".into());
    }
    let db = state.db.clone();
    // 文件写入和 SQLite 提交放到阻塞线程，避免拖住发现与转发请求。
    tauri::async_runtime::spawn_blocking(move || {
        keys::save_with_keys(
            &current,
            document,
            key_update,
            &keys::FileKeyStore::default(),
            |document| {
                mirror_sources(&db, document)?;
                document.save(&db)
            },
        )
    })
    .await
    .map_err(|_| "密钥保存任务失败，请重试".to_string())?
}

#[tauri::command]
async fn manual_set_model_protocol(
    state: State<'_, ManualState>,
    group_id: String,
    protocol: Option<catalog::Protocol>,
    revision: u64,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let mut doc = Document::load(&state.db)?;
    if revision != doc.revision {
        return Err("目录已变化，请刷新后重试".into());
    }
    if catalog::resolve_group(&doc.groups(), &group_id)
        .is_some_and(|g| g.model.id.starts_with("copilot/"))
    {
        return Err("Copilot Endpoint 由模型目录决定，请刷新模型，不手动覆盖".into());
    }
    endpoints::set_override(&mut doc, &group_id, protocol)?;
    doc.save(&state.db)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法建立 HTTP 客户端".into())
}
async fn request(
    client: &reqwest::Client,
    source: &Source,
    method: reqwest::Method,
    path: &str,
) -> Result<reqwest::RequestBuilder, String> {
    catalog::validate_url(&source.base_url)?;
    let credential_source = source.clone();
    let key =
        tauri::async_runtime::spawn_blocking(move || catalog::credentials(&credential_source))
            .await
            .map_err(|_| "密钥读取任务失败".to_string())??;
    let builder = client.request(method, catalog::endpoint(source, path));
    Ok(match source.protocol {
        catalog::Protocol::Anthropic => builder
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        _ if !key.is_empty() => builder.bearer_auth(key),
        _ => builder,
    })
}
async fn bounded_json(response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!("上游返回 HTTP {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "响应读取失败")?;
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("上游响应超过 2 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "上游未返回有效 JSON".into())
}

async fn discover_models(source: &Source) -> Result<Vec<Model>, String> {
    let client = client()?;
    let mut models: Vec<Model> = vec![];
    let mut cursor: Option<String> = None;
    let mut cursors = std::collections::HashSet::new();
    // 分页只读目录，不做探测。页数、总时间和响应体都有上限。
    for _ in 0..20 {
        let mut builder = request(&client, source, reqwest::Method::GET, "models").await?;
        if source.protocol == catalog::Protocol::Anthropic {
            builder = builder.query(&[("limit", "100")]);
        }
        if let Some(cursor) = &cursor {
            let key = if source.protocol == catalog::Protocol::Anthropic {
                "after_id"
            } else {
                "after"
            };
            builder = builder.query(&[(key, cursor)]);
        }
        let result = bounded_json(
            builder
                .send()
                .await
                .map_err(|_| "模型目录请求失败（网络或 TLS）")?,
        )
        .await?;
        let entries = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or("Provider 必须通过 /v1/models 返回 data 模型列表")?;
        for entry in entries {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            if models.iter().any(|model| model.id == id) {
                continue;
            }
            let enabled = source
                .models
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.enabled)
                .unwrap_or(true);
            let mut model = metadata::parse_model(entry, enabled)?;
            model.protocol_override = source
                .models
                .iter()
                .find(|old| old.id == id)
                .and_then(|old| old.protocol_override.clone());
            models.push(model);
            if models.len() > 2000 {
                return Err("目录超过 2000 个模型，未覆盖已有列表".into());
            }
        }
        if result.get("has_more").and_then(Value::as_bool) != Some(true) {
            return if models.is_empty() {
                Err("上游没有返回模型，已有列表保持不变".into())
            } else {
                Ok(models)
            };
        }
        let next = result
            .get("last_id")
            .and_then(Value::as_str)
            .or_else(|| {
                entries
                    .last()
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str)
            })
            .ok_or("分页目录缺少游标，已有列表保持不变")?;
        if !cursors.insert(next.to_string()) {
            return Err("上游分页未前进，已有列表保持不变".into());
        }
        cursor = Some(next.to_string());
    }
    Err("目录超过 20 页，未覆盖已有列表".into())
}

#[tauri::command]
async fn manual_discover(
    state: State<'_, ManualState>,
    provider_id: String,
) -> Result<usize, String> {
    discover_catalog(&state, &provider_id).await
}

async fn discover_catalog(state: &ManualState, provider_id: &str) -> Result<usize, String> {
    let _guard = state
        .mutation
        .try_lock()
        .map_err(|_| "已有配置操作进行中，请稍后刷新模型")?;
    let snapshot = Document::load(&state.db)?;
    let source = snapshot
        .providers
        .iter()
        .find(|s| s.id == provider_id)
        .ok_or("Provider 不存在")?;
    let models = tokio::time::timeout(Duration::from_secs(90), async {
        if source.copilot.is_some() {
            state.copilot.discover(source).await
        } else {
            discover_models(source).await
        }
    })
    .await
    .map_err(|_| "模型发现超时，已有列表保持不变")??;
    let count = models.len();
    let mut doc = Document::load(&state.db)?;
    if doc.revision != snapshot.revision {
        return Err("发现期间配置已变化，请重试".into());
    }
    doc.providers
        .iter_mut()
        .find(|s| s.id == provider_id)
        .ok_or("Provider 已删除")?
        .models = models;
    // 保留仍存在的策略；引用消失的 fallback 不静默替换，交由验证报错。
    doc.validate()?;
    if source.copilot.is_some() {
        controls::clear_source_tests(&mut doc, provider_id);
    } else {
        doc.tests.clear();
    }
    doc.save(&state.db)?;
    Ok(count)
}

async fn test_model_once(source: &Source, model: &Model) -> Result<(), String> {
    let mut effective_source = source.clone();
    effective_source.protocol = endpoints::protocol(model);
    let source = &effective_source;
    let output_limit = model.max_output_tokens.unwrap_or(64).min(64);
    let payload = match source.protocol {
        catalog::Protocol::OpenaiResponses => {
            json!({"model":model.id,"input":"Reply with OK.","max_output_tokens":output_limit,"stream":false})
        }
        _ => {
            json!({"model":model.id,"messages":[{"role":"user","content":"Reply with OK."}],"max_tokens":output_limit,"stream":false})
        }
    };
    let client = client()?;
    let value = bounded_json(
        request(
            &client,
            source,
            reqwest::Method::POST,
            source.protocol.endpoint(),
        )
        .await?
        .json(&payload)
        .send()
        .await
        .map_err(|_| "请求失败（网络或 TLS）")?,
    )
    .await?;
    // Responses 成功响应会包含 error:null，只有非 null 错误才表示失败。
    if value.get("error").is_some_and(|error| !error.is_null())
        || value.get("status").and_then(Value::as_str) == Some("failed")
    {
        return Err("上游返回错误信封".to_string());
    }
    let has_output = ["choices", "content", "output"].iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    });
    if !has_output {
        return Err("HTTP 成功，但没有有效输出".into());
    }
    Ok(())
}

#[tauri::command]
async fn manual_test(
    state: State<'_, ManualState>,
    group_id: String,
    provider_id: String,
) -> Result<TestResult, String> {
    let _request = state.drain.enter()?;
    let doc = Document::load(&state.db)?;
    let (source, model) = controls::test_target(&doc, &group_id, &provider_id)?;
    let result = if source.copilot.is_some() {
        let start = std::time::Instant::now();
        let outcome = state.copilot.test(source, model).await;
        TestResult {
            success: outcome.is_ok(),
            latency_ms: start.elapsed().as_millis() as u64,
            checked_at: chrono::Utc::now().to_rfc3339(),
            detail: outcome.err().unwrap_or("测试通过".into()),
            provider_id: Some(source.id.clone()),
            provider_name: Some(source.name.clone()),
            model_id: Some(model.id.clone()),
        }
    } else {
        controls::test_deployment(&doc, &group_id, &provider_id).await?
    };
    let _guard = state.mutation.lock().await;
    let mut current = Document::load(&state.db)?;
    if current.revision != doc.revision {
        return Err("测试期间配置已变化，结果未写入".into());
    }
    controls::record_test(&mut current, &group_id, &provider_id, result.clone());
    current.save(&state.db)?;
    Ok(result)
}

#[tauri::command]
async fn manual_set_model_blocked(
    state: State<'_, ManualState>,
    model_id: String,
    blocked: bool,
    revision: u64,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let mut doc = Document::load(&state.db)?;
    if doc.revision != revision {
        return Err("目录已变化，请刷新后重试".into());
    }
    controls::set_blocked(&mut doc, &model_id, blocked)?;
    doc.save(&state.db)
}

#[tauri::command]
async fn manual_gateway(state: State<'_, ManualState>, running: bool) -> Result<(), String> {
    state.set_gateway(running).await
}

#[tauri::command]
async fn manual_save_gateway_settings(
    state: State<'_, ManualState>,
    settings: network::Update,
) -> Result<network::View, String> {
    state.save_network(settings).await
}

#[tauri::command]
async fn manual_update_access_key(
    state: State<'_, ManualState>,
    revision: u64,
    operation: network::access_keys::Operation,
) -> Result<network::access_keys::ChangeResult, String> {
    let _guard = state.mutation.lock().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let registry = network::access_keys::Registry::load(&db)?;
        registry.change(
            revision,
            operation,
            &keys::FileKeyStore::gateway_access(),
            |value| {
                db.set_setting(network::access_keys::REGISTRY_KEY, value)
                    .map_err(|_| "保存访问密钥列表失败，旧记录未改变".to_string())
            },
        )
    })
    .await
    .map_err(|_| "访问密钥保存任务失败，请重试".to_string())?
}

#[tauri::command]
async fn manual_copy_access_key(
    state: State<'_, ManualState>,
    key_id: String,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Keep clipboard ownership alive on Linux; also avoids WebView user-gesture restrictions.
        static CLIPBOARD: once_cell::sync::Lazy<std::sync::Mutex<Option<arboard::Clipboard>>> =
            once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));
        let value = network::access_keys::Registry::load(&db)?
            .reveal(&key_id, &keys::FileKeyStore::gateway_access())?;
        let mut clipboard = CLIPBOARD.lock().map_err(|_| "无法访问系统剪贴板")?;
        if clipboard.is_none() {
            *clipboard =
                Some(arboard::Clipboard::new().map_err(|_| "无法打开系统剪贴板，可手动复制")?);
        }
        clipboard
            .as_mut()
            .expect("clipboard initialized")
            .set_text(value)
            .map_err(|_| "无法写入系统剪贴板，可手动复制".to_string())
    })
    .await
    .map_err(|_| "复制访问密钥任务失败，请重试".to_string())?
}

#[tauri::command]
async fn manual_reveal_access_key(
    state: State<'_, ManualState>,
    key_id: String,
) -> Result<String, String> {
    let _guard = state.mutation.lock().await;
    let db = state.db.clone();
    // The only read path returning a plaintext gateway credential; never part of snapshots/logs.
    tauri::async_runtime::spawn_blocking(move || {
        network::access_keys::Registry::load(&db)?
            .reveal(&key_id, &keys::FileKeyStore::gateway_access())
    })
    .await
    .map_err(|_| "访问密钥读取任务失败，请重试".to_string())?
}

#[tauri::command]
async fn manual_preview_sync(
    state: State<'_, ManualState>,
    agent: sync::Agent,
) -> Result<sync::Preview, String> {
    let _guard = state.mutation.lock().await;
    let doc = Document::load(&state.db)?;
    let settings = network::Settings::load(&state.db)?;
    let server = state.server.lock().await;
    let base = settings.sync_base(server.as_ref().map(|active| &active.settings))?;
    drop(server);
    let home = crate::config::get_home_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let plan = sync::Plan::build(&home, &doc, agent, &base)?;
    let preview = plan.preview();
    let mut plans = state.plans.lock().await;
    plans.clear();
    plans.insert(
        plan.id.clone(),
        PendingSync {
            plan,
            network_revision: settings.revision,
        },
    );
    Ok(preview)
}

#[tauri::command]
async fn manual_apply_sync(
    state: State<'_, ManualState>,
    plan_id: String,
) -> Result<Vec<sync::Applied>, String> {
    let _guard = state.mutation.lock().await;
    let plan = state
        .plans
        .lock()
        .await
        .remove(&plan_id)
        .ok_or("预览不存在或已使用，请重新预览")?;
    plan.apply(&state.db)
}

#[tauri::command]
async fn manual_launch(agent: sync::Agent) -> Result<(), String> {
    // 参数只能来自枚举；不接受命令文本、环境变量或工作目录，避免启动入口变成配置注入入口。
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("osascript");
        command.args([
            "-e",
            &format!(
                "tell application \"Terminal\" to do script \"{}\"",
                agent.command()
            ),
            "-e",
            "tell application \"Terminal\" to activate",
        ]);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("wt");
        command.args(["new-tab", agent.command()]);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = std::process::Command::new("x-terminal-emulator");
        command.args(["-e", agent.command()]);
        command
    };
    let status = tauri::async_runtime::spawn_blocking(move || command.status())
        .await
        .map_err(|_| "启动任务失败")?
        .map_err(|_| "无法打开终端；请确认终端和 Agent 已安装")?;
    if status.success() {
        Ok(())
    } else {
        Err("终端拒绝启动；请检查桌面自动化权限。没有修改 Agent 配置".into())
    }
}

fn http_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error":{"code":code,"message":message,"type":"gateway_error"}})),
    )
        .into_response()
}

pub async fn models(HttpState(state): HttpState<ProxyState>) -> Response {
    match Document::load(&state.db) {
        Ok(doc) => Json(catalog::public_models(&doc)).into_response(),
        Err(_) => http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "catalog_unavailable",
            "本地模型目录暂时不可用，请在应用内检查配置",
        ),
    }
}
pub(crate) fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    let _instance = match migration::prepare(&crate::config::get_home_dir()) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("LumaGate 数据迁移未完成：{error}");
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("osascript")
                .args(["-e", "on run argv\n display alert \"LumaGate 无法安全打开数据\" message (item 1 of argv) as critical\nend run", &error.to_string()])
                .status();
            return;
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let db = Arc::new(Database::init()?);
            let mut doc = Document::load(&db).map_err(std::io::Error::other)?;
            if db.get_setting(catalog::DOCUMENT_KEY)?.is_none() {
                doc.save(&db).map_err(std::io::Error::other)?;
            }
            mirror_sources(&db, &doc).map_err(std::io::Error::other)?;
            tauri::async_runtime::block_on(async {
                for name in ["claude", "codex"] {
                    let mut config = db.get_proxy_config_for_app(name).await?;
                    config.enabled = false;
                    config.auto_failover_enabled = true;
                    config.max_retries = 3;
                    db.update_proxy_config_for_app(config).await?;
                }
                Ok::<(), crate::error::AppError>(())
            })?;
            let logs = Arc::new(
                logs::RequestLogs::open(&crate::config::get_app_config_dir().join("logs"))
                    .unwrap_or_else(logs::RequestLogs::unavailable),
            );
            pricing::SERVICE.load(&pricing::cache_path());
            app.manage(ManualState {
                db,
                copilot: Arc::new(copilot::Manager::new(
                    crate::config::get_app_config_dir().join("copilot"),
                )),
                logs: std::sync::Mutex::new(logs),
                drain: Arc::new(drain::Gate::default()),
                mutation: Mutex::new(()),
                server: Mutex::new(None),
                plans: Mutex::new(HashMap::new()),
            });
            let updates = updater::Updates::new(&app.state::<ManualState>().db)?;
            app.manage(updates);
            // Restore the one-use update intent before the renderer takes its first snapshot.
            let recovery_failed = updater::restore_before_show(app.handle());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                updater::startup(handle, recovery_failed).await;
            });
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        // Only guarded native update commands; no direct JS install/restart capability.
        .invoke_handler(tauri::generate_handler![
            updater::manual_update_state,
            updater::manual_update_check,
            updater::manual_update_install,
            updater::manual_update_later,
            updater::manual_update_auto,
            manual_snapshot,
            copilot::manual_copilot_start,
            copilot::manual_copilot_poll,
            copilot::manual_copilot_cancel,
            copilot::manual_copilot_disconnect,
            copilot::manual_copilot_open_login,
            manual_request_logs,
            manual_query_logs,
            manual_refresh_default_prices,
            manual_retry_log_storage,
            manual_open_log_directory,
            manual_save,
            manual_set_provider_enabled,
            manual_set_provider_cost_estimation,
            manual_set_model_protocol,
            manual_set_model_blocked,
            manual_discover,
            manual_test,
            manual_gateway,
            manual_save_gateway_settings,
            manual_update_access_key,
            manual_copy_access_key,
            manual_reveal_access_key,
            manual_preview_sync,
            manual_apply_sync,
            manual_launch
        ])
        .build(context())
        .expect("无法启动 LumaGate")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit)
                && app.state::<ManualState>().current_logs().flush().is_err()
            {
                log::error!("退出前调用日志未能全部保存，请检查日志存储");
            }
        });
}
