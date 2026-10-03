use super::catalog::{upstream_provider, Balance, Document, Group};
use crate::{
    provider::Provider,
    proxy::{provider_router::ProviderRouter, ProxyError},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, RwLock};

const AFFINITY_KEY: &str = "manual_gateway_affinity_v1";

#[derive(Default)]
pub struct ManualRoutes {
    sequence: RwLock<HashMap<String, u64>>,
    affinity: RwLock<HashMap<String, (String, String)>>,
    db: Option<Arc<crate::database::Database>>,
    restore_failed: bool,
    copilot: std::sync::OnceLock<Arc<super::copilot::Manager>>,
    session_locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl ManualRoutes {
    pub fn attach_copilot(&self, manager: Arc<super::copilot::Manager>) {
        let _ = self.copilot.set(manager);
    }
    pub fn with_db(db: Arc<crate::database::Database>) -> Self {
        let loaded = match db.get_setting(AFFINITY_KEY) {
            Ok(None) => Some(HashMap::<String, (String, String)>::new()),
            Ok(Some(text)) if text.len() <= 2 * 1024 * 1024 => {
                serde_json::from_str::<HashMap<String, (String, String)>>(&text)
                    .ok()
                    .filter(|entries| entries.len() <= 4096)
            }
            _ => None,
        };
        let restore_failed = loaded.is_none();
        Self {
            affinity: RwLock::new(loaded.unwrap_or_default()),
            db: Some(db),
            restore_failed,
            ..Self::default()
        }
    }

    fn session_key(app: &str, session: &str) -> String {
        super::catalog::hash(&format!("{app}:{session}"))
    }

    pub async fn session_guard(
        &self,
        app: &str,
        session: &str,
    ) -> Result<Option<OwnedMutexGuard<()>>, ProxyError> {
        let key = Self::session_key(app, session);
        let lock = {
            let mut locks = self
                .session_locks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                if locks.len() >= 4096 {
                    return Err(ProxyError::InvalidRequest(
                        "并行会话过多，请稍后重试".into(),
                    ));
                }
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        // Serialize selection until success is bound/preferred, including a Chat failover.
        // Otherwise a concurrent request could select against the old preference.
        Ok(Some(lock.lock_owned().await))
    }

    pub async fn bind(&self, app: &str, session: &str, provider: &str, credential_version: &str) {
        self.remember_success(app, session, provider, credential_version, false)
            .await;
    }

    pub async fn remember_success(
        &self,
        app: &str,
        session: &str,
        provider: &str,
        credential_version: &str,
        allow_rebind: bool,
    ) {
        let mut entries = self.affinity.write().await;
        let key = Self::session_key(app, session);
        if let Some((current, version)) = entries.get(&key) {
            if !allow_rebind || (current == provider && version == credential_version) {
                return;
            }
        } else if entries.len() >= 4096 {
            return;
        }
        entries.insert(key, (provider.into(), credential_version.into()));
        // 只持久化会话摘要、来源 ID 和凭据版本摘要，不保存原始会话标识或对话。
        if let Some(db) = &self.db {
            if db
                .set_setting(
                    AFFINITY_KEY,
                    &serde_json::to_string(&*entries).expect("绑定可序列化"),
                )
                .is_err()
            {
                log::warn!("无法持久化会话来源绑定；重启后未保存会话需重新建立");
                super::logs::route_note("已固定来源，但持久化失败；重启后请新建会话");
            }
        }
    }

    pub async fn select(
        &self,
        router: &ProviderRouter,
        doc: &Document,
        app: &str,
        body: &Value,
        session: Option<&str>,
    ) -> Result<Vec<Provider>, ProxyError> {
        if self.restore_failed {
            return Err(ProxyError::ConfigError(
                "会话绑定记录不可读取，已停止重新分配来源；请修复本地存储后重启".into(),
            ));
        }
        let model = body.get("model").and_then(Value::as_str).ok_or_else(|| {
            ProxyError::InvalidRequest("必须指定模型 ID，可从 /v1/models 获取".into())
        })?;
        let groups = doc.groups();
        let root = super::catalog::resolve_public_group(&groups, model).ok_or_else(|| {
            ProxyError::InvalidRequest(format!("模型未注册、已禁用或名称有歧义：{model}"))
        })?;
        if root.blocked {
            return Err(ProxyError::InvalidRequest("模型已屏蔽，请先恢复".into()));
        }
        if let Some(error) = &root.routing_error {
            return Err(ProxyError::InvalidRequest(error.clone()));
        }
        let opaque = [
            "previous_response_id",
            "conversation",
            "conversation_id",
            "cached_content",
            "cachedContent",
        ]
        .iter()
        .any(|key| body.get(*key).is_some_and(has_state))
            || body.get("input").is_some_and(contains_opaque)
            || body.get("messages").is_some_and(contains_opaque);
        // Only native, self-contained Chat messages may relax a source preference.
        // Responses/Anthropic requests remain strict even when this turn omits opaque state.
        let replayable = root.protocol == super::catalog::Protocol::OpenaiChat
            && !opaque
            && body.get("input").is_none_or(Value::is_null)
            && body
                .get("messages")
                .and_then(Value::as_array)
                .is_some_and(|messages| {
                    !messages.is_empty()
                        && messages.iter().all(|message| {
                            message.get("role").and_then(Value::as_str).is_some()
                                && (message.get("content").is_some()
                                    || message.get("tool_calls").is_some())
                        })
                });
        let allow_rebind = replayable && root.policy.failover;
        let binding = if let Some(session) = session {
            self.affinity
                .read()
                .await
                .get(&Self::session_key(app, session))
                .cloned()
        } else {
            None
        };
        super::logs::route_note(if binding.is_some() {
            "固定会话来源"
        } else if opaque {
            "拒绝：缺少旧会话绑定，未请求上游"
        } else if session.is_some() {
            "首次会话选路"
        } else {
            "无稳定会话标识，仅本次选路"
        });
        if session.is_some() && binding.is_none() && self.affinity.read().await.len() >= 4096 {
            return Err(ProxyError::InvalidRequest(
                "会话绑定已达上限，不能建立无法可靠续轮的新会话".into(),
            ));
        }
        if opaque && binding.is_none() {
            return Err(ProxyError::InvalidRequest(
                "无法确定续轮账号；未找到此会话的来源绑定，未向任何 Provider 发送请求。请新建会话，并保持 session_id 或 prompt_cache_key 稳定".into(),
            ));
        }
        if binding.is_some() {
            if let Some((provider, version)) = &binding {
                let name = doc
                    .providers
                    .iter()
                    .find(|source| &source.id == provider)
                    .map(|source| source.name.as_str())
                    .unwrap_or("来源已移除");
                super::logs::route_note(&if allow_rebind {
                    format!("优先会话来源：{name}（可重放 Chat，允许同模型回退）")
                } else {
                    format!("固定会话来源：{name}")
                });
                if !allow_rebind
                    && !doc.providers.iter().any(|source| {
                        &source.id == provider
                            && super::catalog::deployment_version(source, &root.model) == *version
                    })
                {
                    return Err(ProxyError::InvalidRequest(
                        "当前会话来源的密钥、地址或 Endpoint 已变更，请新建会话；不会自动更换账号"
                            .into(),
                    ));
                }
            }
        }
        let pinned = binding.map(|(provider, _)| provider);
        let locked = pinned.is_some() && !allow_rebind;
        let mut chain = vec![];
        let mut has_eligible_source = false;
        let mut seen = HashSet::new();
        let mut group_ids = vec![root.id.clone()];
        while let Some(id) = group_ids.first().cloned() {
            group_ids.remove(0);
            if !seen.insert(id.clone()) {
                continue;
            }
            let group = super::catalog::resolve_group(&groups, &id)
                .ok_or_else(|| ProxyError::ConfigError("后备模型已不可用".into()))?;
            if group.blocked {
                continue;
            }
            if group.routing_error.is_some() || !compatible(root, group) {
                return Err(ProxyError::InvalidRequest(
                    "Endpoint 或模型能力与 Fallback 不兼容，请检查路由设置".into(),
                ));
            }
            if !opaque && pinned.is_none() && root.policy.failover {
                group_ids.extend(group.policy.fallbacks.clone());
            }
            let mut ids: Vec<_> = group
                .provider_ids
                .iter()
                .filter(|id| {
                    doc.providers
                        .iter()
                        .find(|source| &source.id == *id)
                        .and_then(|source| {
                            source
                                .models
                                .iter()
                                .find(|model| model.id == group.model.id)
                        })
                        .is_some_and(|model| super::modalities::supports_request(model, body))
                })
                .cloned()
                .collect();
            has_eligible_source |= !ids.is_empty();
            if locked && pinned.as_ref().is_some_and(|id| !ids.contains(id)) {
                return Err(ProxyError::InvalidRequest(
                    "当前会话来源已停用或未声明支持本次模态，不能跨账号续轮；请新建会话".into(),
                ));
            }
            if let Some(id) = &pinned {
                if let Some(pos) = ids.iter().position(|v| v == id) {
                    ids.rotate_left(pos);
                }
            } else if matches!(group.policy.balance, Balance::RoundRobin) && !ids.is_empty() {
                let mut counters = self.sequence.write().await;
                counters.retain(|key, _| groups.iter().any(|g| &g.id == key));
                let counter = counters.entry(group.id.clone()).or_default();
                let offset = *counter as usize % ids.len();
                *counter = counter.wrapping_add(1);
                ids.rotate_left(offset);
            }
            if opaque || locked {
                if let Some(id) = &pinned {
                    ids.retain(|v| v == id);
                }
                ids.truncate(1);
            }
            if !root.policy.failover {
                ids.truncate(1);
            }
            for id in ids {
                let source = doc
                    .providers
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| ProxyError::ConfigError("Provider 已移除".into()))?;
                if !router.manual_available(&id, app).await {
                    continue;
                }
                let source = source.clone();
                let model = source
                    .models
                    .iter()
                    .find(|model| model.id == group.model.id)
                    .ok_or_else(|| ProxyError::ConfigError("模型部署已变化".into()))?
                    .clone();
                let app_name = app.to_string();
                let managed_copilot = source.copilot.is_some();
                let resolved = if source.copilot.is_some() {
                    let manager = self
                        .copilot
                        .get()
                        .ok_or_else(|| ProxyError::AuthError("Copilot 认证模块未连接".into()))?;
                    manager.provider(&source, &model, &app_name).await
                } else {
                    tokio::task::spawn_blocking(move || {
                        upstream_provider(&source, &model, &app_name)
                    })
                    .await
                    .map_err(|_| ProxyError::AuthError("密钥读取任务失败".into()))?
                };
                let mut candidate = match resolved {
                    Ok(candidate) => candidate,
                    Err(error) if managed_copilot => return Err(ProxyError::AuthError(error)),
                    Err(_) if root.policy.failover && !opaque && !locked => continue,
                    Err(error) => return Err(ProxyError::ConfigError(error)),
                };
                candidate.settings_config["manual_allow_rebind"] = serde_json::json!(allow_rebind);
                if !chain.iter().any(|p: &Provider| {
                    p.id == candidate.id && p.settings_config == candidate.settings_config
                }) {
                    chain.push(candidate);
                }
                // 总尝试上限与上游转发器一致，不能让 fallback 图放大请求。
                if chain.len() == 4 {
                    return Ok(chain);
                }
            }
            if opaque || locked || !root.policy.failover {
                break;
            }
        }
        if chain.is_empty() {
            if !has_eligible_source {
                return Err(ProxyError::InvalidRequest(
                    "没有启用且声明支持本次输入/输出模态的来源".into(),
                ));
            }
            return Err(ProxyError::NoAvailableProvider);
        }
        Ok(chain)
    }
}

fn has_state(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn contains_opaque(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            [
                "encrypted_content",
                "signature",
                "thought_signature",
                "thoughtSignature",
            ]
            .iter()
            .any(|key| map.get(*key).is_some_and(has_state))
                || ["content", "parts", "tool_calls", "extra_content", "google"]
                    .iter()
                    .any(|key| map.get(*key).is_some_and(contains_opaque))
        }
        Value::Array(items) => items.iter().any(contains_opaque),
        _ => false,
    }
}

pub fn compatible(a: &Group, b: &Group) -> bool {
    super::catalog::signature(&a.protocol, &a.model)
        == super::catalog::signature(&b.protocol, &b.model)
}
