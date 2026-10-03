//! Named gateway credentials. Public views never contain a secret or verification tag.
//! Files are immutable versions; the registry commit is the authorization boundary.
use super::{credential_mac, Settings};
use crate::{database::Database, manual::keys::KeyStore};
use hmac::Mac;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const REGISTRY_KEY: &str = "manual_gateway_access_keys_v1";
const MAX_KEYS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    id: String,
    name: String,
    enabled: bool,
    tag: Vec<u8>,
    secret_ref: Option<String>,
    created_at: Option<String>,
}
impl Credential {
    fn matches(&self, value: &str) -> bool {
        credential_mac(value).verify_slice(&self.tag).is_ok()
    }
    fn identity(&self) -> Identity {
        Identity {
            id: self.id.clone(),
            name: self.name.clone(),
        }
    }
}

#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub name: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub can_copy: bool,
    pub created_at: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyList {
    pub revision: u64,
    pub items: Vec<KeyView>,
}
#[derive(Serialize)]
pub struct ChangeResult {
    pub keys: KeyList,
    pub warnings: Vec<String>,
}

// Secret-bearing inputs intentionally do not implement Debug or Serialize.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Add {
        name: String,
        value: String,
    },
    Rename {
        #[serde(rename = "keyId")]
        key_id: String,
        name: String,
    },
    SetEnabled {
        #[serde(rename = "keyId")]
        key_id: String,
        enabled: bool,
    },
    Remove {
        #[serde(rename = "keyId")]
        key_id: String,
    },
    RestoreCopy {
        #[serde(rename = "keyId")]
        key_id: String,
        value: String,
    },
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    revision: u64,
    items: Vec<Credential>,
}
impl Registry {
    pub fn load(db: &Database) -> Result<Self, String> {
        let registry = match db
            .get_setting(REGISTRY_KEY)
            .map_err(|_| "无法读取访问密钥列表")?
        {
            Some(value) => {
                serde_json::from_str(&value).map_err(|_| "访问密钥列表损坏，已拒绝密钥鉴权")?
            }
            None => {
                // Read-only compatibility: do not rotate, write files or persist on startup.
                let items = Settings::load(db)?
                    .access_key_tag
                    .map(|tag| Credential {
                        id: "legacy".into(),
                        name: "旧版密钥".into(),
                        enabled: true,
                        tag,
                        secret_ref: None,
                        created_at: None,
                    })
                    .into_iter()
                    .collect();
                Self { revision: 0, items }
            }
        };
        registry.validate()?;
        Ok(registry)
    }
    fn validate(&self) -> Result<(), String> {
        if self.items.len() > MAX_KEYS {
            return Err("访问密钥数量超出上限".into());
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut tags = HashSet::new();
        let mut refs = HashSet::new();
        for item in &self.items {
            if (item.id != "legacy" && uuid::Uuid::parse_str(&item.id).is_err())
                || !ids.insert(&item.id)
                || !names.insert(item.name.to_lowercase())
                || !tags.insert(&item.tag)
                || item.tag.len() != 32
                || validate_name(&item.name).is_err()
                || item.secret_ref.as_ref().is_some_and(|reference| {
                    uuid::Uuid::parse_str(reference).is_err() || !refs.insert(reference)
                })
            {
                return Err("访问密钥记录无效或重复，已拒绝密钥鉴权".into());
            }
        }
        Ok(())
    }
    pub fn has_enabled(&self) -> bool {
        self.items.iter().any(|item| item.enabled)
    }
    pub fn view(&self) -> KeyList {
        KeyList {
            revision: self.revision,
            items: self
                .items
                .iter()
                .map(|item| KeyView {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    enabled: item.enabled,
                    can_copy: item.secret_ref.is_some(),
                    created_at: item.created_at.clone(),
                })
                .collect(),
        }
    }
    pub fn identify(&self, value: &str) -> Option<Identity> {
        self.items
            .iter()
            .find(|item| item.enabled && item.matches(value))
            .map(Credential::identity)
    }
    pub fn reveal(&self, key_id: &str, store: &impl KeyStore) -> Result<String, String> {
        let item = self
            .items
            .iter()
            .find(|item| item.id == key_id)
            .ok_or("访问密钥不存在，请刷新列表")?;
        let reference = item
            .secret_ref
            .as_deref()
            .ok_or("旧版只保存了校验值，请补录一次原密钥以启用复制；原密钥仍可使用")?;
        let value = store
            .get(reference)
            .map_err(|_| "无法读取访问密钥文件；请检查 access-keys 目录或补录原密钥")?;
        if !item.matches(&value) {
            return Err("密钥文件与校验值不匹配，未返回内容；请补录原密钥".into());
        }
        Ok(value)
    }

    pub fn change(
        self,
        revision: u64,
        operation: Operation,
        store: &impl KeyStore,
        persist: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<ChangeResult, String> {
        if revision != self.revision {
            return Err("访问密钥列表已变化，请刷新后重试".into());
        }
        let mut next = self.clone();
        let mut fresh: Option<(String, String)> = None;
        match operation {
            Operation::Add { name, value } => {
                let name = name.trim().to_owned();
                validate_name(&name)?;
                validate_value(&value)?;
                if name.contains(&value) || self.items.iter().any(|item| item.matches(&name)) {
                    return Err("名称不能包含密钥明文".into());
                }
                if next
                    .items
                    .iter()
                    .any(|item| item.name.to_lowercase() == name.to_lowercase())
                {
                    return Err("密钥名称已存在，请使用不同名称".into());
                }
                if next.items.len() >= MAX_KEYS {
                    return Err("最多保存 64 把访问密钥，请先移除不再需要的记录".into());
                }
                if next.items.iter().any(|item| item.matches(&value)) {
                    return Err("这把密钥已存在；旧版记录请使用“补录原密钥”，不要重复添加".into());
                }
                let reference = uuid::Uuid::new_v4().to_string();
                next.items.push(Credential {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    enabled: true,
                    tag: credential_mac(&value).finalize().into_bytes().to_vec(),
                    secret_ref: Some(reference.clone()),
                    created_at: Some(chrono::Utc::now().to_rfc3339()),
                });
                fresh = Some((reference, value));
            }
            operation => {
                let id = match &operation {
                    Operation::Rename { key_id, .. }
                    | Operation::SetEnabled { key_id, .. }
                    | Operation::Remove { key_id }
                    | Operation::RestoreCopy { key_id, .. } => key_id,
                    Operation::Add { .. } => unreachable!(),
                };
                let item = next
                    .items
                    .iter_mut()
                    .find(|item| &item.id == id)
                    .ok_or("访问密钥不存在，请刷新列表")?;
                match operation {
                    Operation::Rename { name, .. } => {
                        validate_name(name.trim())?;
                        // Do not turn an accidentally pasted credential into public log metadata.
                        if self.items.iter().any(|entry| entry.matches(name.trim())) {
                            return Err("名称不能使用密钥明文".into());
                        }
                        if self.items.iter().any(|entry| {
                            entry.id != item.id
                                && entry.name.to_lowercase() == name.trim().to_lowercase()
                        }) {
                            return Err("密钥名称已存在，请使用不同名称".into());
                        }
                        item.name = name.trim().into();
                    }
                    Operation::SetEnabled { enabled, .. } => item.enabled = enabled,
                    Operation::Remove { key_id } => next.items.retain(|item| item.id != key_id),
                    Operation::RestoreCopy { value, .. } => {
                        validate_value(&value)?;
                        if !item.matches(&value) {
                            return Err("与原密钥不匹配；未修改原密钥或访问权限".into());
                        }
                        let reference = uuid::Uuid::new_v4().to_string();
                        item.secret_ref = Some(reference.clone());
                        fresh = Some((reference, value));
                    }
                    Operation::Add { .. } => unreachable!(),
                }
            }
        }
        next.revision = next.revision.checked_add(1).ok_or("密钥版本超出上限")?;
        next.validate()?;
        let serialized = serde_json::to_string(&next).map_err(|_| "无法保存访问密钥列表")?;
        if let Some((reference, value)) = &fresh {
            if store.set(reference, value).is_err() {
                let cleanup_failed = store.delete(reference).is_err();
                return Err(if cleanup_failed {
                    "访问密钥文件写入失败且新文件清理未完成，请检查 access-keys 目录；旧权限未改变"
                } else {
                    "访问密钥文件写入失败，请检查 access-keys 目录权限或磁盘空间；旧权限未改变"
                }
                .into());
            }
        }
        if let Err(error) = persist(&serialized) {
            if let Some((reference, _)) = &fresh {
                if store.delete(reference).is_err() {
                    return Err(format!("{error}；新文件清理失败，旧密钥和访问权限未改变"));
                }
            }
            return Err(error);
        }
        let retained: HashSet<_> = next
            .items
            .iter()
            .filter_map(|item| item.secret_ref.as_ref())
            .collect();
        let mut warnings = vec![];
        for retired in self
            .items
            .iter()
            .filter_map(|item| item.secret_ref.as_ref())
            .filter(|reference| !retained.contains(reference))
        {
            if store.delete(retired).is_err() && warnings.is_empty() {
                warnings
                    .push("变更已生效，但旧明文文件清理失败，请检查 access-keys 目录权限".into());
            }
        }
        Ok(ChangeResult {
            keys: next.view(),
            warnings,
        })
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("密钥名称需为 1–60 个字符，不能包含控制字符".into());
    }
    Ok(())
}
fn validate_value(value: &str) -> Result<(), String> {
    if !(32..=256).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("密钥需为 32–256 位无空白 ASCII 字符，建议生成随机密钥".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
