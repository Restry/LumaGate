use super::catalog::{hash, Document, Protocol};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

/// Codex 的网关 profile 名。写入配置和启动命令都用这一个常量，
/// 两端必须一致，否则启动会指向不存在的 profile。
pub const CODEX_PROFILE: &str = "cc_switch_manual";
/// LumaGate 接管的 Codex 模型目录文件名。
pub const CODEX_CATALOG_FILE: &str = "cc-switch-manual-models.json";
/// 持久化自动同步偏好的 SQLite settings 键。
pub const AUTO_SYNC_KEY: &str = "manual_auto_sync_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Pi,
    Claude,
    Codex,
    Opencode,
    Openclaw,
    Hermes,
    Droid,
}
impl Agent {
    pub fn command(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Openclaw => "openclaw",
            Self::Hermes => "hermes",
            Self::Droid => "droid",
        }
    }

    /// 网关接入后的启动命令。只返回编译期常量；不拼接外部输入、
    /// 环境变量或工作目录，避免启动入口变成配置注入入口。
    pub fn gateway_command(self) -> &'static str {
        match self {
            Self::Codex => "codex --profile cc_switch_manual",
            other => other.command(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoSyncSettings {
    #[serde(default)]
    pub codex: bool,
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub pi: bool,
    #[serde(default)]
    pub opencode: bool,
    #[serde(default)]
    pub openclaw: bool,
    #[serde(default)]
    pub hermes: bool,
    #[serde(default)]
    pub droid: bool,
}

impl AutoSyncSettings {
    pub fn load(db: &crate::database::Database) -> Self {
        db.get_setting(AUTO_SYNC_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, db: &crate::database::Database) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        db.set_setting(AUTO_SYNC_KEY, &text)
            .map_err(|e| e.to_string())
    }

    pub fn set(&mut self, agent: Agent, enabled: bool) {
        match agent {
            Agent::Codex => self.codex = enabled,
            Agent::Claude => self.claude = enabled,
            Agent::Pi => self.pi = enabled,
            Agent::Opencode => self.opencode = enabled,
            Agent::Openclaw => self.openclaw = enabled,
            Agent::Hermes => self.hermes = enabled,
            Agent::Droid => self.droid = enabled,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexRestoreState {
    previous_openai_base_url: Option<String>,
    previous_model_catalog_json: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexClientStatus {
    pub connected: bool,
    pub mode: String, // "desktop" | "profile" | "other" | "official"
    pub current_base_url: Option<String>,
    pub current_model: Option<String>,
    pub catalog_path: Option<String>,
    pub catalog_owner: Option<String>, // "lumagate" | "magpie" | "other"
    pub catalog_models: Vec<String>,
    pub auto_sync: bool,
    pub can_restore: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleClientStatus {
    pub connected: bool,
    pub installed: bool,
    pub current_base_url: Option<String>,
    pub auto_sync: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsStatus {
    pub codex: CodexClientStatus,
    pub claude: SimpleClientStatus,
    pub pi: SimpleClientStatus,
    pub opencode: SimpleClientStatus,
    pub openclaw: SimpleClientStatus,
    pub hermes: SimpleClientStatus,
    pub droid: SimpleClientStatus,
}

/// 生成 Codex 兼容的 `{ "models": [...] }` 目录 JSON，供 `GET /backend-api/codex/models`
/// 和 `~/.codex/cc-switch-manual-models.json` 共用。使用干净的 `public_id` 作为 slug。
pub fn build_codex_catalog_value(doc: &Document) -> Result<Value, String> {
    let groups: Vec<_> = doc
        .groups()
        .into_iter()
        .filter(|group| !group.blocked)
        .collect();
    if groups.is_empty() {
        return Ok(json!({ "models": [] }));
    }
    let settings = json!({
        "modelCatalog": {
            "models": groups
                .iter()
                .map(|g| json!({
                    "model": g.public_id,
                    "displayName": g.display_name
                }))
                .collect::<Vec<_>>()
        }
    });
    let mut catalog = crate::codex_config::codex_model_catalog_from_settings(
        &settings,
        "",
        crate::codex_config::CodexCatalogToolProfile::NativeResponses,
    )
    .map_err(|e| e.to_string())?
    .ok_or("无法生成 Codex 模型目录")?;
    if let Some(entries) = catalog.get_mut("models").and_then(Value::as_array_mut) {
        for entry in entries {
            if let Some(group) = groups
                .iter()
                .find(|g| entry.get("slug").and_then(Value::as_str) == Some(g.public_id.as_str()))
            {
                super::metadata::enrich_codex_entry(entry, &group.model);
                entry["visibility"] = json!("list");
                entry["supported_in_api"] = json!(true);
            }
        }
    }
    Ok(catalog)
}

/// 该客户端的网关接入是否已落盘。
pub fn gateway_profile_ready(home: &Path, agent: Agent) -> bool {
    match agent {
        Agent::Codex => {
            let Ok(Some(bytes)) = read(&home.join(".codex/config.toml")) else {
                return false;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return false;
            };
            text.parse::<toml_edit::DocumentMut>().ok().is_some_and(|doc| {
                doc.get("profiles")
                    .and_then(|profiles| profiles.get(CODEX_PROFILE))
                    .is_some()
            })
        }
        _ => true,
    }
}

fn opencode_config_path(home: &Path) -> PathBuf {
    let dir = home.join(".config/opencode");
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.is_file() {
        jsonc
    } else {
        dir.join("opencode.json")
    }
}

fn parse_json5(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "配置文件必须是 UTF-8")?;
    let val: Value = json5::from_str(text).map_err(|e| format!("配置 JSON/JSONC 解析失败: {e}"))?;
    if !val.is_object() {
        return Err("配置必须是 JSON 对象".into());
    }
    Ok(val)
}

fn build_opencode_providers(groups: &[super::catalog::Group], base: &str) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let v1 = format!("{}/v1", base.trim_end_matches('/'));
    let mut openai_models = serde_json::Map::new();
    let mut anthropic_models = serde_json::Map::new();
    for g in groups {
        let mut entry = serde_json::Map::new();
        entry.insert(
            "name".into(),
            json!(g.model.name.as_deref().unwrap_or(&g.display_name)),
        );
        let mut limit = serde_json::Map::new();
        if let Some(ctx) = g.model.context_window {
            limit.insert("context".into(), json!(ctx));
        }
        if let Some(out_tok) = g.model.max_output_tokens {
            limit.insert("output".into(), json!(out_tok));
        }
        if !limit.is_empty() {
            entry.insert("limit".into(), Value::Object(limit));
        }
        if g.protocol == Protocol::Anthropic {
            anthropic_models.insert(g.public_id.clone(), Value::Object(entry));
        } else {
            openai_models.insert(g.public_id.clone(), Value::Object(entry));
        }
    }
    if !openai_models.is_empty() {
        out.insert(
            "lumagate".into(),
            json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "LumaGate",
                "options": { "baseURL": v1, "apiKey": super::LOCAL_TOKEN },
                "models": openai_models
            }),
        );
    }
    if !anthropic_models.is_empty() {
        out.insert(
            "lumagate-anthropic".into(),
            json!({
                "npm": "@ai-sdk/anthropic",
                "name": "LumaGate (Anthropic)",
                "options": { "baseURL": format!("{}/v1", base.trim_end_matches('/')), "apiKey": super::LOCAL_TOKEN },
                "models": anthropic_models
            }),
        );
    }
    out
}

fn build_openclaw_provider(groups: &[super::catalog::Group], base: &str) -> Value {
    let models: Vec<Value> = groups
        .iter()
        .map(|g| {
            let mut m = serde_json::Map::new();
            m.insert("id".into(), json!(g.public_id));
            m.insert(
                "name".into(),
                json!(g.model.name.as_deref().unwrap_or(&g.display_name)),
            );
            if let Some(ctx) = g.model.context_window {
                m.insert("contextWindow".into(), json!(ctx));
            }
            Value::Object(m)
        })
        .collect();
    json!({
        "baseUrl": format!("{}/v1", base.trim_end_matches('/')),
        "apiKey": super::LOCAL_TOKEN,
        "api": "openai-completions",
        "models": models
    })
}

fn build_hermes_provider(groups: &[super::catalog::Group], base: &str) -> serde_yaml::Value {
    let mut models_map = serde_yaml::Mapping::new();
    for g in groups {
        let mut entry = serde_yaml::Mapping::new();
        if let Some(ctx) = g.model.context_window {
            entry.insert(
                serde_yaml::Value::String("context_length".into()),
                serde_yaml::Value::Number(serde_yaml::Number::from(ctx)),
            );
        }
        models_map.insert(
            serde_yaml::Value::String(g.public_id.clone()),
            serde_yaml::Value::Mapping(entry),
        );
    }
    let mut provider = serde_yaml::Mapping::new();
    provider.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String("lumagate".into()),
    );
    provider.insert(
        serde_yaml::Value::String("base_url".into()),
        serde_yaml::Value::String(format!("{}/v1", base.trim_end_matches('/'))),
    );
    provider.insert(
        serde_yaml::Value::String("api_key".into()),
        serde_yaml::Value::String(super::LOCAL_TOKEN.into()),
    );
    if let Some(first) = groups.first() {
        provider.insert(
            serde_yaml::Value::String("model".into()),
            serde_yaml::Value::String(first.public_id.clone()),
        );
    }
    provider.insert(
        serde_yaml::Value::String("models".into()),
        serde_yaml::Value::Mapping(models_map),
    );
    serde_yaml::Value::Mapping(provider)
}

fn build_droid_custom_models(groups: &[super::catalog::Group], base: &str) -> Vec<Value> {
    let base_trimmed = base.trim_end_matches('/');
    groups
        .iter()
        .map(|g| {
            let is_anthropic = g.protocol == Protocol::Anthropic;
            let mut m = serde_json::Map::new();
            m.insert("id".into(), json!(format!("lumagate-{}", g.public_id)));
            m.insert("model".into(), json!(g.public_id));
            m.insert(
                "displayName".into(),
                json!(format!(
                    "{} (LumaGate)",
                    g.model.name.as_deref().unwrap_or(&g.display_name)
                )),
            );
            m.insert(
                "baseUrl".into(),
                json!(if is_anthropic {
                    base_trimmed.to_string()
                } else {
                    format!("{base_trimmed}/v1")
                }),
            );
            m.insert("apiKey".into(), json!(super::LOCAL_TOKEN));
            m.insert(
                "provider".into(),
                json!(if is_anthropic { "anthropic" } else { "openai" }),
            );
            if let Some(out) = g.model.max_output_tokens {
                m.insert("maxOutputTokens".into(), json!(out));
            }
            Value::Object(m)
        })
        .collect()
}

fn read_catalog_model_slugs(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return vec![];
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return vec![];
    };
    value
        .get("models")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("visibility").and_then(Value::as_str) != Some("hide"))
                .filter_map(|item| {
                    item.get("slug")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn restore_state_path() -> PathBuf {
    crate::config::get_app_config_dir().join("codex-restore.json")
}

/// 去掉 Windows `canonicalize()` 产生的 `\\?\` 逐字路径前缀，避免写入配置文件后让外部 CLI 无法识别。
fn clean_path_str(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        raw.into_owned()
    }
}

/// 只读探测本机 Codex / Claude Code / Pi 的当前连接状态与本地 Codex 模型清单。
pub fn inspect_agents(home: &Path, base: &str, auto_sync: &AutoSyncSettings) -> AgentsStatus {
    let lumagate_catalog_str = clean_path_str(&home.join(".codex").join(CODEX_CATALOG_FILE));
    let expected_codex_base = format!("{}/backend-api/codex", base.trim_end_matches('/'));
    let expected_v1_base = format!("{}/v1", base.trim_end_matches('/'));

    let codex = (|| {
        let mut current_base_url = None;
        let mut current_model = None;
        let mut catalog_path = None;
        let mut has_profile = false;
        let mut profile_points_to_lumagate = false;

        if let Ok(Some(bytes)) = read(&home.join(".codex/config.toml")) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Ok(doc) = text.parse::<toml_edit::DocumentMut>() {
                    current_base_url = doc
                        .get("openai_base_url")
                        .and_then(toml_edit::Item::as_str)
                        .map(str::to_owned);
                    current_model = doc
                        .get("model")
                        .and_then(toml_edit::Item::as_str)
                        .map(str::to_owned);
                    catalog_path = doc
                        .get("model_catalog_json")
                        .and_then(toml_edit::Item::as_str)
                        .map(str::to_owned);
                    has_profile = doc
                        .get("profiles")
                        .and_then(|p| p.get(CODEX_PROFILE))
                        .is_some();
                    profile_points_to_lumagate = doc
                        .get("model_providers")
                        .and_then(|p| p.get(CODEX_PROFILE))
                        .and_then(|p| p.get("base_url"))
                        .and_then(toml_edit::Item::as_str)
                        .is_some_and(|u| u.trim_end_matches('/') == expected_v1_base);
                }
            }
        }

        let base_points_to_lumagate = current_base_url
            .as_deref()
            .is_some_and(|u| {
                let u = u.trim_end_matches('/');
                u == expected_codex_base || u == expected_v1_base
            });
        let catalog_is_lumagate = catalog_path.as_deref().is_some_and(|p| {
            p == lumagate_catalog_str || p.ends_with(CODEX_CATALOG_FILE)
        });
        let catalog_owner = catalog_path.as_deref().map(|p| {
            if p == lumagate_catalog_str || p.ends_with(CODEX_CATALOG_FILE) {
                "lumagate".to_string()
            } else if p.to_ascii_lowercase().contains("magpie") {
                "magpie".to_string()
            } else {
                "other".to_string()
            }
        });
        let mode = if base_points_to_lumagate && catalog_is_lumagate {
            "desktop"
        } else if catalog_is_lumagate && has_profile && profile_points_to_lumagate {
            "profile"
        } else if current_base_url.is_some() || catalog_path.is_some() {
            "other"
        } else {
            "official"
        };
        let connected = matches!(mode, "desktop" | "profile");
        let catalog_models = if let Some(path_str) = &catalog_path {
            let p = PathBuf::from(path_str);
            let resolved = if p.is_absolute() {
                p
            } else {
                home.join(".codex").join(p)
            };
            read_catalog_model_slugs(&resolved)
        } else {
            read_catalog_model_slugs(&home.join(".codex/models_cache.json"))
        };

        CodexClientStatus {
            connected,
            mode: mode.into(),
            current_base_url,
            current_model,
            catalog_path,
            catalog_owner,
            catalog_models,
            auto_sync: auto_sync.codex,
            can_restore: restore_state_path().is_file(),
        }
    })();

    let claude = (|| {
        let mut current_base_url = None;
        if let Ok(Some(bytes)) = read(&home.join(".claude/settings.json")) {
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                current_base_url = val
                    .pointer("/env/ANTHROPIC_BASE_URL")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
        }
        let connected = current_base_url
            .as_deref()
            .is_some_and(|u| u.trim_end_matches('/') == base.trim_end_matches('/'));
        let installed = home.join(".claude").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url,
            auto_sync: auto_sync.claude,
        }
    })();

    let pi = (|| {
        let mut connected = false;
        if let Ok(Some(bytes)) = read(&home.join(".pi/agent/models.json")) {
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(providers) = val.get("providers").and_then(Value::as_object) {
                    connected = providers.iter().any(|(k, v)| {
                        k.starts_with("cc-switch-manual-")
                            && v.get("baseUrl")
                                .and_then(Value::as_str)
                                .is_some_and(|u| u.trim_end_matches('/') == expected_v1_base)
                    });
                }
            }
        }
        let installed = home.join(".pi").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url: connected.then_some(expected_v1_base.clone()),
            auto_sync: auto_sync.pi,
        }
    })();

    let opencode = (|| {
        let path = opencode_config_path(home);
        let mut current_base_url = None;
        if let Ok(Some(bytes)) = read(&path) {
            if let Ok(val) = parse_json5(&bytes) {
                current_base_url = val
                    .pointer("/provider/lumagate/options/baseURL")
                    .or_else(|| val.pointer("/provider/lumagate-anthropic/options/baseURL"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
        }
        let connected = current_base_url
            .as_deref()
            .is_some_and(|u| u.trim_end_matches('/') == expected_v1_base);
        let installed = home.join(".config/opencode").is_dir() || home.join(".opencode").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url,
            auto_sync: auto_sync.opencode,
        }
    })();

    let openclaw = (|| {
        let mut current_base_url = None;
        if let Ok(Some(bytes)) = read(&home.join(".openclaw/openclaw.json")) {
            if let Ok(val) = parse_json5(&bytes) {
                current_base_url = val
                    .pointer("/models/providers/lumagate/baseUrl")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
        }
        let connected = current_base_url
            .as_deref()
            .is_some_and(|u| u.trim_end_matches('/') == expected_v1_base);
        let installed = home.join(".openclaw").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url,
            auto_sync: auto_sync.openclaw,
        }
    })();

    let hermes = (|| {
        let mut current_base_url = None;
        if let Ok(Some(bytes)) = read(&home.join(".hermes/config.yaml")) {
            if let Ok(val) = serde_yaml::from_slice::<serde_yaml::Value>(&bytes) {
                if let Some(list) = val.get("custom_providers").and_then(|v| v.as_sequence()) {
                    for item in list {
                        if item.get("name").and_then(|v| v.as_str()) == Some("lumagate") {
                            current_base_url = item
                                .get("base_url")
                                .and_then(|v| v.as_str())
                                .map(str::to_owned);
                            break;
                        }
                    }
                }
            }
        }
        let connected = current_base_url
            .as_deref()
            .is_some_and(|u| u.trim_end_matches('/') == expected_v1_base);
        let installed = home.join(".hermes").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url,
            auto_sync: auto_sync.hermes,
        }
    })();

    let droid = (|| {
        let mut connected = false;
        let mut current_base_url = None;
        if let Ok(Some(bytes)) = read(&home.join(".factory/settings.json")) {
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(models) = val.get("customModels").and_then(Value::as_array) {
                    for item in models {
                        if item
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.starts_with("lumagate-"))
                        {
                            if let Some(u) = item.get("baseUrl").and_then(Value::as_str) {
                                current_base_url = Some(u.to_string());
                                let trimmed = u.trim_end_matches('/');
                                if trimmed == expected_v1_base
                                    || trimmed == base.trim_end_matches('/')
                                {
                                    connected = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        let installed = home.join(".factory").is_dir();
        SimpleClientStatus {
            connected,
            installed,
            current_base_url,
            auto_sync: auto_sync.droid,
        }
    })();

    AgentsStatus {
        codex,
        claude,
        pi,
        opencode,
        openclaw,
        hermes,
        droid,
    }
}

/// 自动刷新已连接客户端的受管模型目录文件（仅在用户已显式接入并开启自动同步时执行）。
/// 绝不修改 `~/.codex/config.toml` 或任何其他工具接管的目录文件。
pub fn sync_managed_catalogs(
    home: &Path,
    doc: &Document,
    base: &str,
    settings: &AutoSyncSettings,
) -> Result<Vec<Agent>, String> {
    let mut updated = Vec::new();
    let status = inspect_agents(home, base, settings);

    if settings.codex
        && status.codex.connected
        && status.codex.catalog_owner.as_deref() == Some("lumagate")
    {
        let catalog_path = home.join(".codex").join(CODEX_CATALOG_FILE);
        reject_symlinks(&catalog_path)?;
        let catalog = build_codex_catalog_value(doc)?;
        if let Some(parent) = catalog_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::config::atomic_write_private(&catalog_path, &json_bytes(&catalog))
            .map_err(|e| e.to_string())?;
        updated.push(Agent::Codex);
    }

    if settings.pi && status.pi.connected {
        let path = home.join(".pi/agent/models.json");
        if let Ok(Some(before)) = read(&path) {
            if let Ok(mut config) = json_config(&Some(before)) {
                if let Ok(providers) = object_field(&mut config, "providers") {
                    let groups: Vec<_> = doc
                        .groups()
                        .into_iter()
                        .filter(|g| !g.blocked)
                        .collect();
                    for (format, api) in [
                        (Protocol::Anthropic, "anthropic-messages"),
                        (Protocol::OpenaiChat, "openai-completions"),
                        (Protocol::OpenaiResponses, "openai-responses"),
                    ] {
                        let models: Vec<Value> = groups
                            .iter()
                            .filter(|g| g.protocol == format)
                            .map(|g| super::metadata::pi_model(&g.model, &g.public_id))
                            .collect();
                        let key = format!("cc-switch-manual-{}", format.format());
                        if models.is_empty() {
                            providers.remove(&key);
                        } else {
                            providers.insert(
                                key,
                                json!({
                                    "baseUrl": format!("{base}/v1"),
                                    "api": api,
                                    "apiKey": super::LOCAL_TOKEN,
                                    "models": models
                                }),
                            );
                        }
                    }
                    let _ = crate::config::atomic_write_private(&path, &json_bytes(&config));
                    updated.push(Agent::Pi);
                }
            }
        }
    }

    if settings.claude && status.claude.connected {
        let path = home.join(".claude/cc-switch-manual-models.json");
        if path.is_file() && reject_symlinks(&path).is_ok() {
            let groups: Vec<_> = doc
                .groups()
                .into_iter()
                .filter(|g| !g.blocked)
                .collect();
            let list = json!({
                "note": "辅助清单，不是 Claude Code 原生自动加载配置；用 /model <id> 手动选择",
                "models": groups.iter().map(|g| json!({
                    "id": g.public_id,
                    "routeId": g.id,
                    "name": g.model.name.as_deref().unwrap_or(&g.display_name),
                    "contextWindow": g.model.context_window,
                    "capabilities": super::metadata::capability_details(&g.model)
                })).collect::<Vec<_>>()
            });
            let _ = crate::config::atomic_write_private(&path, &json_bytes(&list));
            updated.push(Agent::Claude);
        }
    }

    let groups: Vec<_> = doc.groups().into_iter().filter(|g| !g.blocked).collect();

    if settings.opencode && status.opencode.connected {
        let path = opencode_config_path(home);
        if let Ok(Some(before)) = read(&path) {
            if let Ok(mut config) = parse_json5(&before) {
                if let Ok(providers) = object_field(&mut config, "provider") {
                    providers.remove("lumagate");
                    providers.remove("lumagate-anthropic");
                    for (k, v) in build_opencode_providers(&groups, base) {
                        providers.insert(k, v);
                    }
                    let _ = crate::config::atomic_write_private(&path, &json_bytes(&config));
                    updated.push(Agent::Opencode);
                }
            }
        }
    }

    if settings.openclaw && status.openclaw.connected {
        let path = home.join(".openclaw/openclaw.json");
        if let Ok(Some(before)) = read(&path) {
            if let Ok(mut config) = parse_json5(&before) {
                if let Ok(models) = object_field(&mut config, "models") {
                    models.entry("mode".to_string()).or_insert_with(|| json!("merge"));
                    if let Ok(providers) = object_field(&mut config["models"], "providers") {
                        providers.insert("lumagate".into(), build_openclaw_provider(&groups, base));
                        let _ = crate::config::atomic_write_private(&path, &json_bytes(&config));
                        updated.push(Agent::Openclaw);
                    }
                }
            }
        }
    }

    if settings.hermes && status.hermes.connected {
        let path = home.join(".hermes/config.yaml");
        if let Ok(Some(before)) = read(&path) {
            if let Ok(mut root) = serde_yaml::from_slice::<serde_yaml::Value>(&before) {
                if let Some(map) = root.as_mapping_mut() {
                    let key = serde_yaml::Value::String("custom_providers".into());
                    let entry = map
                        .entry(key)
                        .or_insert_with(|| serde_yaml::Value::Sequence(vec![]));
                    if let Some(seq) = entry.as_sequence_mut() {
                        seq.retain(|item| {
                            item.get("name").and_then(|v| v.as_str()) != Some("lumagate")
                        });
                        seq.push(build_hermes_provider(&groups, base));
                        if let Ok(text) = serde_yaml::to_string(&root) {
                            let _ = crate::config::atomic_write_private(&path, text.as_bytes());
                            updated.push(Agent::Hermes);
                        }
                    }
                }
            }
        }
    }

    if settings.droid && status.droid.connected {
        let path = home.join(".factory/settings.json");
        if let Ok(Some(before)) = read(&path) {
            if let Ok(mut config) = json_config(&Some(before)) {
                let mut existing: Vec<Value> = config
                    .get("customModels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| {
                        !item
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.starts_with("lumagate-"))
                    })
                    .collect();
                existing.extend(build_droid_custom_models(&groups, base));
                config["customModels"] = Value::Array(existing);
                let _ = crate::config::atomic_write_private(&path, &json_bytes(&config));
                updated.push(Agent::Droid);
            }
        }
    }

    Ok(updated)
}

/// 更新 `~/.codex/config.toml` 的默认模型字段 `model = "<slug>"`。
pub fn set_codex_default_model(home: &Path, model_id: &str) -> Result<(), String> {
    let model_id = model_id.trim();
    if model_id.is_empty() || model_id.len() > 200 || model_id.chars().any(char::is_control) {
        return Err("无效的模型 ID".into());
    }
    let path = home.join(".codex/config.toml");
    let before = read(&path)?;
    let text = std::str::from_utf8(before.as_deref().unwrap_or(b""))
        .map_err(|_| "Codex 配置必须是 UTF-8")?;
    let mut config: toml_edit::DocumentMut =
        text.parse().map_err(|_| "Codex TOML 无效，未修改")?;
    config["model"] = toml_edit::value(model_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::config::atomic_write_private(&path, config.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

/// 断开 Codex 与 LumaGate 的默认连接并还原先前保存的 `openai_base_url` 与 `model_catalog_json`。
pub fn disconnect_codex(home: &Path) -> Result<String, String> {
    let path = home.join(".codex/config.toml");
    let before = read(&path)?;
    let text = std::str::from_utf8(before.as_deref().unwrap_or(b""))
        .map_err(|_| "Codex 配置必须是 UTF-8")?;
    let mut config: toml_edit::DocumentMut =
        text.parse().map_err(|_| "Codex TOML 无效，未修改")?;
    let restore = fs::read(restore_state_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<CodexRestoreState>(&b).ok());

    if let Some(prev) = restore {
        match prev.previous_openai_base_url {
            Some(url) => config["openai_base_url"] = toml_edit::value(url),
            None => {
                config.as_table_mut().remove("openai_base_url");
            }
        }
        match prev.previous_model_catalog_json {
            Some(cat) => config["model_catalog_json"] = toml_edit::value(cat),
            None => {
                config.as_table_mut().remove("model_catalog_json");
            }
        }
        let _ = fs::remove_file(restore_state_path());
    } else {
        if config
            .get("openai_base_url")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|u| u.contains("127.0.0.1:") && u.ends_with("/backend-api/codex"))
        {
            config.as_table_mut().remove("openai_base_url");
        }
        if config
            .get("model_catalog_json")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|p| p.ends_with(CODEX_CATALOG_FILE))
        {
            config.as_table_mut().remove("model_catalog_json");
        }
    }
    crate::config::atomic_write_private(&path, config.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok("已断开 Codex 与 LumaGate 的连接并恢复原配置".into())
}

pub struct FileChange {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    summary: Value,
}
pub struct Plan {
    pub id: String,
    pub revision: u64,
    pub agent: Agent,
    pub created: Instant,
    files: Vec<FileChange>,
    codex_restore: Option<CodexRestoreState>,
    pub note: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub revision: u64,
    pub files: Vec<FilePreview>,
    pub note: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub path: String,
    pub before_hash: Option<String>,
    pub changes: Value,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub path: String,
    pub backup: Option<String>,
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    reject_symlinks(path)?;
    match fs::metadata(path) {
        Ok(meta) if !meta.is_file() || meta.len() > 2 * 1024 * 1024 => {
            Err("配置文件不是普通文件或超过 2 MiB".into())
        }
        Ok(_) => fs::read(path).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn reject_symlinks(path: &Path) -> Result<(), String> {
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("拒绝写入符号链接：{}", part.display()))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
fn json_config(bytes: &Option<Vec<u8>>) -> Result<Value, String> {
    let value = match bytes {
        Some(bytes) => serde_json::from_slice(bytes).map_err(|_| "配置 JSON 无效，未修改")?,
        None => json!({}),
    };
    if !value.is_object() {
        return Err("配置必须是 JSON 对象".into());
    }
    Ok(value)
}
fn object_field<'a>(
    value: &'a mut Value,
    field: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, String> {
    if value.get(field).is_none() {
        value[field] = json!({});
    }
    value[field]
        .as_object_mut()
        .ok_or_else(|| format!("现有 {field} 不是对象，拒绝覆盖"))
}
fn json_bytes(value: &Value) -> Vec<u8> {
    format!(
        "{}\n",
        serde_json::to_string_pretty(value).expect("JSON 可序列化")
    )
    .into_bytes()
}

impl Plan {
    pub fn build(home: &Path, doc: &Document, agent: Agent, base: &str) -> Result<Self, String> {
        let groups: Vec<_> = doc
            .groups()
            .into_iter()
            .filter(|group| !group.blocked)
            .collect();
        if groups.is_empty() {
            return Err("没有可同步的启用模型，请先开启至少一个模型".into());
        }
        let mut files = vec![];
        let mut codex_restore = None;
        let note;
        match agent {
            Agent::Pi => {
                let path = home.join(".pi/agent/models.json");
                let before = read(&path)?;
                let mut config = json_config(&before)?;
                let providers = object_field(&mut config, "providers")?;
                let mut projection = serde_json::Map::new();
                for (format, api) in [
                    (Protocol::Anthropic, "anthropic-messages"),
                    (Protocol::OpenaiChat, "openai-completions"),
                    (Protocol::OpenaiResponses, "openai-responses"),
                ] {
                    let models: Vec<Value> = groups
                        .iter()
                        .filter(|g| g.protocol == format)
                        .map(|g| super::metadata::pi_model(&g.model, &g.public_id))
                        .collect();
                    let key = format!("cc-switch-manual-{}", format.format());
                    if models.is_empty() {
                        providers.remove(&key);
                        continue;
                    }
                    let value = json!({"baseUrl": format!("{base}/v1"), "api": api, "apiKey": super::LOCAL_TOKEN, "models": models});
                    providers.insert(key.clone(), value.clone());
                    projection.insert(key, value);
                }
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&config),
                    summary: json!({"providers": projection}),
                });
                note = "合并 LumaGate 模型目录到 Pi，不改动默认模型设置。".into();
            }
            Agent::Claude => {
                let path = home.join(".claude/settings.json");
                let before = read(&path)?;
                let mut config = json_config(&before)?;
                let fields =
                    json!({"ANTHROPIC_BASE_URL": base, "ANTHROPIC_AUTH_TOKEN": super::LOCAL_TOKEN});
                let env = object_field(&mut config, "env")?;
                for (key, value) in fields.as_object().unwrap() {
                    env.insert(key.clone(), value.clone());
                }
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&config),
                    summary: json!({"env": fields}),
                });
                let path = home.join(".claude/cc-switch-manual-models.json");
                let before = read(&path)?;
                let list = json!({
                    "note": "辅助清单，在 Claude Code 中使用 /model <id> 切换模型",
                    "models": groups.iter().map(|g| json!({
                        "id": g.public_id,
                        "routeId": g.id,
                        "name": g.model.name.as_deref().unwrap_or(&g.display_name),
                        "contextWindow": g.model.context_window,
                        "capabilities": super::metadata::capability_details(&g.model)
                    })).collect::<Vec<_>>()
                });
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&list),
                    summary: list,
                });
                note = "接入 Claude Code，保留默认模型与 MCP/Skills/Prompts。".into();
            }
            Agent::Codex => {
                let path = home.join(".codex/config.toml");
                let before = read(&path)?;
                let text = std::str::from_utf8(before.as_deref().unwrap_or(b""))
                    .map_err(|_| "Codex 配置必须是 UTF-8")?;
                let mut config: toml_edit::DocumentMut =
                    text.parse().map_err(|_| "Codex TOML 无效，未修改")?;
                let catalog_path = home.join(".codex").join(CODEX_CATALOG_FILE);
                let catalog_str = clean_path_str(&catalog_path);
                let codex_base = format!("{}/backend-api/codex", base.trim_end_matches('/'));

                let prev_base = config
                    .get("openai_base_url")
                    .and_then(toml_edit::Item::as_str)
                    .map(str::to_string);
                let prev_catalog = config
                    .get("model_catalog_json")
                    .and_then(toml_edit::Item::as_str)
                    .map(str::to_string);
                let taken_over = prev_catalog
                    .as_ref()
                    .filter(|existing| existing.as_str() != catalog_str)
                    .cloned();
                let taken_over_base = prev_base
                    .as_ref()
                    .filter(|existing| existing.trim_end_matches('/') != codex_base)
                    .cloned();

                if taken_over.is_some() || taken_over_base.is_some() || !restore_state_path().is_file() {
                    if !prev_catalog
                        .as_deref()
                        .is_some_and(|p| p.ends_with(CODEX_CATALOG_FILE))
                    {
                        codex_restore = Some(CodexRestoreState {
                            previous_openai_base_url: prev_base,
                            previous_model_catalog_json: prev_catalog,
                        });
                    }
                }

                if let Some(existing) = config.get("model_providers") {
                    if !existing.is_table() {
                        return Err("model_providers 不是 TOML 表，拒绝覆盖".into());
                    }
                }
                if config.get("model_providers").is_none() {
                    config["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let mut provider = toml_edit::Table::new();
                provider["name"] = toml_edit::value("LumaGate");
                provider["base_url"] = toml_edit::value(format!("{base}/v1"));
                provider["wire_api"] = toml_edit::value("responses");
                provider["experimental_bearer_token"] = toml_edit::value(super::LOCAL_TOKEN);
                config["model_providers"][CODEX_PROFILE] = toml_edit::Item::Table(provider);

                if let Some(existing) = config.get("profiles") {
                    if !existing.is_table() {
                        return Err("profiles 不是 TOML 表，拒绝覆盖".into());
                    }
                }
                if config.get("profiles").is_none() {
                    config["profiles"] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let mut profile = toml_edit::Table::new();
                profile["model_provider"] = toml_edit::value(CODEX_PROFILE);
                config["profiles"][CODEX_PROFILE] = toml_edit::Item::Table(profile);

                // 同时接入 Codex 桌面端与 CLI 默认路由 + 模型目录，修改 LumaGate 模型后自动更新。
                config["openai_base_url"] = toml_edit::value(&codex_base);
                config["model_catalog_json"] = toml_edit::value(&catalog_str);

                let catalog = build_codex_catalog_value(doc)?;
                let catalog_before = read(&catalog_path)?;
                files.push(FileChange {
                    path: catalog_path,
                    before: catalog_before,
                    after: json_bytes(&catalog),
                    summary: json!({"models": catalog["models"]}),
                });
                let mut changes = json!({
                    "openai_base_url": codex_base,
                    "model_catalog_json": catalog_str,
                    "model_providers.cc_switch_manual": {"base_url": format!("{base}/v1"), "wire_api": "responses"},
                    "profiles.cc_switch_manual.model_provider": CODEX_PROFILE
                });
                if let Some(existing) = &taken_over {
                    changes["model_catalog_json_原值"] = json!(existing);
                }
                if let Some(existing) = &taken_over_base {
                    changes["openai_base_url_原值"] = json!(existing);
                }
                files.push(FileChange {
                    path,
                    before,
                    after: config.to_string().into_bytes(),
                    summary: changes,
                });
                note = match (&taken_over, &taken_over_base) {
                    (Some(cat), _) => format!(
                        "将 Codex 桌面端与 CLI 接入 LumaGate，并把模型目录从 {cat} 切换为 LumaGate 托管目录（自动备份原配置，可随时一键还原）。接入后在 LumaGate 启停模型会自动更新到 Codex。"
                    ),
                    (None, Some(url)) => format!(
                        "将 Codex 桌面端与 CLI 接入 LumaGate（原地址 {url} 已备份，可一键还原）。接入后启停模型会自动更新到 Codex。"
                    ),
                    (None, None) => {
                        "将 Codex 桌面端与 CLI 接入 LumaGate（保留默认模型、MCP、Skills 与插件）。接入后在 LumaGate 启停模型会自动更新到 Codex。".into()
                    }
                };
            }
            Agent::Opencode => {
                let path = opencode_config_path(home);
                let before = read(&path)?;
                let mut config = match &before {
                    Some(bytes) if !bytes.is_empty() => parse_json5(bytes)?,
                    _ => json!({ "$schema": "https://opencode.ai/config.json" }),
                };
                let providers = object_field(&mut config, "provider")?;
                providers.remove("lumagate");
                providers.remove("lumagate-anthropic");
                let built = build_opencode_providers(&groups, base);
                for (k, v) in &built {
                    providers.insert(k.clone(), v.clone());
                }
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&config),
                    summary: json!({ "provider": built }),
                });
                note = "将 LumaGate 模型写入 OpenCode provider.lumagate，保留现有 MCP 与其他配置。".into();
            }
            Agent::Openclaw => {
                let path = home.join(".openclaw/openclaw.json");
                let before = read(&path)?;
                let mut config = match &before {
                    Some(bytes) if !bytes.is_empty() => parse_json5(bytes)?,
                    _ => json!({ "models": { "mode": "merge", "providers": {} } }),
                };
                let models = object_field(&mut config, "models")?;
                models.entry("mode".to_string()).or_insert_with(|| json!("merge"));
                let providers = object_field(&mut config["models"], "providers")?;
                let built = build_openclaw_provider(&groups, base);
                providers.insert("lumagate".into(), built.clone());
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&config),
                    summary: json!({ "models.providers.lumagate": built }),
                });
                note = "将 LumaGate 模型合并写入 OpenClaw，保留现有 MCP 与其他提供商。".into();
            }
            Agent::Hermes => {
                let path = home.join(".hermes/config.yaml");
                let before = read(&path)?;
                let mut root: serde_yaml::Value = match &before {
                    Some(bytes) if !bytes.is_empty() => {
                        serde_yaml::from_slice(bytes).map_err(|e| format!("Hermes YAML 解析失败: {e}"))?
                    }
                    _ => serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                };
                let map = root
                    .as_mapping_mut()
                    .ok_or("Hermes config.yaml 根节点必须是映射")?;
                let key = serde_yaml::Value::String("custom_providers".into());
                let entry = map
                    .entry(key)
                    .or_insert_with(|| serde_yaml::Value::Sequence(vec![]));
                let seq = entry
                    .as_sequence_mut()
                    .ok_or("Hermes custom_providers 不是列表，拒绝覆盖")?;
                seq.retain(|item| item.get("name").and_then(|v| v.as_str()) != Some("lumagate"));
                let built = build_hermes_provider(&groups, base);
                seq.push(built);
                let after = serde_yaml::to_string(&root)
                    .map_err(|e| format!("序列化 Hermes YAML 失败: {e}"))?
                    .into_bytes();
                files.push(FileChange {
                    path,
                    before,
                    after,
                    summary: json!({
                        "custom_providers[name=lumagate]": {
                            "base_url": format!("{}/v1", base.trim_end_matches('/')),
                            "models": groups.iter().map(|g| &g.public_id).collect::<Vec<_>>()
                        }
                    }),
                });
                note = "在 Hermes config.yaml 的 custom_providers 中写入 lumagate，不改动当前默认模型与现有密钥。".into();
            }
            Agent::Droid => {
                let path = home.join(".factory/settings.json");
                let before = read(&path)?;
                let mut config = json_config(&before)?;
                let mut existing: Vec<Value> = config
                    .get("customModels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| {
                        !item
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.starts_with("lumagate-"))
                    })
                    .collect();
                let built = build_droid_custom_models(&groups, base);
                existing.extend(built.clone());
                config["customModels"] = Value::Array(existing);
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&config),
                    summary: json!({ "customModels": built }),
                });
                note = "将 LumaGate 模型写入 Factory Droid customModels，保留现有 hooks 与自定义模型。".into();
            }
        }
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            revision: doc.revision,
            agent,
            created: Instant::now(),
            files,
            codex_restore,
            note,
        })
    }
    pub fn preview(&self) -> Preview {
        Preview {
            id: self.id.clone(),
            revision: self.revision,
            note: self.note.clone(),
            files: self
                .files
                .iter()
                .map(|f| FilePreview {
                    path: f.path.display().to_string(),
                    before_hash: f.before.as_ref().map(|v| hash(&String::from_utf8_lossy(v))),
                    changes: f.summary.clone(),
                })
                .collect(),
        }
    }
    pub fn apply(self, revision: u64) -> Result<Vec<Applied>, String> {
        if self.created.elapsed().as_secs() > 600 || self.revision != revision {
            return Err("预览已过期或模型目录已变更，请重新预览".into());
        }
        for file in &self.files {
            if read(&file.path)? != file.before {
                return Err(format!("文件已被修改，未写入：{}", file.path.display()));
            }
        }
        if let Some(restore) = &self.codex_restore {
            let rp = restore_state_path();
            if let Some(parent) = rp.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(bytes) = serde_json::to_vec_pretty(restore) {
                let _ = crate::config::atomic_write_private(&rp, &bytes);
            }
        }
        let mut applied = vec![];
        for file in &self.files {
            let result = (|| -> Result<Applied, String> {
                if read(&file.path)? != file.before {
                    return Err("文件发生并发修改".into());
                }
                let parent = file.path.parent().ok_or("无效路径")?;
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                reject_symlinks(&file.path)?;
                let backup = if let Some(before) = &file.before {
                    let path = parent.join(format!(
                        "{}.manual-{}.bak",
                        file.path.file_name().unwrap().to_string_lossy(),
                        self.id
                    ));
                    crate::config::atomic_write_private(&path, before)
                        .map_err(|e| e.to_string())?;
                    Some(path.display().to_string())
                } else {
                    None
                };
                if read(&file.path)? != file.before {
                    return Err("备份期间原文件已修改，拒绝覆盖".into());
                }
                crate::config::atomic_write_private(&file.path, &file.after)
                    .map_err(|e| e.to_string())?;
                Ok(Applied {
                    path: file.path.display().to_string(),
                    backup,
                })
            })();
            match result {
                Ok(result) => applied.push(result),
                Err(error) => {
                    return Err(format!(
                        "同步未完成：{}；已写入 {} 个文件：{}。备份保留在原目录，不自动回滚。",
                        error,
                        applied.len(),
                        applied
                            .iter()
                            .map(|f| f.path.clone())
                            .collect::<Vec<_>>()
                            .join("、")
                    ))
                }
            }
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        serde_json::from_value(json!({"revision":0,"providers":[{"id":"a","name":"A","baseUrl":"http://127.0.0.1:1234/v1","keyEnv":"","protocol":"openai_chat","models":[{"id":"demo"}]}],"policies":{},"tests":{}})).unwrap()
    }
    #[test]
    fn preview_is_read_only_and_confirmation_preserves_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let path = home.join(".pi/agent/models.json");
        fs::write(&path, br#"{"providers":{"custom":{"baseUrl":"keep"}}}"#).unwrap();
        let before = fs::read(&path).unwrap();
        let plan = Plan::build(&home, &document(), Agent::Pi, "http://127.0.0.1:15722").unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        let applied = plan.apply(0).unwrap();
        assert!(applied[0].backup.is_some());
        let after: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["providers"]["custom"]["baseUrl"], "keep");
    }
    #[test]
    fn refuses_stale_files_and_stale_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let plan =
            Plan::build(&home, &document(), Agent::Claude, "http://127.0.0.1:15722").unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/settings.json"), b"{}").unwrap();
        assert!(plan.apply(0).is_err());
        assert!(!home.join(".claude/cc-switch-manual-models.json").exists());
        let plan = Plan::build(&home, &document(), Agent::Pi, "http://127.0.0.1:15722").unwrap();
        assert!(plan.apply(1).is_err());
        assert!(!home.join(".pi").exists());
    }
    #[test]
    fn claude_keeps_model_and_unrelated_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        let path = home.join(".claude/settings.json");
        fs::write(
            &path,
            br#"{"model":"keep-model","env":{"CUSTOM":"keep"},"permissions":{"allow":[]}}"#,
        )
        .unwrap();
        Plan::build(&home, &document(), Agent::Claude, "http://127.0.0.1:15722")
            .unwrap()
            .apply(0)
            .unwrap();
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["model"], "keep-model");
        assert_eq!(value["env"]["CUSTOM"], "keep");
        assert!(value.get("permissions").is_some());
    }
    #[test]
    fn codex_keeps_defaults_and_uses_clean_public_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let path = home.join(".codex/config.toml");
        fs::write(&path, "# preserve comment\nmodel = \"existing\"\nmodel_provider = \"existing_provider\"\n[mcp_servers.custom]\ncommand = \"keep\"\n").unwrap();
        let plan = Plan::build(&home, &document(), Agent::Codex, "http://127.0.0.1:15722").unwrap();
        assert!(!home.join(".codex/cc-switch-manual-models.json").exists());
        plan.apply(0).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let config: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(config["model"].as_str(), Some("existing"));
        assert_eq!(config["model_provider"].as_str(), Some("existing_provider"));
        assert_eq!(
            config["openai_base_url"].as_str(),
            Some("http://127.0.0.1:15722/backend-api/codex")
        );
        assert_eq!(
            config["mcp_servers"]["custom"]["command"].as_str(),
            Some("keep")
        );
        assert!(text.contains("# preserve comment"));
        let catalog: Value = serde_json::from_slice(
            &fs::read(home.join(".codex/cc-switch-manual-models.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["models"][0]["slug"].as_str(), Some("demo"));
    }

    #[test]
    fn auto_sync_refreshes_codex_catalog_without_touching_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let mut doc = document();
        Plan::build(&home, &doc, Agent::Codex, "http://127.0.0.1:15722")
            .unwrap()
            .apply(0)
            .unwrap();
        let config_before = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        // Add a second model and trigger auto-sync
        doc.providers[0].models.push(super::super::catalog::Model {
            id: "gpt-6-astra".into(),
            enabled: true,
            ..Default::default()
        });
        let settings = AutoSyncSettings {
            codex: true,
            ..Default::default()
        };
        let updated = sync_managed_catalogs(&home, &doc, "http://127.0.0.1:15722", &settings).unwrap();
        assert_eq!(updated, vec![Agent::Codex]);
        assert_eq!(
            fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
            config_before
        );
        let catalog: Value = serde_json::from_slice(
            &fs::read(home.join(".codex/cc-switch-manual-models.json")).unwrap(),
        )
        .unwrap();
        let slugs: Vec<_> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["slug"].as_str())
            .collect();
        assert!(slugs.contains(&"demo"));
        assert!(slugs.contains(&"gpt-6-astra"));
    }

    #[test]
    fn preview_does_not_expose_existing_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            br#"{"env":{"ANTHROPIC_AUTH_TOKEN":"fixture-not-a-real-secret"}}"#,
        )
        .unwrap();
        let plan =
            Plan::build(&home, &document(), Agent::Claude, "http://127.0.0.1:15722").unwrap();
        assert!(!serde_json::to_string(&plan.preview())
            .unwrap()
            .contains("fixture-not-a-real-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(target.path(), home.join(".claude")).unwrap();
        assert!(Plan::build(&home, &document(), Agent::Claude, "http://127.0.0.1:15722").is_err());
    }

    #[test]
    fn foreign_model_catalog_is_surfaced_for_overwrite_confirmation() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let foreign = home.join(".codex/other-tool-models.json");
        fs::write(
            home.join(".codex/config.toml"),
            format!("model_catalog_json = \"{}\"\n", foreign.display()),
        )
        .unwrap();
        let plan = Plan::build(&home, &document(), Agent::Codex, "http://127.0.0.1:15722").unwrap();
        let preview = plan.preview();
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            serialized.contains(&foreign.display().to_string()),
            "预览必须点名被改写的原目录，否则用户无法判断是否覆盖：{serialized}"
        );
        assert!(!home.join(".codex/cc-switch-manual-models.json").exists());
        assert!(fs::read_to_string(home.join(".codex/config.toml"))
            .unwrap()
            .contains("other-tool-models.json"));
    }

    #[test]
    fn codex_gateway_launch_uses_profile_only_once_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        assert!(!gateway_profile_ready(&home, Agent::Codex));
        assert_eq!(Agent::Codex.command(), "codex");
        assert_eq!(Agent::Codex.gateway_command(), "codex --profile cc_switch_manual");
        assert!(gateway_profile_ready(&home, Agent::Pi));
        assert!(gateway_profile_ready(&home, Agent::Claude));
        assert_eq!(Agent::Pi.gateway_command(), Agent::Pi.command());

        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(home.join(".codex/config.toml"), "[profiles.other]\nmodel_provider = \"x\"\n")
            .unwrap();
        assert!(
            !gateway_profile_ready(&home, Agent::Codex),
            "别的 profile 不算网关接入"
        );
        Plan::build(&home, &document(), Agent::Codex, "http://127.0.0.1:15722")
            .unwrap()
            .apply(0)
            .unwrap();
        assert!(
            gateway_profile_ready(&home, Agent::Codex),
            "接入后启动必须自动带网关 profile"
        );
    }

    #[test]
    fn syncs_opencode_openclaw_hermes_and_droid_preserving_existing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let base = "http://127.0.0.1:15722";
        let doc = document();

        // OpenCode (.jsonc with existing mcp)
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        fs::write(
            home.join(".config/opencode/opencode.jsonc"),
            br#"{"mcp":{"keep":{"enabled":true}}}"#,
        )
        .unwrap();
        Plan::build(&home, &doc, Agent::Opencode, base)
            .unwrap()
            .apply(0)
            .unwrap();
        let oc: Value =
            serde_json::from_slice(&fs::read(home.join(".config/opencode/opencode.jsonc")).unwrap())
                .unwrap();
        assert_eq!(oc["mcp"]["keep"]["enabled"], true);
        assert_eq!(
            oc["provider"]["lumagate"]["options"]["baseURL"],
            "http://127.0.0.1:15722/v1"
        );

        // OpenClaw
        fs::create_dir_all(home.join(".openclaw")).unwrap();
        fs::write(
            home.join(".openclaw/openclaw.json"),
            br#"{"mcp":{"servers":{"keep":{}}}}"#,
        )
        .unwrap();
        Plan::build(&home, &doc, Agent::Openclaw, base)
            .unwrap()
            .apply(0)
            .unwrap();
        let claw: Value =
            serde_json::from_slice(&fs::read(home.join(".openclaw/openclaw.json")).unwrap())
                .unwrap();
        assert!(claw["mcp"]["servers"]["keep"].is_object());
        assert_eq!(
            claw["models"]["providers"]["lumagate"]["baseUrl"],
            "http://127.0.0.1:15722/v1"
        );

        // Hermes
        fs::create_dir_all(home.join(".hermes")).unwrap();
        fs::write(
            home.join(".hermes/config.yaml"),
            "model:\n  default: existing\ncustom_providers:\n  - name: other\n    base_url: https://other.example/v1\n",
        )
        .unwrap();
        Plan::build(&home, &doc, Agent::Hermes, base)
            .unwrap()
            .apply(0)
            .unwrap();
        let hermes_val: serde_yaml::Value =
            serde_yaml::from_slice(&fs::read(home.join(".hermes/config.yaml")).unwrap()).unwrap();
        assert_eq!(hermes_val["model"]["default"].as_str(), Some("existing"));
        let list = hermes_val["custom_providers"].as_sequence().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|p| p["name"].as_str() == Some("lumagate")));

        // Droid
        fs::create_dir_all(home.join(".factory")).unwrap();
        fs::write(
            home.join(".factory/settings.json"),
            br#"{"hooks":{"keep":true},"customModels":[{"id":"user-custom","model":"x"}]}"#,
        )
        .unwrap();
        Plan::build(&home, &doc, Agent::Droid, base)
            .unwrap()
            .apply(0)
            .unwrap();
        let droid: Value =
            serde_json::from_slice(&fs::read(home.join(".factory/settings.json")).unwrap())
                .unwrap();
        assert_eq!(droid["hooks"]["keep"], true);
        let models = droid["customModels"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert!(models.iter().any(|m| m["id"] == "user-custom"));
        assert!(models.iter().any(|m| m["id"] == "lumagate-demo"));

        // Inspect verifies all 4 show connected + installed
        let status = inspect_agents(&home, base, &AutoSyncSettings::default());
        assert!(status.opencode.connected && status.opencode.installed);
        assert!(status.openclaw.connected && status.openclaw.installed);
        assert!(status.hermes.connected && status.hermes.installed);
        assert!(status.droid.connected && status.droid.installed);
    }
}
