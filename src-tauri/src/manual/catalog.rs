use crate::{database::Database, provider::Provider};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const DOCUMENT_KEY: &str = "manual_gateway_v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
}

impl Protocol {
    pub fn format(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenaiChat => "openai_chat",
            Self::OpenaiResponses => "openai_responses",
        }
    }
    pub fn endpoint(&self) -> &'static str {
        match self {
            Self::Anthropic => "messages",
            Self::OpenaiChat => "chat/completions",
            Self::OpenaiResponses => "responses",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub protocol_override: Option<Protocol>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub thinking_types: Option<Vec<String>>,
    #[serde(default)]
    pub reasoning_summaries: Option<bool>,
    #[serde(default)]
    pub reasoning_levels: Option<Vec<String>>,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub input_modalities: Option<Vec<String>>,
    #[serde(default)]
    pub output_modalities: Option<Vec<String>>,
    #[serde(default)]
    pub supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub tools: Option<bool>,
    #[serde(default)]
    pub context_window: Option<u64>,
    // Derived provenance: neither Provider data nor a saved document can assert a user rule.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub limits_source: Option<super::limits::LimitSource>,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub key_env: String,
    #[serde(default)]
    pub key_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot: Option<super::copilot::Binding>,
    pub protocol: Protocol,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<Model>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Balance {
    #[default]
    RoundRobin,
    Priority,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    #[serde(default)]
    pub balance: Balance,
    #[serde(default = "yes")]
    pub failover: bool,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            balance: Balance::RoundRobin,
            failover: true,
            fallbacks: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub success: bool,
    pub latency_ms: u64,
    pub checked_at: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub revision: u64,
    pub providers: Vec<Source>,
    pub policies: BTreeMap<String, Policy>,
    pub tests: BTreeMap<String, TestResult>,
    #[serde(default)]
    pub blocked_models: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub public_id: String,
    pub model: Model,
    pub protocol: Protocol,
    pub protocol_mode: &'static str,
    pub automatic_protocol: Protocol,
    pub legacy_ids: Vec<String>,
    pub routing_error: Option<String>,
    pub blocked: bool,
    pub provider_ids: Vec<String>,
    pub provider_names: Vec<String>,
    pub policy: Policy,
}

pub fn test_key(group_id: &str, provider_id: &str) -> String {
    // 与前端 JSON.stringify([groupId, providerId]) 一致，不把来源结果混入旧的组测试。
    serde_json::to_string(&(group_id, provider_id)).expect("字符串对可序列化")
}

pub fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub fn signature(protocol: &Protocol, model: &Model) -> String {
    let legacy = format!(
        "{}|{}|{:?}|{:?}",
        protocol.format(),
        model.image,
        model.tools,
        model.context_window
    );
    let details = super::metadata::capability_details(model);
    // 未声明任何扩展能力的旧目录保留 ID；原始元数据和显示名称不参与能力分组。
    if details.as_object().unwrap().values().all(Value::is_null) {
        legacy
    } else {
        format!("{legacy}|{details}")
    }
}

pub fn group_id(protocol: &Protocol, model: &Model) -> String {
    // 能力字段和部署数量会变化，不能因此拆散同模型或改变客户端访问 ID。
    format!(
        "{}--{}",
        model.id,
        &hash(&format!("model-route|{}|{}", model.id, protocol.format()))[..10]
    )
}

fn legacy_group_id(protocol: &Protocol, model: &Model) -> String {
    format!(
        "{}--{}",
        model.id,
        &hash(&format!("{}|{}", model.id, signature(protocol, model)))[..10]
    )
}

pub fn resolve_group<'a>(groups: &'a [Group], id: &str) -> Option<&'a Group> {
    if let Some(group) = groups.iter().find(|g| g.id == id) {
        return Some(group);
    }
    let matches: Vec<_> = groups
        .iter()
        .filter(|g| g.model.id == id || g.legacy_ids.iter().any(|alias| alias == id))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

fn public_name_index(groups: &[Group]) -> super::names::Names<'_> {
    super::names::Names::new(groups.iter().map(|group| super::names::RouteName {
        id: &group.id,
        model: &group.model.id,
        legacy: &group.legacy_ids,
    }))
}

/// External model names may have safe convenience aliases. Internal policies still
/// use resolve_group so adding a public alias cannot change persisted fallback graphs.
pub fn resolve_public_group<'a>(groups: &'a [Group], name: &str) -> Option<&'a Group> {
    public_name_index(groups)
        .resolve(name)
        .and_then(|index| groups.get(index))
}

pub fn deployment_version(source: &Source, model: &Model) -> String {
    hash(&format!(
        "{}|{}",
        super::keys::credential_version(source),
        super::endpoints::protocol(model).format()
    ))
}

impl Document {
    pub fn load(db: &Database) -> Result<Self, String> {
        match db.get_setting(DOCUMENT_KEY).map_err(|e| e.to_string())? {
            Some(text) => serde_json::from_str(&text).map_err(|e| format!("模型目录损坏：{e}")),
            None => Ok(Self::default()),
        }
    }
    pub fn save(&mut self, db: &Database) -> Result<(), String> {
        self.revision += 1;
        db.set_setting(
            DOCUMENT_KEY,
            &serde_json::to_string(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn groups(&self) -> Vec<Group> {
        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        for source in &self.providers {
            for model in &source.models {
                // 屏蔽记录仍用于识别后备引用，但不会进入公开目录或路由。
                if !self.blocked_models.contains(&model.id) && (!source.enabled || !model.enabled) {
                    continue;
                }
                let protocol = super::endpoints::protocol(model);
                let id = group_id(&protocol, model);
                let aliases = [
                    legacy_group_id(&protocol, model),
                    legacy_group_id(&source.protocol, model),
                    group_id(&source.protocol, model),
                ];
                let mode = if model.protocol_override.is_some() {
                    "manual"
                } else {
                    "auto"
                };
                let group = groups.entry(id.clone()).or_insert_with(|| Group {
                    id,
                    public_id: String::new(),
                    model: model.clone(),
                    protocol,
                    protocol_mode: mode,
                    automatic_protocol: super::endpoints::automatic_protocol(&model.id),
                    legacy_ids: vec![],
                    routing_error: None,
                    blocked: self.blocked_models.contains(&model.id),
                    provider_ids: vec![],
                    provider_names: vec![],
                    policy: Policy::default(),
                });
                if group.protocol_mode != mode {
                    group.protocol_mode = "mixed";
                }
                for alias in aliases {
                    if alias != group.id && !group.legacy_ids.contains(&alias) {
                        group.legacy_ids.push(alias);
                    }
                }
                if !group.provider_ids.contains(&source.id) {
                    if !group.provider_ids.is_empty() {
                        super::aggregation::merge(&mut group.model, model);
                    }
                    group.provider_ids.push(source.id.clone());
                    group.provider_names.push(source.name.clone());
                }
            }
        }
        let mut groups: Vec<_> = groups.into_values().collect();
        for group in &mut groups {
            // Resolve original aliases first, then apply the explicit owner-specified limits.
            super::limits::apply(&mut group.model);
            if let Some(policy) = self.policies.get(&group.id) {
                group.policy = policy.clone();
                continue;
            }
            let mut candidates: Vec<&Policy> = vec![];
            for id in &group.legacy_ids {
                if let Some(policy) = self.policies.get(id) {
                    if !candidates.contains(&policy) {
                        candidates.push(policy);
                    }
                }
            }
            match candidates.as_slice() {
                [] => {}
                [policy] => group.policy = (*policy).clone(),
                _ => {
                    group.routing_error =
                        Some("合并前的路由策略存在冲突，请为此组重新保存路由策略".into())
                }
            }
        }
        let names = public_name_index(&groups);
        let preferred: Vec<_> = (0..groups.len())
            .map(|index| names.preferred(index).to_string())
            .collect();
        for (group, public_id) in groups.iter_mut().zip(preferred) {
            group.public_id = public_id;
        }
        groups
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.providers.len() > 100 {
            return Err("最多 100 个 Provider".into());
        }
        let mut ids = HashSet::new();
        for source in &self.providers {
            if !ids.insert(&source.id) || source.id.is_empty() {
                return Err("Provider 标识重复或为空".into());
            }
            if source.name.trim().is_empty() || source.name.len() > 200 {
                return Err("Provider 名称无效".into());
            }
            validate_url(&source.base_url)?;
            if let Some(binding) = &source.copilot {
                if source.id != super::copilot::PROVIDER_ID
                    || source.base_url != "https://api.githubcopilot.com"
                    || source.key_ref.is_some()
                    || !source.key_env.is_empty()
                    || binding.account_id.parse::<u64>().is_err()
                    || uuid::Uuid::parse_str(&binding.grant_id).is_err()
                {
                    return Err("Copilot 账号绑定或连接配置无效，请通过 GitHub 登录入口连接".into());
                }
                if source.models.iter().any(|m| {
                    !m.id.starts_with("copilot/")
                        || !matches!(
                            m.protocol_override,
                            Some(
                                Protocol::OpenaiChat
                                    | Protocol::OpenaiResponses
                                    | Protocol::Anthropic
                            )
                        )
                }) {
                    return Err("Copilot 目录或 Endpoint 无效，请刷新模型".into());
                }
            } else if source.models.iter().any(|m| m.id.starts_with("copilot/")) {
                return Err("copilot/ 模型命名空间只供独立 Copilot 来源使用".into());
            }
            if let Some(reference) = &source.key_ref {
                if uuid::Uuid::parse_str(reference).is_err() {
                    return Err("密钥引用无效，请重新保存 API Key".into());
                }
                if !source.key_env.is_empty() {
                    return Err("请选择 API Key 或环境变量中的一种鉴权方式".into());
                }
            }
            if !source.key_env.is_empty()
                && (!source
                    .key_env
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || source.key_env.as_bytes()[0].is_ascii_digit())
            {
                return Err("密钥只填写环境变量名称，例如 OPENAI_API_KEY".into());
            }
            if source.models.len() > 2000 {
                return Err("单个 Provider 最多 2000 个模型".into());
            }
            let mut models = HashSet::new();
            for model in &source.models {
                if model.id.trim() != model.id
                    || model.id.is_empty()
                    || model.id.len() > 200
                    || model.id.chars().any(char::is_control)
                    || !models.insert(&model.id)
                {
                    return Err("模型标识为空、重复或包含非法字符".into());
                }
                if model.max_output_tokens == Some(0) {
                    return Err("输出上限必须大于零".into());
                }
                if model.context_window == Some(0) {
                    return Err("上下文窗口必须大于零".into());
                }
            }
        }
        let groups = self.groups();
        for group in &groups {
            if group.blocked {
                continue;
            }
            if let Some(error) = &group.routing_error {
                return Err(error.clone());
            }
            if group.policy.fallbacks.len() > 3 {
                return Err("最多设置三个后备模型".into());
            }
            for target in &group.policy.fallbacks {
                let other =
                    resolve_group(&groups, target).ok_or("后备模型已不存在、禁用或有歧义")?;
                if group.model.id.starts_with("copilot/") || other.model.id.starts_with("copilot/")
                {
                    return Err("Copilot 第一版使用独立路由，不参与跨来源或跨模型回退".into());
                }
                if other.blocked {
                    continue;
                }
                if signature(&group.protocol, &group.model)
                    != signature(&other.protocol, &other.model)
                {
                    return Err("Fallback 的协议、模态、工具和上下文能力必须一致".into());
                }
            }
            let mut visited = HashSet::new();
            Self::visit(&groups, &group.id, &mut visited)?;
        }
        Ok(())
    }
    fn visit(groups: &[Group], id: &str, path: &mut HashSet<String>) -> Result<(), String> {
        let group = resolve_group(groups, id).ok_or("后备模型已不可用")?;
        if group.blocked {
            return Ok(());
        }
        if !path.insert(group.id.clone()) {
            return Err("Fallback 不能形成循环".into());
        }
        for target in &group.policy.fallbacks {
            Self::visit(groups, target, path)?;
        }
        path.remove(&group.id);
        Ok(())
    }
}

pub fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|_| "API 地址无效")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 地址不能含凭据、查询参数或片段".into());
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none() || !(url.scheme() == "https" || (local && url.scheme() == "http")) {
        return Err("远端 API 必须使用 HTTPS；仅本机允许 HTTP".into());
    }
    Ok(())
}

pub fn credentials(source: &Source) -> Result<String, String> {
    super::keys::resolve(source, &super::keys::FileKeyStore::default())
}

pub fn endpoint(source: &Source, path: &str) -> String {
    let base = source.base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

pub fn upstream_provider(source: &Source, model: &Model, app: &str) -> Result<Provider, String> {
    if source.copilot.is_some() {
        return Err("Copilot 需要由账号凭据解析器转发".into());
    }
    upstream_provider_with_key(source, model, app, &credentials(source)?)
}
pub fn upstream_provider_with_key(
    source: &Source,
    model: &Model,
    app: &str,
    key: &str,
) -> Result<Provider, String> {
    let mut effective_source = source.clone();
    effective_source.protocol = super::endpoints::protocol(model);
    let version = deployment_version(source, model);
    let source = &effective_source;
    let base = source.base_url.trim_end_matches('/');
    let mut config = if app == "claude" {
        json!({"env": {"ANTHROPIC_BASE_URL": base, "ANTHROPIC_AUTH_TOKEN": key, "ANTHROPIC_MODEL": model.id}, "api_format": source.protocol.format()})
    } else {
        let base = if base.ends_with("/v1") {
            base.to_string()
        } else {
            format!("{base}/v1")
        };
        json!({"base_url": base, "model": model.id, "auth": {"OPENAI_API_KEY": key}, "api_format": source.protocol.format(), "config": format!("model = {}\nmodel_provider = \"manual\"\n[model_providers.manual]\nname = \"manual\"\nbase_url = {}\nwire_api = \"responses\"\n", json!(model.id), json!(base))})
    };
    // 只存在请求快照里，绝不调用上游 ProviderService 的 live 写入路径。
    config["manual_upstream_model"] = json!(model.id);
    config["manual_credential_version"] = json!(version);
    Ok(Provider::with_id(
        source.id.clone(),
        source.name.clone(),
        config,
        None,
    ))
}

pub fn public_models(doc: &Document) -> Value {
    let groups = doc.groups();
    let names = public_name_index(&groups);
    json!({"object": "list", "data": groups.iter().enumerate().filter(|(_,g)| !g.blocked).map(|(index,g)| json!({
        "id": g.public_id, "route_id": g.id, "object": "model", "owned_by": "lumagate",
        "name": g.model.name.as_deref().unwrap_or(&g.model.id), "protocol": g.protocol,
        "endpoint": format!("/v1/{}", g.protocol.endpoint()), "endpoint_source": g.protocol_mode,
        "aliases":names.aliases(index), "routing_error":g.routing_error,
        "deployments": g.provider_ids.len(),
        "deployment_tests": g.provider_ids.iter().zip(&g.provider_names).map(|(id, name)| json!({
            "provider_id":id,"provider_name":name,"result":doc.tests.get(&test_key(&g.id, id))
        })).collect::<Vec<_>>(),
        "context_window": g.model.context_window, "max_output_tokens": g.model.max_output_tokens,
        "limits_source": g.model.limits_source,
        "reasoning": g.model.reasoning, "supported_reasoning_levels": g.model.reasoning_levels,
        "thinking_types": g.model.thinking_types, "supports_reasoning_summaries": g.model.reasoning_summaries,
        "default_reasoning_level": g.model.default_reasoning_level,
        "supported_parameters": g.model.supported_parameters,
        "architecture": {"input_modalities":super::modalities::declared_input(&g.model),"output_modalities":g.model.output_modalities},
        "capabilities": {"tools":g.model.tools}
    })).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn source(id: &str) -> Source {
        Source {
            id: id.into(),
            name: id.into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            key_env: String::new(),
            key_ref: None,
            copilot: None,
            protocol: Protocol::OpenaiChat,
            enabled: true,
            models: vec![Model {
                id: "model-a".into(),
                image: false,
                tools: None,
                context_window: None,
                enabled: true,
                ..Model::default()
            }],
        }
    }
    #[test]
    fn groups_exact_models_only() {
        let mut doc = Document::default();
        doc.providers = vec![source("a"), source("b")];
        assert_eq!(doc.groups().len(), 1);
        assert_eq!(doc.groups()[0].provider_ids.len(), 2);
        doc.providers[1].models[0].id = "model-a-20260101".into();
        assert_eq!(doc.groups().len(), 2);
        doc.providers[1].models[0].id = "model-a".into();
        doc.providers[1].models[0].tools = Some(true);
        assert_eq!(doc.groups().len(), 1);
        assert_eq!(doc.groups()[0].model.tools, None);
        doc.providers[1].models[0].tools = None;
        doc.providers[1].protocol = Protocol::Anthropic;
        assert_eq!(doc.groups().len(), 1);
        doc.providers[1].models[0].protocol_override = Some(Protocol::Anthropic);
        assert_eq!(doc.groups().len(), 2);
    }
    #[test]
    fn same_model_merges_metadata_variants_and_keeps_stable_route() {
        let mut doc = Document {
            providers: vec![source("a"), source("b")],
            ..Document::default()
        };
        let first = &mut doc.providers[0].models[0];
        first.context_window = Some(1_000_000);
        first.max_output_tokens = Some(128_000);
        first.input_modalities = Some(vec!["image".into(), "text".into()]);
        first.image = true;
        first.metadata = json!({"context_length":1_000_000});
        let old_ids: Vec<_> = doc
            .providers
            .iter()
            .map(|p| {
                let m = &p.models[0];
                format!(
                    "{}--{}",
                    m.id,
                    &hash(&format!("{}|{}", m.id, signature(&Protocol::OpenaiChat, m)))[..10]
                )
            })
            .collect();
        let groups = doc.groups();
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.provider_ids, ["a", "b"]);
        assert!(group.policy.failover);
        assert_eq!(group.policy.balance, Balance::RoundRobin);
        assert_eq!(group.model.context_window, None);
        assert_eq!(
            group.model.input_modalities,
            Some(vec!["image".into(), "text".into()])
        );
        assert!(group.model.image);
        assert!(group.model.metadata.is_null());
        assert_eq!(
            doc.providers[0].models[0].metadata["context_length"],
            1_000_000
        );
        for old in &old_ids {
            assert_eq!(resolve_group(&groups, old).unwrap().id, group.id);
        }
        let stable_id = group.id.clone();
        doc.providers[1].models[0].context_window = Some(128_000);
        assert_eq!(doc.groups()[0].id, stable_id);
        assert_eq!(doc.groups()[0].model.context_window, Some(128_000));
        doc.policies.insert(
            old_ids[0].clone(),
            Policy {
                balance: Balance::Priority,
                ..Policy::default()
            },
        );
        assert_eq!(doc.groups()[0].policy.balance, Balance::Priority);
    }

    #[test]
    fn merged_model_advertises_only_common_capabilities() {
        let mut doc = Document {
            providers: vec![source("a"), source("b")],
            ..Document::default()
        };
        for source in &mut doc.providers {
            let m = &mut source.models[0];
            m.context_window = Some(256_000);
            m.max_output_tokens = Some(64_000);
            m.reasoning = Some(true);
            m.reasoning_levels = Some(vec!["high".into(), "low".into()]);
            m.default_reasoning_level = Some("high".into());
        }
        doc.providers[1].models[0].max_output_tokens = Some(32_000);
        doc.providers[1].models[0].reasoning_levels = Some(vec!["low".into()]);
        let groups = doc.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].model.max_output_tokens, Some(32_000));
        assert_eq!(groups[0].model.reasoning_levels, Some(vec!["low".into()]));
        assert_eq!(groups[0].model.default_reasoning_level, None);
        doc.providers[1].models[0].reasoning = Some(false);
        assert_eq!(doc.groups()[0].model.reasoning, Some(false));
    }

    #[test]
    fn rejects_cycles_and_remote_plaintext() {
        let mut doc = Document::default();
        doc.providers = vec![source("a")];
        let id = doc.groups()[0].id.clone();
        doc.policies.insert(
            id.clone(),
            Policy {
                fallbacks: vec![id],
                ..Policy::default()
            },
        );
        assert!(doc.validate().is_err());
        assert!(validate_url("http://example.com/v1").is_err());
        assert!(validate_url("https://example.com/v1?key=secret").is_err());
    }
}
