//! Personal github.com Copilot adapter. Secrets and device codes never cross the UI seam.
//! Uses the repository's existing editor-compatible integration identity; no AppHandle,
//! automatic client configuration writes, account rotation, or billing/header optimizations.
mod commands;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;
use super::catalog::{Model, Protocol, Source};
use crate::proxy::providers::copilot_auth::{
    COPILOT_API_VERSION, COPILOT_EDITOR_VERSION, COPILOT_INTEGRATION_ID, COPILOT_PLUGIN_VERSION,
    COPILOT_USER_AGENT,
};
pub use commands::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub const PROVIDER_ID: &str = "copilot-personal";
pub const LOGIN_URL: &str = "https://github.com/login/device";
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98"; // Same compatibility identity as the existing Copilot adapter.
const API_ROOT: &str = "https://api.githubcopilot.com";
const MAX_BODY: usize = 4 * 1024 * 1024;
const AUTH_EXPIRED: &str = "GitHub/Copilot 授权失效，请重新登录";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub account_id: String,
    pub grant_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    version: u32,
    account_id: String,
    login: String,
    github_token: String,
    grant_id: String,
}
struct Access {
    token: String,
    expires_at: i64,
    endpoint: String,
}
#[derive(Default)]
struct State {
    credential: Option<Credential>,
    access: Option<Access>,
    load_error: bool,
    reauth_required: bool,
}
struct Device {
    id: String,
    code: String,
    expires: Instant,
    interval: u64,
    next_poll: Instant,
    polling: bool,
    revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub flow_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub login: Option<String>,
    pub message: Option<String>,
}
pub struct Manager {
    path: PathBuf,
    client: reqwest::Client,
    state: Mutex<State>,
    device: Mutex<Option<Device>>,
    github: String,
    api: String,
}
struct Authorized {
    credential: Credential,
    access: Access,
    revision: u64,
}
enum Poll {
    Pending(u64),
    Ready(Authorized),
}
impl Manager {
    pub fn new(root: PathBuf) -> Self {
        Self::at(
            root,
            "https://github.com".into(),
            "https://api.github.com".into(),
        )
    }
    fn at(root: PathBuf, github: String, api: String) -> Self {
        let path = root.join("credentials.json");
        let loaded = if path.exists() {
            fs::read(&path)
                .ok()
                .filter(|b| b.len() < 65536)
                .and_then(|b| serde_json::from_slice::<Credential>(&b).ok())
                .filter(|c| {
                    c.version == 1
                        && !c.github_token.is_empty()
                        && !c.account_id.is_empty()
                        && uuid::Uuid::parse_str(&c.grant_id).is_ok()
                })
        } else {
            None
        };
        let failed = path.exists() && loaded.is_none();
        Self {
            path,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Copilot HTTP client"),
            state: Mutex::new(State {
                credential: loaded,
                access: None,
                load_error: failed,
                reauth_required: false,
            }),
            device: Mutex::new(None),
            github,
            api,
        }
    }
    pub async fn status(&self, binding: Option<&Binding>) -> Status {
        let state = self.state.lock().await;
        let matches = state.credential.as_ref().filter(|c| {
            binding.is_some_and(|b| c.account_id == b.account_id && c.grant_id == b.grant_id)
        });
        Status {
            connected: matches.is_some() && !state.reauth_required,
            login: matches.map(|c| c.login.clone()),
            message: if state.load_error {
                Some(
                    "本机 Copilot 授权文件无法读取。先断开本机登录后重新授权；原文件未被覆盖。"
                        .into(),
                )
            } else if state.reauth_required || (binding.is_some() && matches.is_none()) {
                Some(AUTH_EXPIRED.into())
            } else {
                None
            },
        }
    }
    pub async fn start(&self, revision: u64) -> Result<Challenge, String> {
        if self.state.lock().await.load_error {
            return Err("授权文件无法读取，请先断开本机登录再重试".into());
        }
        // Reserve the generation before I/O. A cancellation during the request remains effective.
        let id = uuid::Uuid::new_v4().to_string();
        *self.device.lock().await = Some(Device {
            id: id.clone(),
            code: String::new(),
            expires: Instant::now() + Duration::from_secs(900),
            interval: 5,
            next_poll: Instant::now(),
            polling: true,
            revision,
        });
        let result = read_json(
            self.client
                .post(format!("{}/login/device/code", self.github))
                .header("accept", "application/json")
                .form(&[("client_id", CLIENT_ID), ("scope", "read:user")])
                .send()
                .await
                .map_err(|_| "无法连接 GitHub，请检查网络后重试")?,
        )
        .await?;
        let code = required(&result, "device_code")?;
        let user = required(&result, "user_code")?;
        let expires = result["expires_in"]
            .as_u64()
            .filter(|n| (1..=1800).contains(n))
            .ok_or("GitHub 返回了无效的授权有效期")?;
        let interval = result["interval"].as_u64().unwrap_or(5).clamp(5, 60);
        let mut flow = self.device.lock().await;
        let d = flow.as_mut().filter(|d| d.id == id).ok_or("授权已取消")?;
        d.code = code;
        d.expires = Instant::now() + Duration::from_secs(expires);
        d.interval = interval;
        d.next_poll = Instant::now() + Duration::from_secs(interval);
        d.polling = false;
        Ok(Challenge {
            flow_id: id,
            user_code: user,
            verification_uri: LOGIN_URL.into(),
            expires_in: expires,
            interval,
        })
    }
    pub async fn cancel(&self, flow_id: &str) {
        let mut d = self.device.lock().await;
        if d.as_ref().is_some_and(|d| d.id == flow_id) {
            *d = None;
        }
    }
    async fn poll(&self, id: &str) -> Result<Poll, String> {
        let (code, revision) = {
            let mut all = self.device.lock().await;
            let d = all
                .as_mut()
                .filter(|d| d.id == id)
                .ok_or("授权已取消或过期")?;
            if Instant::now() >= d.expires {
                *all = None;
                return Err("授权码已过期，请重新生成".into());
            }
            if d.polling || Instant::now() < d.next_poll {
                return Ok(Poll::Pending(d.interval));
            }
            d.polling = true;
            d.next_poll = Instant::now() + Duration::from_secs(d.interval);
            (d.code.clone(), d.revision)
        };
        let attempt = self.poll_remote(&code, revision).await;
        let mut all = self.device.lock().await;
        let d = all.as_mut().filter(|d| d.id == id).ok_or("授权已取消")?;
        d.polling = false;
        match attempt {
            Ok(None) => Ok(Poll::Pending(d.interval)),
            Err(error) if error == "slow_down" => {
                d.interval = (d.interval + 5).min(300);
                d.next_poll = Instant::now() + Duration::from_secs(d.interval);
                Ok(Poll::Pending(d.interval))
            }
            Ok(Some(auth)) => Ok(Poll::Ready(auth)),
            Err(error) => {
                *all = None;
                Err(error)
            }
        }
    }
    async fn poll_remote(&self, code: &str, revision: u64) -> Result<Option<Authorized>, String> {
        let data = read_json(
            self.client
                .post(format!("{}/login/oauth/access_token", self.github))
                .header("accept", "application/json")
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("device_code", code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await
                .map_err(|_| "GitHub 授权查询失败，请重试")?,
        )
        .await?;
        if let Some(error) = data["error"].as_str() {
            return match error {
                "authorization_pending" => Ok(None),
                "slow_down" => Err("slow_down".into()),
                "expired_token" => Err("授权码已过期，请重新生成".into()),
                "access_denied" => Err("你已取消 GitHub 授权".into()),
                _ => Err("GitHub 未接受本次设备授权，请重新登录".into()),
            };
        }
        let token = required(&data, "access_token")?;
        let user = read_json(
            self.client
                .get(format!("{}/user", self.api))
                .bearer_auth(&token)
                .header("user-agent", COPILOT_USER_AGENT)
                .send()
                .await
                .map_err(|_| "无法读取 GitHub 账号信息")?,
        )
        .await?;
        let id = user["id"]
            .as_u64()
            .ok_or("GitHub 账号信息无效")?
            .to_string();
        let login = required(&user, "login")?;
        let credential = Credential {
            version: 1,
            account_id: id,
            login,
            github_token: token,
            grant_id: uuid::Uuid::new_v4().to_string(),
        };
        let access = self.exchange(&credential.github_token).await?;
        Ok(Some(Authorized {
            credential,
            access,
            revision,
        }))
    }
    async fn exchange(&self, github_token: &str) -> Result<Access, String> {
        let data = read_json(
            self.client
                .get(format!("{}/copilot_internal/v2/token", self.api))
                .bearer_auth(github_token)
                .header("user-agent", COPILOT_USER_AGENT)
                .header("editor-version", COPILOT_EDITOR_VERSION)
                .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
                .send()
                .await
                .map_err(|_| "无法获取 Copilot 会话，请检查网络后重试")?,
        )
        .await?;
        let token = required(&data, "token")?;
        let expires = data["expires_at"]
            .as_i64()
            .filter(|t| *t > chrono::Utc::now().timestamp())
            .ok_or("Copilot 会话已过期，请重新登录")?;
        let usage = if data.pointer("/endpoints/api").is_none() {
            match self
                .client
                .get(format!("{}/copilot_internal/user", self.api))
                .bearer_auth(github_token)
                .header("user-agent", COPILOT_USER_AGENT)
                .header("editor-version", COPILOT_EDITOR_VERSION)
                .send()
                .await
            {
                Ok(response) => read_json(response).await.unwrap_or(Value::Null),
                Err(_) => Value::Null,
            }
        } else {
            Value::Null
        };
        let endpoint = data
            .pointer("/endpoints/api")
            .or_else(|| usage.pointer("/endpoints/api"))
            .and_then(Value::as_str)
            .unwrap_or(API_ROOT);
        validate_endpoint(endpoint)?;
        Ok(Access {
            token,
            expires_at: expires,
            endpoint: endpoint.trim_end_matches('/').into(),
        })
    }
    fn persist(&self, credential: &Credential) -> Result<(), String> {
        let dir = self.path.parent().ok_or("授权存储目录无效")?;
        fs::create_dir_all(dir).map_err(|_| "无法创建私有授权目录")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法保护授权目录")?;
        }
        let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&tmp)?;
            file.write_all(&serde_json::to_vec(credential).map_err(std::io::Error::other)?)?;
            file.sync_all()?;
            fs::rename(&tmp, &self.path)
        })()
        .map_err(|_: std::io::Error| "无法保存 Copilot 登录，请检查磁盘和私有目录权限".to_string());
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    pub async fn resolve(&self, binding: &Binding) -> Result<(String, String), String> {
        let mut state = self.state.lock().await;
        let credential = state
            .credential
            .as_ref()
            .filter(|c| c.account_id == binding.account_id && c.grant_id == binding.grant_id)
            .cloned()
            .ok_or("Copilot 登录已失效，请重新登录")?;
        if state
            .access
            .as_ref()
            .is_none_or(|a| a.expires_at - chrono::Utc::now().timestamp() < 60)
        {
            match self.exchange(&credential.github_token).await {
                Ok(access) => {
                    state.access = Some(access);
                    state.reauth_required = false;
                }
                Err(error) => {
                    if error == AUTH_EXPIRED {
                        state.reauth_required = true;
                        state.access = None;
                    }
                    return Err(error);
                }
            }
        }
        let a = state.access.as_ref().ok_or("Copilot 会话不可用")?;
        Ok((a.token.clone(), a.endpoint.clone()))
    }
    pub async fn discover(&self, source: &Source) -> Result<Vec<Model>, String> {
        let binding = source.copilot.as_ref().ok_or("不是 Copilot 来源")?;
        for attempt in 0..2 {
            let (token, base) = self.resolve(binding).await?;
            let response = self
                .client
                .get(format!("{base}/models"))
                .headers(headers(&token)?)
                .send()
                .await
                .map_err(|_| "无法读取 Copilot 模型目录，已有目录保持不变")?;
            // A short-lived session can be revoked before expires_at. Refresh only that
            // session once, using the saved OAuth grant; never start a device login here.
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let mut state = self.state.lock().await;
                if state.access.as_ref().is_some_and(|a| a.token == token) {
                    state.access = None;
                }
                continue;
            }
            let mut models = parse_models(&read_json(response).await?)?;
            for model in &mut models {
                if let Some(old) = source.models.iter().find(|m| m.id == model.id) {
                    model.enabled = old.enabled;
                }
            }
            return Ok(models);
        }
        Err(AUTH_EXPIRED.into())
    }
    pub async fn provider(
        &self,
        source: &Source,
        model: &Model,
        app: &str,
    ) -> Result<crate::provider::Provider, String> {
        let (token, base) = self
            .resolve(source.copilot.as_ref().ok_or("Copilot 绑定缺失")?)
            .await?;
        let mut copy = source.clone();
        copy.base_url = base;
        let mut upstream = super::catalog::upstream_provider_with_key(&copy, model, app, &token)?;
        upstream.settings_config["manual_credential_version"] =
            json!(super::catalog::deployment_version(source, model));
        upstream.settings_config["manual_copilot_token"] = json!(token);
        upstream.settings_config["manual_upstream_model"] = json!(upstream_id(model)?);
        Ok(upstream)
    }
    pub async fn test(&self, source: &Source, model: &Model) -> Result<(), String> {
        let (token, base) = self
            .resolve(source.copilot.as_ref().ok_or("Copilot 绑定缺失")?)
            .await?;
        let id = upstream_id(model)?;
        let response = matches!(model.protocol_override, Some(Protocol::OpenaiResponses));
        let output_limit = model.max_output_tokens.unwrap_or(64).min(64);
        let payload = if response {
            json!({"model":id,"input":"Reply with OK.","stream":false,"max_output_tokens":output_limit})
        } else {
            json!({"model":id,"messages":[{"role":"user","content":"Reply with OK."}],"stream":false,"max_tokens":output_limit})
        };
        let path = if response {
            "responses"
        } else if matches!(model.protocol_override, Some(Protocol::Anthropic)) {
            "v1/messages"
        } else {
            "chat/completions"
        };
        let mut request_headers = headers(&token)?;
        if matches!(model.protocol_override, Some(Protocol::Anthropic)) {
            request_headers.insert(
                "anthropic-version",
                reqwest::header::HeaderValue::from_static("2023-06-01"),
            );
        }
        let value = read_json(
            self.client
                .post(format!("{base}/{path}"))
                .headers(request_headers)
                .json(&payload)
                .send()
                .await
                .map_err(|_| "Copilot 请求失败（网络或 TLS），未自动重试")?,
        )
        .await?;
        validate_test_response(&value)
    }
}
fn validate_test_response(value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "无法解析模型响应")?;
    if super::completion::inspect(&bytes).ok() != Some(super::completion::Completion::Completed) {
        return Err(
            "模型未确认完成本次测试；请检查权限、额度或输出预算。HTTP 200 不代表测试成功".into(),
        );
    }
    if !["choices", "output", "content"]
        .iter()
        .any(|k| value[*k].as_array().is_some_and(|a| !a.is_empty()))
    {
        return Err("模型响应没有有效输出".into());
    }
    Ok(())
}
pub fn validate_document_edit(
    current: &super::catalog::Document,
    proposed: &super::catalog::Document,
) -> Result<(), String> {
    for source in &proposed.providers {
        let previous = current.providers.iter().find(|p| p.id == source.id);
        if source.copilot.as_ref() != previous.and_then(|p| p.copilot.as_ref()) {
            return Err("Copilot 绑定只能通过登录或断开入口修改".into());
        }
        if source.copilot.is_some() {
            let old = previous.ok_or("Copilot 来源缺失")?;
            if source.models.len() != old.models.len()
                || source.models.iter().any(|model| {
                    old.models
                        .iter()
                        .find(|m| m.id == model.id)
                        .is_none_or(|prior| {
                            let mut copy = model.clone();
                            copy.enabled = prior.enabled;
                            copy != *prior
                        })
                })
            {
                return Err(
                    "Copilot 目录、Endpoint 和能力只能通过刷新模型更新；这里仅允许启停模型".into(),
                );
            }
        }
    }
    if current
        .providers
        .iter()
        .any(|p| p.copilot.is_some() && !proposed.providers.iter().any(|next| next.id == p.id))
    {
        return Err("请使用 Copilot 的断开登录入口移除此来源".into());
    }
    Ok(())
}
fn required(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() < 16384)
        .map(str::to_owned)
        .ok_or_else(|| "GitHub/Copilot 响应缺少必要字段".into())
}
async fn read_json(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => AUTH_EXPIRED.into(),
            403 => "Copilot 权限不足：请检查个人订阅、账号授权或模型策略".into(),
            429 => "GitHub/Copilot 正在限流或额度受限，请稍后再试".into(),
            _ => format!(
                "GitHub/Copilot 服务返回 HTTP {}，请稍后重试",
                status.as_u16()
            ),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "GitHub/Copilot 响应读取失败")?
    {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("GitHub/Copilot 响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "GitHub/Copilot 未返回有效 JSON".into())
}
pub fn validate_endpoint(raw: &str) -> Result<(), String> {
    let u = url::Url::parse(raw).map_err(|_| "Copilot 地址无效")?;
    let host = u.host_str().unwrap_or("");
    if u.scheme() != "https"
        || !(host == "githubcopilot.com" || host.ends_with(".githubcopilot.com"))
        || u.port().is_some_and(|p| p != 443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || !matches!(u.path(), "" | "/")
    {
        return Err("拒绝向非受信任的 Copilot 地址发送凭据".into());
    }
    Ok(())
}
pub fn headers(token: &str) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut h = HeaderMap::new();
    for (name, value) in [
        ("authorization", format!("Bearer {token}")),
        ("editor-version", COPILOT_EDITOR_VERSION.into()),
        ("editor-plugin-version", COPILOT_PLUGIN_VERSION.into()),
        ("copilot-integration-id", COPILOT_INTEGRATION_ID.into()),
        ("user-agent", COPILOT_USER_AGENT.into()),
        ("x-github-api-version", COPILOT_API_VERSION.into()),
        ("openai-intent", "conversation-agent".into()),
        ("x-initiator", "user".into()),
        ("x-interaction-type", "conversation-agent".into()),
    ] {
        h.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| "认证头名称无效")?,
            HeaderValue::from_str(&value).map_err(|_| "认证材料格式无效")?,
        );
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    h.insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).map_err(|_| "请求标识无效")?,
    );
    h.insert(
        "x-agent-task-id",
        HeaderValue::from_str(&request_id).map_err(|_| "请求标识无效")?,
    );
    h.insert(
        "x-vscode-user-agent-library-version",
        HeaderValue::from_static("electron-fetch"),
    );
    Ok(h)
}
pub fn inference_url(base: &str, endpoint: &str) -> Result<String, String> {
    let origin = base
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or(base.trim_end_matches('/'));
    validate_endpoint(origin)?;
    let path = endpoint.split('?').next().unwrap_or(endpoint);
    let path = path.strip_prefix("/v1").unwrap_or(path);
    if !matches!(path, "/chat/completions" | "/responses" | "/messages") {
        return Err("Copilot 第一版只支持对话、Messages 与 Responses，不支持此独立端点".into());
    }
    Ok(format!(
        "{origin}{}",
        if path == "/messages" {
            "/v1/messages"
        } else {
            path
        }
    ))
}
pub fn upstream_id(model: &Model) -> Result<&str, String> {
    model
        .id
        .strip_prefix("copilot/")
        .filter(|id| !id.is_empty())
        .ok_or("Copilot 模型路由 ID 无效".into())
}
fn parse_models(value: &Value) -> Result<Vec<Model>, String> {
    let data = value["data"]
        .as_array()
        .filter(|a| a.len() <= 2000)
        .ok_or("Copilot 模型目录格式无效")?;
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in data {
        let Some(id) = raw["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 190 && !id.chars().any(char::is_control))
        else {
            continue;
        };
        if !seen.insert(id)
            || raw["model_picker_enabled"] != true
            || raw
                .pointer("/policy/state")
                .and_then(Value::as_str)
                .is_some_and(|s| !matches!(s, "enabled" | "unconfigured"))
        {
            continue;
        }
        let endpoints = raw["supported_endpoints"].as_array();
        let supports = |path: &str| {
            endpoints.is_some_and(|a| {
                a.iter().any(|e| {
                    e.as_str()
                        .is_some_and(|s| s.trim_start_matches("/v1") == path)
                })
            })
        };
        let protocol = if supports("/responses") {
            Protocol::OpenaiResponses
        } else if supports("/chat/completions")
            || (endpoints.is_none()
                && raw.pointer("/capabilities/type").and_then(Value::as_str) == Some("chat"))
        {
            Protocol::OpenaiChat
        } else if supports("/messages") {
            Protocol::Anthropic
        } else {
            continue;
        };
        let flags = raw.pointer("/capabilities/supports");
        let flag = |k: &str| flags.and_then(|f| f[k].as_bool());
        let count = |k: &str| {
            raw.pointer(&format!("/capabilities/limits/{k}"))
                .and_then(Value::as_u64)
                .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
        };
        let vision = flag("vision");
        models.push(Model {
            id: format!("copilot/{id}"),
            name: raw["name"].as_str().map(str::to_owned),
            protocol_override: Some(protocol),
            context_window: count("max_context_window_tokens"),
            max_output_tokens: count("max_output_tokens"),
            image: vision == Some(true),
            input_modalities: vision.map(|v| {
                if v {
                    vec!["text".into(), "image".into()]
                } else {
                    vec!["text".into()]
                }
            }),
            tools: flag("tool_calls"),
            reasoning: flag("reasoning"),
            metadata: raw.clone(),
            enabled: true,
            ..Model::default()
        });
    }
    if models.is_empty() {
        return Err("当前账号没有可路由的 Copilot 对话模型；请检查订阅、模型策略或服务接口变化。已有目录未覆盖。".into());
    }
    Ok(models)
}
