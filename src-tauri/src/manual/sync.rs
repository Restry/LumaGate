use super::catalog::{hash, Document, Protocol};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Pi,
    Claude,
    Codex,
}
impl Agent {
    pub fn command(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
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
    pub created: Instant,
    files: Vec<FileChange>,
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
            return Err("没有可同步的启用模型，请先选择或恢复模型".into());
        }
        let mut files = vec![];
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
                        .map(|g| super::metadata::pi_model(&g.model, &g.id))
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
                note = "合并三个受管 Provider；不修改 Pi settings.json 中的默认 Provider/模型。"
                    .into();
            }
            Agent::Claude => {
                let path = home.join(".claude/settings.json");
                let before = read(&path)?;
                let mut config = json_config(&before)?;
                // 不使用未知的模型目录字段。Claude 的额外模型以辅助清单提供，用户用 /model ID 选择。
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
                let list = json!({"note": "辅助清单，不是 Claude Code 原生自动加载配置；用 /model <id> 手动选择", "models": groups.iter().map(|g| json!({"id": g.id, "name": g.model.name.as_deref().unwrap_or(&g.model.id), "contextWindow":g.model.context_window,"capabilities":super::metadata::capability_details(&g.model)})).collect::<Vec<_>>()});
                files.push(FileChange {
                    path,
                    before,
                    after: json_bytes(&list),
                    summary: list,
                });
                note = "保留默认模型与 MCP/Skills/Prompts。Claude 不原生读取此辅助目录；请从模型表复制 ID 后使用 /model。若原默认模型不在网关目录，需手选模型。".into();
            }
            Agent::Codex => {
                let path = home.join(".codex/config.toml");
                let before = read(&path)?;
                let text = std::str::from_utf8(before.as_deref().unwrap_or(b""))
                    .map_err(|_| "Codex 配置必须是 UTF-8")?;
                let mut config: toml_edit::DocumentMut =
                    text.parse().map_err(|_| "Codex TOML 无效，未修改")?;
                let catalog_path = home.join(".codex/cc-switch-manual-models.json");
                if let Some(existing) = config.get("model_catalog_json") {
                    if existing.as_str() != catalog_path.to_str() {
                        return Err("Codex 模型目录已由其他配置管理。为避免覆盖，请先手动处理 model_catalog_json，再预览同步".into());
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
                // 部分客户端要求非空密钥字段；这是兼容占位，不是访问授权。
                provider["experimental_bearer_token"] = toml_edit::value(super::LOCAL_TOKEN);
                config["model_providers"]["cc_switch_manual"] = toml_edit::Item::Table(provider);
                // 保留根 model/model_provider；用户明确选择 profile 后才使用网关。
                if let Some(existing) = config.get("profiles") {
                    if !existing.is_table() {
                        return Err("profiles 不是 TOML 表，拒绝覆盖".into());
                    }
                }
                if config.get("profiles").is_none() {
                    config["profiles"] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let mut profile = toml_edit::Table::new();
                profile["model_provider"] = toml_edit::value("cc_switch_manual");
                config["profiles"]["cc_switch_manual"] = toml_edit::Item::Table(profile);
                config["model_catalog_json"] =
                    toml_edit::value(catalog_path.to_string_lossy().to_string());
                let settings = json!({"modelCatalog": {"models": groups.iter().map(|g| json!({"model":g.id,"displayName":g.model.id})).collect::<Vec<_>>()}});
                let mut catalog = crate::codex_config::codex_model_catalog_from_settings(
                    &settings,
                    "",
                    crate::codex_config::CodexCatalogToolProfile::NativeResponses,
                )
                .map_err(|e| e.to_string())?
                .ok_or("无法生成 Codex 模型目录")?;
                if let Some(entries) = catalog.get_mut("models").and_then(Value::as_array_mut) {
                    for entry in entries {
                        if let Some(group) = groups.iter().find(|g| {
                            entry.get("slug").and_then(Value::as_str) == Some(g.id.as_str())
                        }) {
                            super::metadata::enrich_codex_entry(entry, &group.model);
                        }
                    }
                }
                let catalog_before = read(&catalog_path)?;
                files.push(FileChange {
                    path: catalog_path,
                    before: catalog_before,
                    after: json_bytes(&catalog),
                    summary: json!({"models":catalog["models"]}),
                });
                files.push(FileChange { path, before, after: config.to_string().into_bytes(), summary: json!({"model_providers.cc_switch_manual": {"base_url": format!("{base}/v1"), "wire_api":"responses"}, "profiles.cc_switch_manual.model_provider":"cc_switch_manual", "model_catalog_json":home.join(".codex/cc-switch-manual-models.json")}) });
                note = "保留默认模型和默认 Provider，新增 cc_switch_manual profile。需要使用网关时，手动运行 codex --profile cc_switch_manual；本页启动按钮仍只启动 codex。".into();
            }
        }
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            revision: doc.revision,
            created: Instant::now(),
            files,
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
        // 全部文件通过检查后才开始写；逐文件仍复验，避免覆盖预览后用户的改动。
        for file in &self.files {
            if read(&file.path)? != file.before {
                return Err(format!("文件已被修改，未写入：{}", file.path.display()));
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
    fn codex_keeps_defaults_and_uses_upstream_catalog_schema() {
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
            config["mcp_servers"]["custom"]["command"].as_str(),
            Some("keep")
        );
        assert!(text.contains("# preserve comment"));
        let catalog: Value = serde_json::from_slice(
            &fs::read(home.join(".codex/cc-switch-manual-models.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
        assert!(catalog["models"][0]["slug"]
            .as_str()
            .unwrap()
            .starts_with("demo--"));
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
}
