use super::catalog::{Document, Source};
use serde::Deserialize;
use std::{collections::HashSet, fs, io::Read, path::PathBuf};

// 不派生 Debug/Serialize，避免密钥输入进入日志或被当作读模型返回。
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum KeyUpdate {
    Set {
        #[serde(rename = "providerId")]
        provider_id: String,
        value: String,
    },
    Clear {
        #[serde(rename = "providerId")]
        provider_id: String,
    },
}

pub trait KeyStore {
    fn get(&self, reference: &str) -> Result<String, String>;
    fn set(&self, reference: &str, value: &str) -> Result<(), String>;
    fn delete(&self, reference: &str) -> Result<(), String>;
}

pub struct FileKeyStore {
    directory: PathBuf,
}

impl Default for FileKeyStore {
    fn default() -> Self {
        Self {
            // Finder 启动的工作目录不固定，密钥必须跟随应用数据，而不是源码或安装包。
            directory: crate::config::get_app_config_dir().join("keys"),
        }
    }
}

impl FileKeyStore {
    pub(crate) fn gateway_access() -> Self {
        Self {
            directory: crate::config::get_app_config_dir().join("access-keys"),
        }
    }

    #[cfg(test)]
    pub(crate) fn at_directory(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn path(&self, reference: &str) -> Result<PathBuf, String> {
        let id =
            uuid::Uuid::parse_str(reference).map_err(|_| "密钥引用无效，请重新保存 API Key")?;
        let path = self.directory.join(id.to_string());
        // 拒绝将本地密钥目录或文件重定向到别处，错误信息也不携带文件内容。
        for (candidate, directory) in [(&self.directory, true), (&path, false)] {
            match fs::symlink_metadata(candidate) {
                Ok(meta) => {
                    let expected_type = if directory {
                        meta.is_dir()
                    } else {
                        meta.is_file()
                    };
                    if meta.file_type().is_symlink() || !expected_type {
                        return Err(
                            "本地密钥路径不是普通目录或文件，请检查 ~/.lumagate/keys/".into()
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("无法访问本地密钥，请检查 ~/.lumagate/keys/ 的权限".into()),
            }
        }
        Ok(path)
    }
}

impl KeyStore for FileKeyStore {
    fn get(&self, reference: &str) -> Result<String, String> {
        let path = self.path(reference)?;
        let file = fs::File::open(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                // 旧的钥匙串引用也走此分支；不尝试迁移读取，避免再次弹出系统授权。
                "未找到本地 API Key，请编辑 Provider 重新填写并保存；应用不再读取系统钥匙串"
            } else {
                "无法读取本地 API Key，请检查 ~/.lumagate/keys/ 的权限"
            }
        })?;
        let mut value = String::new();
        file.take(8193)
            .read_to_string(&mut value)
            .map_err(|_| "本地 API Key 文件无法读取，请重新填写并保存")?;
        if value.is_empty() {
            return Err("本地 API Key 文件为空，请重新填写并保存".into());
        }
        validate_secret(&value)?;
        Ok(value)
    }

    fn set(&self, reference: &str, value: &str) -> Result<(), String> {
        validate_secret(value)?;
        if value.is_empty() {
            return Err("API Key 不能为空".into());
        }
        let path = self.path(reference)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&self.directory)
            .map_err(|_| "无法创建本地密钥目录，请检查 ~/.lumagate/ 的写入权限")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法限制本地密钥目录的访问权限，未保存密钥")?;
        }
        crate::config::atomic_write_private(&path, value.as_bytes())
            .map_err(|_| "无法保存本地 API Key，请检查目录写入权限和磁盘空间".into())
    }

    fn delete(&self, reference: &str) -> Result<(), String> {
        let path = self.path(reference)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            // 已删除或从未落盘的旧引用不应阻塞重新保存，也不触碰系统钥匙串。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法删除旧的本地密钥文件，请检查目录写入权限".into()),
        }
    }
}

pub fn credential_version(source: &Source) -> String {
    if let Some(binding) = &source.copilot {
        return super::catalog::hash(
            &serde_json::json!(["manual_copilot", source.base_url, source.protocol, binding])
                .to_string(),
        );
    }
    super::catalog::hash(
        &serde_json::json!([
            source.base_url,
            source.protocol,
            source.key_env,
            source.key_ref,
            // 环境变量名字不变不代表账号不变；重启后仍需识别密钥轮换。
            if source.key_ref.is_none() && !source.key_env.is_empty() {
                std::env::var(&source.key_env)
                    .ok()
                    .map(|value| super::catalog::hash(&value))
            } else {
                None
            }
        ])
        .to_string(),
    )
}

pub fn resolve(source: &Source, store: &impl KeyStore) -> Result<String, String> {
    if let Some(reference) = &source.key_ref {
        return store.get(reference);
    }
    if source.key_env.is_empty() {
        return Ok(String::new());
    }
    std::env::var(&source.key_env)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "环境变量 {} 未设置；也可编辑 Provider，直接粘贴 API Key",
                source.key_env
            )
        })
}

fn validate_secret(value: &str) -> Result<(), String> {
    if value.len() > 8192 || http::HeaderValue::from_str(value).is_err() {
        return Err("API Key 过长或包含不允许的字符，请检查粘贴内容".into());
    }
    Ok(())
}

/// 先写不可变的新密钥版本，再提交引用；提交失败只清理新条目，不破坏旧密钥。
pub fn save_with_keys(
    current: &Document,
    mut proposed: Document,
    update: Option<KeyUpdate>,
    store: &impl KeyStore,
    persist: impl FnOnce(&mut Document) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    if proposed.revision != current.revision {
        return Err("配置已变化，请刷新后重试".into());
    }
    // 屏蔽状态只由专用命令修改，旧表单或缺字段的客户端不能意外清空它。
    proposed.blocked_models = current.blocked_models.clone();
    for source in &mut proposed.providers {
        // 客户端不能通过猜测引用，借用其他 Provider 的密钥文件。
        source.key_ref = current
            .providers
            .iter()
            .find(|p| p.id == source.id)
            .and_then(|p| p.key_ref.clone());
    }
    let mut new_key: Option<(String, String)> = None;
    if let Some(update) = update {
        let provider_id = match &update {
            KeyUpdate::Set { provider_id, .. } | KeyUpdate::Clear { provider_id } => provider_id,
        };
        let source = proposed
            .providers
            .iter_mut()
            .find(|p| &p.id == provider_id)
            .ok_or("密钥对应的 Provider 不存在")?;
        match update {
            KeyUpdate::Set { value, .. } => {
                let value = value.trim();
                // 留空保留已有密钥；移除必须使用独立的 Clear 意图。
                if !value.is_empty() {
                    validate_secret(value)?;
                    let reference = uuid::Uuid::new_v4().to_string();
                    source.key_ref = Some(reference.clone());
                    source.key_env.clear();
                    new_key = Some((reference, value.to_string()));
                }
            }
            KeyUpdate::Clear { .. } => {
                source.key_ref = None;
            }
        }
    }
    for source in &mut proposed.providers {
        // 模型列表仍由发现结果管理，表单只能修改已有条目的启用状态。
        let selected: std::collections::HashMap<_, _> = source
            .models
            .iter()
            .map(|model| (model.id.clone(), model.enabled))
            .collect();
        source.models = current
            .providers
            .iter()
            .find(|previous| {
                previous.id == source.id
                    && previous.base_url == source.base_url
                    && previous.protocol == source.protocol
                    && previous.key_env == source.key_env
                    && previous.key_ref == source.key_ref
            })
            .map(|previous| previous.models.clone())
            .unwrap_or_default();
        for model in &mut source.models {
            if let Some(enabled) = selected.get(&model.id) {
                model.enabled = *enabled;
            }
        }
    }
    proposed.validate()?;
    proposed.tests.clear();
    if let Some((reference, value)) = &new_key {
        store.set(reference, value)?;
    }
    if let Err(error) = persist(&mut proposed) {
        if let Some((reference, _)) = &new_key {
            if store.delete(reference).is_err() {
                return Err(format!("{error}；新密钥文件清理失败，原有密钥未更改"));
            }
        }
        return Err(error);
    }
    let retained: HashSet<_> = proposed
        .providers
        .iter()
        .filter_map(|p| p.key_ref.as_deref())
        .collect();
    let retired: HashSet<_> = current
        .providers
        .iter()
        .filter_map(|p| p.key_ref.as_deref())
        .filter(|r| !retained.contains(r))
        .collect();
    let mut warnings = vec![];
    for reference in retired {
        if store.delete(reference).is_err() && warnings.is_empty() {
            warnings.push(
                "配置已保存，但旧密钥文件清理失败，请检查 ~/.lumagate/keys/ 的写入权限。".into(),
            );
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{cell::RefCell, collections::HashMap};
    #[derive(Default)]
    struct FakeStore {
        values: RefCell<HashMap<String, String>>,
        fail_set: bool,
        fail_delete: bool,
    }
    impl KeyStore for FakeStore {
        fn get(&self, r: &str) -> Result<String, String> {
            self.values.borrow().get(r).cloned().ok_or("missing".into())
        }
        fn set(&self, r: &str, v: &str) -> Result<(), String> {
            if self.fail_set {
                return Err("store unavailable".into());
            };
            self.values.borrow_mut().insert(r.into(), v.into());
            Ok(())
        }
        fn delete(&self, r: &str) -> Result<(), String> {
            if self.fail_delete {
                return Err("store unavailable".into());
            };
            self.values.borrow_mut().remove(r);
            Ok(())
        }
    }
    fn doc() -> Document {
        serde_json::from_value(json!({"revision":1,"providers":[{"id":"p","name":"P","baseUrl":"https://example.com/v1","keyEnv":"","protocol":"openai_chat","models":[{"id":"model"}]}],"policies":{},"tests":{}})).unwrap()
    }
    fn set(value: &str) -> Option<KeyUpdate> {
        Some(KeyUpdate::Set {
            provider_id: "p".into(),
            value: value.into(),
        })
    }
    #[test]
    fn model_selection_only_changes_enabled_not_catalog_or_endpoint() {
        let store = FakeStore::default();
        let mut current = doc();
        current.providers[0].models[0].context_window = Some(32_000);
        current.providers[0].models[0].protocol_override =
            Some(super::super::catalog::Protocol::OpenaiResponses);
        current.providers[0].models[0].metadata = json!({"owned":"upstream"});
        let mut proposed = current.clone();
        proposed.providers[0].models[0].enabled = false;
        proposed.providers[0].models[0].context_window = Some(999_999);
        proposed.providers[0].models[0].protocol_override = None;
        proposed.providers[0].models[0].metadata = json!({"owned":"client"});
        let mut forged = proposed.providers[0].models[0].clone();
        forged.id = "forged-model".into();
        proposed.providers[0].models.push(forged);
        save_with_keys(&current, proposed, None, &store, |saved| {
            assert_eq!(saved.providers[0].models.len(), 1);
            let model = &saved.providers[0].models[0];
            assert!(!model.enabled);
            assert_eq!(model.context_window, Some(32_000));
            assert_eq!(
                model.protocol_override,
                current.providers[0].models[0].protocol_override
            );
            assert_eq!(model.metadata["owned"], "upstream");
            assert!(saved.groups().is_empty());
            Ok(())
        })
        .unwrap();
        assert!(current.providers[0].models[0].enabled);
    }

    #[test]
    fn partial_selection_preserves_other_models_and_providers() {
        let store = FakeStore::default();
        let mut current = doc();
        let mut hidden = current.providers[0].models[0].clone();
        hidden.id = "hidden-model".into();
        current.providers[0].models.push(hidden);
        let mut other = current.providers[0].clone();
        other.id = "other".into();
        current.providers.push(other);
        let mut proposed = current.clone();
        proposed.providers[0]
            .models
            .retain(|model| model.id == "model");
        proposed.providers[0].models[0].enabled = false;
        let mut saved = None;
        save_with_keys(&current, proposed, None, &store, |document| {
            assert!(document.providers[0].models[1].enabled);
            assert!(document.providers[1]
                .models
                .iter()
                .all(|model| model.enabled));
            let groups = document.groups();
            assert_eq!(
                groups
                    .iter()
                    .find(|g| g.model.id == "model")
                    .unwrap()
                    .provider_ids,
                ["other"]
            );
            saved = Some(document.clone());
            Ok(())
        })
        .unwrap();
        let saved = saved.unwrap();
        let mut enabled = saved.clone();
        enabled.providers[0].models[0].enabled = true;
        save_with_keys(&saved, enabled, None, &store, |document| {
            assert_eq!(
                document
                    .groups()
                    .iter()
                    .find(|g| g.model.id == "model")
                    .unwrap()
                    .provider_ids,
                ["p", "other"]
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn stores_only_reference_and_resolves_saved_secret() {
        let store = FakeStore::default();
        let current = doc();
        let mut saved = None;
        save_with_keys(
            &current,
            current.clone(),
            set(" fixture-key "),
            &store,
            |d| {
                saved = Some(d.clone());
                Ok(())
            },
        )
        .unwrap();
        let saved = saved.unwrap();
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains("fixture-key"));
        assert_eq!(resolve(&saved.providers[0], &store).unwrap(), "fixture-key");
        assert!(saved.providers[0].models.is_empty());
    }
    #[test]
    fn successful_rotation_retires_old_key_after_commit() {
        let store = FakeStore::default();
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        store.set(&reference, "old-fixture").unwrap();
        let mut saved = None;
        save_with_keys(&current, current.clone(), set("new-fixture"), &store, |d| {
            assert_eq!(store.get(&reference).unwrap(), "old-fixture");
            saved = Some(d.clone());
            Ok(())
        })
        .unwrap();
        let saved = saved.unwrap();
        assert!(store.get(&reference).is_err());
        assert_eq!(resolve(&saved.providers[0], &store).unwrap(), "new-fixture");
        assert_ne!(
            credential_version(&current.providers[0]),
            credential_version(&saved.providers[0])
        );
    }
    #[test]
    fn empty_input_preserves_key_and_catalog() {
        let store = FakeStore::default();
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        store.set(&reference, "fixture-key").unwrap();
        save_with_keys(&current, current.clone(), set(""), &store, |d| {
            assert_eq!(d.providers[0].key_ref.as_ref(), Some(&reference));
            assert_eq!(d.providers[0].models.len(), 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(store.get(&reference).unwrap(), "fixture-key");
    }
    #[test]
    fn failed_persistence_does_not_destroy_old_key() {
        let store = FakeStore::default();
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        store.set(&reference, "old-fixture").unwrap();
        assert!(save_with_keys(
            &current,
            current.clone(),
            set("new-fixture"),
            &store,
            |_| Err("db failed".into())
        )
        .is_err());
        assert_eq!(store.values.borrow().len(), 1);
        assert_eq!(store.get(&reference).unwrap(), "old-fixture");
    }
    #[test]
    fn unavailable_store_does_not_commit_config() {
        let store = FakeStore {
            fail_set: true,
            ..FakeStore::default()
        };
        let current = doc();
        assert!(save_with_keys(
            &current,
            current.clone(),
            set("fixture-key"),
            &store,
            |_| panic!("must not persist")
        )
        .is_err());
    }
    #[test]
    fn deletion_removes_key_after_commit() {
        let store = FakeStore::default();
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        store.set(&reference, "fixture-key").unwrap();
        let mut proposed = current.clone();
        proposed.providers.clear();
        save_with_keys(&current, proposed, None, &store, |_| {
            assert!(store.get(&reference).is_ok());
            Ok(())
        })
        .unwrap();
        assert!(store.values.borrow().is_empty());
    }
    #[test]
    fn cannot_forge_references_or_inject_headers() {
        let current = doc();
        let mut proposed = current.clone();
        proposed.providers[0].key_ref = Some(uuid::Uuid::new_v4().to_string());
        let store = FakeStore::default();
        save_with_keys(&current, proposed, None, &store, |d| {
            assert!(d.providers[0].key_ref.is_none());
            Ok(())
        })
        .unwrap();
        assert!(save_with_keys(
            &current,
            current.clone(),
            set("fixture\r\nInjected: header"),
            &store,
            |_| panic!("must not persist")
        )
        .is_err());
    }
    #[test]
    fn file_store_roundtrip_survives_reopen_and_clears_after_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("keys");
        let store = FileKeyStore {
            directory: directory.clone(),
        };
        let current = doc();
        let mut saved = None;
        save_with_keys(
            &current,
            current.clone(),
            set("local-fixture"),
            &store,
            |d| {
                saved = Some(serde_json::to_string(d).unwrap());
                Ok(())
            },
        )
        .unwrap();
        let saved = saved.unwrap();
        assert!(!saved.contains("local-fixture"));
        let current: Document = serde_json::from_str(&saved).unwrap();
        drop(store);
        let reopened = FileKeyStore {
            directory: directory.clone(),
        };
        assert_eq!(
            resolve(&current.providers[0], &reopened).unwrap(),
            "local-fixture"
        );
        save_with_keys(&current, current.clone(), set(""), &reopened, |d| {
            assert_eq!(d.providers[0].key_ref, current.providers[0].key_ref);
            Ok(())
        })
        .unwrap();
        let mut rotated = None;
        save_with_keys(
            &current,
            current.clone(),
            set("replacement-fixture"),
            &reopened,
            |d| {
                assert_eq!(
                    resolve(&current.providers[0], &reopened).unwrap(),
                    "local-fixture"
                );
                rotated = Some(d.clone());
                Ok(())
            },
        )
        .unwrap();
        let rotated = rotated.unwrap();
        assert_eq!(
            resolve(&rotated.providers[0], &reopened).unwrap(),
            "replacement-fixture"
        );
        assert!(resolve(&current.providers[0], &reopened).is_err());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        save_with_keys(
            &rotated,
            rotated.clone(),
            Some(KeyUpdate::Clear {
                provider_id: "p".into(),
            }),
            &reopened,
            |d| {
                assert!(d.providers[0].key_ref.is_none());
                assert!(resolve(&rotated.providers[0], &reopened).is_ok());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
    }

    #[test]
    fn file_store_failed_commit_preserves_old_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileKeyStore {
            directory: tmp.path().join("keys"),
        };
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        store.set(&reference, "old-fixture").unwrap();
        current.providers[0].key_ref = Some(reference.clone());
        assert!(save_with_keys(
            &current,
            current.clone(),
            set("replacement-fixture"),
            &store,
            |_| Err("db fixture failure".into())
        )
        .is_err());
        assert_eq!(store.get(&reference).unwrap(), "old-fixture");
        assert_eq!(std::fs::read_dir(&store.directory).unwrap().count(), 1);
    }

    #[test]
    fn file_store_unavailable_directory_does_not_commit_or_leak_secret() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("keys");
        std::fs::write(&directory, "blocking-file").unwrap();
        let store = FileKeyStore { directory };
        let current = doc();
        let error = save_with_keys(
            &current,
            current.clone(),
            set("private-fixture"),
            &store,
            |_| panic!("保存失败不得提交目录"),
        )
        .unwrap_err();
        assert!(!error.contains("private-fixture"));
    }

    #[test]
    fn file_store_missing_legacy_reference_can_be_replaced_without_keychain() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileKeyStore {
            directory: tmp.path().join("keys"),
        };
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        assert!(resolve(&current.providers[0], &store)
            .unwrap_err()
            .contains("重新填写"));
        store.delete(&reference).unwrap();
        assert!(!store.directory.exists());
        let warnings = save_with_keys(
            &current,
            current.clone(),
            set("local-fixture"),
            &store,
            |d| {
                assert_ne!(d.providers[0].key_ref, current.providers[0].key_ref);
                assert_eq!(resolve(&d.providers[0], &store).unwrap(), "local-fixture");
                Ok(())
            },
        )
        .unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn file_store_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileKeyStore {
            directory: tmp.path().join("keys"),
        };
        for reference in ["../outside", "/tmp/outside", "", "a/b"] {
            assert!(store.get(reference).is_err());
            assert!(store.set(reference, "fixture").is_err());
            assert!(store.delete(reference).is_err());
        }
        assert!(!store.directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn file_store_uses_private_permissions_including_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let store = FileKeyStore {
            directory: tmp.path().join("keys"),
        };
        let reference = uuid::Uuid::new_v4().to_string();
        let path = store.directory.join(&reference);
        store.set(&reference, "fixture").unwrap();
        assert_eq!(
            std::fs::metadata(&store.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&store.directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        store.set(&reference, "replacement-fixture").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&store.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_store_rejects_symlinked_directory_and_key() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("keys");
        let store = FileKeyStore {
            directory: directory.clone(),
        };
        let reference = uuid::Uuid::new_v4().to_string();
        let target = outside.path().join(&reference);
        std::fs::write(&target, "outside-fixture").unwrap();
        symlink(outside.path(), &directory).unwrap();
        assert!(store.get(&reference).is_err());
        assert!(store.set(&reference, "replacement-fixture").is_err());
        assert!(store.delete(&reference).is_err());
        std::fs::remove_file(&directory).unwrap();
        std::fs::create_dir(&directory).unwrap();
        symlink(&target, directory.join(&reference)).unwrap();
        assert!(store.get(&reference).is_err());
        assert!(store.set(&reference, "replacement-fixture").is_err());
        assert!(store.delete(&reference).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "outside-fixture");
    }

    #[test]
    fn file_store_rejects_corrupt_values_without_echoing_them() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileKeyStore {
            directory: tmp.path().join("keys"),
        };
        let reference = uuid::Uuid::new_v4().to_string();
        store.set(&reference, "fixture").unwrap();
        for value in [
            "".to_string(),
            "private-fixture\r\nInjected: header".into(),
            "x".repeat(8193),
        ] {
            std::fs::write(store.directory.join(&reference), &value).unwrap();
            let error = store.get(&reference).unwrap_err();
            assert!(!error.contains("private-fixture"));
        }
    }

    #[test]
    fn file_store_defaults_to_application_data_not_working_directory() {
        assert_eq!(
            FileKeyStore::default().directory,
            crate::config::get_app_config_dir().join("keys")
        );
    }

    #[test]
    fn explicit_clear_can_switch_back_to_environment() {
        let store = FakeStore::default();
        let mut current = doc();
        let reference = uuid::Uuid::new_v4().to_string();
        current.providers[0].key_ref = Some(reference.clone());
        store.set(&reference, "fixture-key").unwrap();
        let mut proposed = current.clone();
        proposed.providers[0].key_env = "EXAMPLE_KEY".into();
        save_with_keys(
            &current,
            proposed,
            Some(KeyUpdate::Clear {
                provider_id: "p".into(),
            }),
            &store,
            |d| {
                assert!(d.providers[0].key_ref.is_none());
                assert_eq!(d.providers[0].key_env, "EXAMPLE_KEY");
                Ok(())
            },
        )
        .unwrap();
        assert!(store.values.borrow().is_empty());
    }
}
