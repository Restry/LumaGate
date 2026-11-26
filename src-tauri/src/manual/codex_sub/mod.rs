//! Personal OpenAI ChatGPT / Codex subscription adapter (OAuth + local `~/.codex/auth.json` import).
//! Secrets never cross the UI seam; tokens are stored privately under `~/.lumagate/codex/`.
mod commands;

use super::catalog::{Model, Protocol, Source};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
pub use commands::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub const PROVIDER_ID: &str = "codex-subscription";
pub const API_ROOT: &str = "https://chatgpt.com/backend-api/codex";
pub const LOGIN_URL: &str = "https://auth.openai.com/codex/device";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ORIGIN: &str = "https://auth.openai.com";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
pub const ORIGINATOR: &str = "codex_cli_rs";
pub const CLIENT_VERSION: &str = "0.171.0";
const MAX_BODY: usize = 4 * 1024 * 1024;
const AUTH_EXPIRED: &str = "Codex 订阅授权已失效，请重新导入本机登录或浏览器授权";

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
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    source: String, // "local_codex" | "device_code"
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    id_token: Option<String>,
    expires_at: i64,
    grant_id: String,
}

#[derive(Default)]
struct State {
    credential: Option<Credential>,
    load_error: bool,
    reauth_required: bool,
}

struct DeviceFlow {
    id: String,
    device_auth_id: String,
    user_code: String,
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
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan: Option<String>,
    pub source: Option<String>,
    pub local_available: bool,
    pub local_email: Option<String>,
    pub message: Option<String>,
}

struct LocalCodexAuth {
    account_id: String,
    email: Option<String>,
    plan: Option<String>,
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_at: i64,
}

#[derive(Default, Deserialize)]
struct JwtClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    openai_auth: Option<OpenAiAuthClaim>,
    #[serde(default, rename = "https://api.openai.com/profile")]
    openai_profile: Option<OpenAiProfileClaim>,
}

#[derive(Default, Deserialize)]
struct OpenAiAuthClaim {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
}

#[derive(Default, Deserialize)]
struct OpenAiProfileClaim {
    #[serde(default)]
    email: Option<String>,
}

pub struct Manager {
    path: PathBuf,
    home: PathBuf,
    client: reqwest::Client,
    state: Mutex<State>,
    device: Mutex<Option<DeviceFlow>>,
    auth_origin: String,
    api_root: String,
}

struct Authorized {
    credential: Credential,
    revision: u64,
}

enum Poll {
    Pending(u64),
    Ready(Authorized),
}

fn decode_jwt_claims(token: &str) -> Option<JwtClaims> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_identity(
    access_token: &str,
    id_token: Option<&str>,
    fallback_account_id: Option<&str>,
) -> (String, Option<String>, Option<String>, i64) {
    let id_claims = id_token.and_then(decode_jwt_claims);
    let access_claims = decode_jwt_claims(access_token);
    let account_id = fallback_account_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            id_claims.as_ref().and_then(|c| {
                c.chatgpt_account_id.clone().or_else(|| {
                    c.openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
            })
        })
        .or_else(|| {
            access_claims.as_ref().and_then(|c| {
                c.chatgpt_account_id.clone().or_else(|| {
                    c.openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
            })
        })
        .unwrap_or_default();
    let email = id_claims
        .as_ref()
        .and_then(|c| {
            c.email
                .clone()
                .or_else(|| c.openai_profile.as_ref().and_then(|p| p.email.clone()))
        })
        .or_else(|| {
            access_claims.as_ref().and_then(|c| {
                c.email
                    .clone()
                    .or_else(|| c.openai_profile.as_ref().and_then(|p| p.email.clone()))
            })
        });
    let plan = access_claims
        .as_ref()
        .and_then(|c| {
            c.openai_auth
                .as_ref()
                .and_then(|a| a.chatgpt_plan_type.clone())
        })
        .or_else(|| {
            id_claims.as_ref().and_then(|c| {
                c.openai_auth
                    .as_ref()
                    .and_then(|a| a.chatgpt_plan_type.clone())
            })
        });
    let expires_at = access_claims
        .as_ref()
        .and_then(|c| c.exp)
        .unwrap_or_else(|| chrono::Utc::now().timestamp() + 3600);
    (account_id, email, plan, expires_at)
}

fn read_local_codex_auth(home: &Path) -> Option<LocalCodexAuth> {
    let path = home.join(".codex/auth.json");
    let meta = fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > 256 * 1024 {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let tokens = value.get("tokens")?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let id_token = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let raw_account_id = tokens.get("account_id").and_then(Value::as_str);
    let (account_id, email, plan, expires_at) =
        extract_identity(&access_token, id_token.as_deref(), raw_account_id);
    if account_id.is_empty() {
        return None;
    }
    Some(LocalCodexAuth {
        account_id,
        email,
        plan,
        access_token,
        refresh_token,
        id_token,
        expires_at,
    })
}

impl Manager {
    pub fn new(root: PathBuf) -> Self {
        Self::at(
            root,
            crate::config::get_home_dir(),
            AUTH_ORIGIN.into(),
            API_ROOT.into(),
        )
    }

    pub fn at(root: PathBuf, home: PathBuf, auth_origin: String, api_root: String) -> Self {
        let path = root.join("credentials.json");
        let loaded = if path.exists() {
            fs::read(&path)
                .ok()
                .filter(|b| b.len() < 128 * 1024)
                .and_then(|b| serde_json::from_slice::<Credential>(&b).ok())
                .filter(|c| {
                    c.version == 1
                        && !c.access_token.is_empty()
                        && !c.refresh_token.is_empty()
                        && !c.account_id.is_empty()
                        && uuid::Uuid::parse_str(&c.grant_id).is_ok()
                })
        } else {
            None
        };
        let failed = path.exists() && loaded.is_none();
        Self {
            path,
            home,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Codex OAuth HTTP client"),
            state: Mutex::new(State {
                credential: loaded,
                load_error: failed,
                reauth_required: false,
            }),
            device: Mutex::new(None),
            auth_origin,
            api_root,
        }
    }

    pub async fn status(&self, binding: Option<&Binding>) -> Status {
        let local = read_local_codex_auth(&self.home);
        let state = self.state.lock().await;
        let matches = state.credential.as_ref().filter(|c| {
            binding.is_some_and(|b| c.account_id == b.account_id && c.grant_id == b.grant_id)
        });
        Status {
            connected: matches.is_some() && !state.reauth_required,
            email: matches.and_then(|c| c.email.clone()),
            account_id: matches.map(|c| c.account_id.clone()),
            plan: matches.and_then(|c| c.plan.clone()),
            source: matches.map(|c| c.source.clone()),
            local_available: local.is_some(),
            local_email: local.and_then(|l| l.email),
            message: if state.load_error {
                Some("本机 Codex 订阅凭据无法读取，请断开后重新连接。".into())
            } else if state.reauth_required || (binding.is_some() && matches.is_none()) {
                Some(AUTH_EXPIRED.into())
            } else {
                None
            },
        }
    }

    pub async fn start(&self, revision: u64) -> Result<Challenge, String> {
        if self.state.lock().await.load_error {
            return Err("Codex 凭据文件无法读取，请先断开后再重试".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        *self.device.lock().await = Some(DeviceFlow {
            id: id.clone(),
            device_auth_id: String::new(),
            user_code: String::new(),
            expires: Instant::now() + Duration::from_secs(900),
            interval: 5,
            next_poll: Instant::now(),
            polling: true,
            revision,
        });
        let result = read_json(
            self.client
                .post(format!(
                    "{}/api/accounts/deviceauth/usercode",
                    self.auth_origin
                ))
                .header("accept", "application/json")
                .header("user-agent", "LumaGate-Codex")
                .json(&json!({ "client_id": CLIENT_ID }))
                .send()
                .await
                .map_err(|_| "无法连接 OpenAI 授权服务，请检查网络或代理设置")?,
        )
        .await?;
        let device_auth_id = required(&result, "device_auth_id")?;
        let user_code = required(&result, "user_code")?;
        let expires = result["expires_in"]
            .as_u64()
            .filter(|n| (1..=1800).contains(n))
            .unwrap_or(900);
        let interval = result["interval"]
            .as_u64()
            .or_else(|| result["interval"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(5)
            .clamp(5, 60);
        let mut flow = self.device.lock().await;
        let d = flow.as_mut().filter(|d| d.id == id).ok_or("授权已取消")?;
        d.device_auth_id = device_auth_id;
        d.user_code = user_code.clone();
        d.expires = Instant::now() + Duration::from_secs(expires);
        d.interval = interval;
        d.next_poll = Instant::now() + Duration::from_secs(interval);
        d.polling = false;
        Ok(Challenge {
            flow_id: id,
            user_code,
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
        let (device_auth_id, user_code, revision) = {
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
            (d.device_auth_id.clone(), d.user_code.clone(), d.revision)
        };
        let attempt = self
            .poll_remote(&device_auth_id, &user_code, revision)
            .await;
        let mut all = self.device.lock().await;
        let d = all.as_mut().filter(|d| d.id == id).ok_or("授权已取消")?;
        d.polling = false;
        match attempt {
            Ok(None) => Ok(Poll::Pending(d.interval)),
            Ok(Some(auth)) => Ok(Poll::Ready(auth)),
            Err(error) => {
                *all = None;
                Err(error)
            }
        }
    }

    async fn poll_remote(
        &self,
        device_auth_id: &str,
        user_code: &str,
        revision: u64,
    ) -> Result<Option<Authorized>, String> {
        let response = self
            .client
            .post(format!("{}/api/accounts/deviceauth/token", self.auth_origin))
            .header("accept", "application/json")
            .header("user-agent", "LumaGate-Codex")
            .json(&json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await
            .map_err(|_| "OpenAI 授权状态查询失败，请重试")?;
        let status = response.status();
        if status.as_u16() == 403 || status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("OpenAI 授权失败（HTTP {}）", status.as_u16()));
        }
        let poll_data = read_json(response).await?;
        let auth_code = required(&poll_data, "authorization_code")?;
        let verifier = required(&poll_data, "code_verifier")?;
        let token_data = read_json(
            self.client
                .post(format!("{}/oauth/token", self.auth_origin))
                .header("accept", "application/json")
                .header("user-agent", "LumaGate-Codex")
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("code", auth_code.as_str()),
                    ("redirect_uri", DEVICE_REDIRECT_URI),
                    ("client_id", CLIENT_ID),
                    ("code_verifier", verifier.as_str()),
                ])
                .send()
                .await
                .map_err(|_| "换取 OpenAI Token 失败，请重试")?,
        )
        .await?;
        let access_token = required(&token_data, "access_token")?;
        let refresh_token = required(&token_data, "refresh_token")?;
        let id_token = token_data
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (account_id, email, plan, exp) =
            extract_identity(&access_token, id_token.as_deref(), None);
        if account_id.is_empty() {
            return Err("无法从 OpenAI 授权响应读取账号 ID".into());
        }
        let expires_at = token_data
            .get("expires_in")
            .and_then(Value::as_i64)
            .map(|secs| chrono::Utc::now().timestamp() + secs)
            .unwrap_or(exp);
        let credential = Credential {
            version: 1,
            account_id,
            email,
            plan,
            source: "device_code".into(),
            access_token,
            refresh_token,
            id_token,
            expires_at,
            grant_id: uuid::Uuid::new_v4().to_string(),
        };
        Ok(Some(Authorized {
            credential,
            revision,
        }))
    }

    async fn refresh_token(&self, current: &Credential) -> Result<Credential, String> {
        let response = self
            .client
            .post(format!("{}/oauth/token", self.auth_origin))
            .header("accept", "application/json")
            .header("user-agent", "LumaGate-Codex")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", current.refresh_token.as_str()),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .map_err(|_| "刷新 Codex 订阅令牌失败（网络错误）")?;
        if !response.status().is_success() {
            // If this credential came from local ~/.codex/auth.json, local Codex CLI may have
            // rotated the refresh token. Adopt the fresh token on disk for the same account.
            if let Some(local) = read_local_codex_auth(&self.home) {
                if local.account_id == current.account_id
                    && (local.access_token != current.access_token
                        || local.refresh_token != current.refresh_token)
                {
                    let adopted = Credential {
                        version: 1,
                        account_id: local.account_id,
                        email: local.email.or_else(|| current.email.clone()),
                        plan: local.plan.or_else(|| current.plan.clone()),
                        source: current.source.clone(),
                        access_token: local.access_token,
                        refresh_token: local.refresh_token,
                        id_token: local.id_token.or_else(|| current.id_token.clone()),
                        expires_at: local.expires_at,
                        grant_id: current.grant_id.clone(),
                    };
                    return Ok(adopted);
                }
            }
            return Err(AUTH_EXPIRED.into());
        }
        let data = read_json(response).await?;
        let access_token = required(&data, "access_token")?;
        let refresh_token = data
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| current.refresh_token.clone());
        let id_token = data
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| current.id_token.clone());
        let (parsed_account, email, plan, exp) = extract_identity(
            &access_token,
            id_token.as_deref(),
            Some(&current.account_id),
        );
        let expires_at = data
            .get("expires_in")
            .and_then(Value::as_i64)
            .map(|secs| chrono::Utc::now().timestamp() + secs)
            .unwrap_or(exp);
        Ok(Credential {
            version: 1,
            account_id: if parsed_account.is_empty() {
                current.account_id.clone()
            } else {
                parsed_account
            },
            email: email.or_else(|| current.email.clone()),
            plan: plan.or_else(|| current.plan.clone()),
            source: current.source.clone(),
            access_token,
            refresh_token,
            id_token,
            expires_at,
            grant_id: current.grant_id.clone(),
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
        let bytes = serde_json::to_vec(credential)
            .map_err(|_| "无法序列化 Codex 订阅凭据".to_string())?;
        crate::config::atomic_write_private(&self.path, &bytes)
            .map_err(|_| "无法保存 Codex 订阅凭据，请检查磁盘和私有目录权限".to_string())
    }

    pub async fn resolve(&self, binding: &Binding) -> Result<(String, String), String> {
        let mut state = self.state.lock().await;
        let mut credential = state
            .credential
            .as_ref()
            .filter(|c| c.account_id == binding.account_id && c.grant_id == binding.grant_id)
            .cloned()
            .ok_or(AUTH_EXPIRED)?;

        let now = chrono::Utc::now().timestamp();
        // If synced from local Codex CLI, transparently adopt a fresher token from ~/.codex/auth.json
        // before attempting a refresh so we never compete with Codex CLI over refresh_token rotation.
        if credential.source == "local_codex" {
            if let Some(local) = read_local_codex_auth(&self.home) {
                if local.account_id == credential.account_id
                    && local.expires_at > credential.expires_at
                {
                    credential.access_token = local.access_token;
                    credential.refresh_token = local.refresh_token;
                    credential.id_token = local.id_token.or(credential.id_token);
                    credential.expires_at = local.expires_at;
                    credential.email = local.email.or(credential.email);
                    credential.plan = local.plan.or(credential.plan);
                    let _ = self.persist(&credential);
                    state.credential = Some(credential.clone());
                    state.reauth_required = false;
                }
            }
        }

        if credential.expires_at - now < 60 {
            match self.refresh_token(&credential).await {
                Ok(refreshed) => {
                    let _ = self.persist(&refreshed);
                    state.credential = Some(refreshed.clone());
                    state.reauth_required = false;
                    credential = refreshed;
                }
                Err(error) => {
                    if error == AUTH_EXPIRED {
                        state.reauth_required = true;
                    }
                    return Err(error);
                }
            }
        }
        Ok((credential.access_token, credential.account_id))
    }

    pub async fn discover(&self, source: &Source) -> Result<Vec<Model>, String> {
        let binding = source.codex.as_ref().ok_or("不是 Codex 订阅来源")?;
        for attempt in 0..2 {
            let (token, account_id) = self.resolve(binding).await?;
            let response = self
                .client
                .get(format!("{}/models", self.api_root.trim_end_matches('/')))
                .query(&[("client_version", CLIENT_VERSION)])
                .headers(headers(&token, &account_id)?)
                .send()
                .await
                .map_err(|_| "无法读取 Codex 订阅模型目录，请检查网络或代理")?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let mut state = self.state.lock().await;
                if let Some(cred) = state.credential.as_mut() {
                    cred.expires_at = 0;
                }
                continue;
            }
            let value = read_json(response).await?;
            let mut models = parse_models(&value)?;
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
        let binding = source.codex.as_ref().ok_or("Codex 订阅绑定缺失")?;
        let (token, account_id) = self.resolve(binding).await?;
        let slug = upstream_id(model)?;
        let version = super::catalog::deployment_version(source, model);
        let config = if app == "claude" {
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": self.api_root,
                    "ANTHROPIC_AUTH_TOKEN": token,
                    "ANTHROPIC_MODEL": slug
                },
                "api_format": "openai_responses",
                "manual_upstream_model": slug,
                "manual_credential_version": version,
                "manual_codex_oauth_token": token,
                "manual_codex_account_id": account_id,
            })
        } else {
            json!({
                "base_url": self.api_root,
                "model": slug,
                "auth": { "OPENAI_API_KEY": token },
                "api_format": "openai_responses",
                "manual_upstream_model": slug,
                "manual_credential_version": version,
                "manual_codex_oauth_token": token,
                "manual_codex_account_id": account_id,
            })
        };
        let meta = if app == "claude" {
            Some(crate::provider::ProviderMeta {
                provider_type: Some("codex_oauth".into()),
                ..Default::default()
            })
        } else {
            None
        };
        let mut provider = crate::provider::Provider::with_id(
            source.id.clone(),
            source.name.clone(),
            config,
            None,
        );
        provider.meta = meta;
        Ok(provider)
    }

    pub async fn test(&self, source: &Source, model: &Model) -> Result<(), String> {
        let binding = source.codex.as_ref().ok_or("Codex 订阅绑定缺失")?;
        let (token, account_id) = self.resolve(binding).await?;
        let slug = upstream_id(model)?;
        let payload = json!({
            "model": slug,
            "instructions": "You are a helpful assistant.",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Reply with OK." }]
            }],
            "store": false,
            "stream": true
        });
        let mut response = self
            .client
            .post(format!("{}/responses", self.api_root.trim_end_matches('/')))
            .headers(headers(&token, &account_id)?)
            .header("accept", "text/event-stream")
            .json(&payload)
            .send()
            .await
            .map_err(|_| "Codex 订阅请求失败（网络或 TLS）")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = body.chars().take(180).collect::<String>();
            return Err(format!("HTTP {}：{}", status.as_u16(), detail));
        }
        let mut text = String::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "读取 Codex 响应流失败")?
        {
            text.push_str(&String::from_utf8_lossy(&chunk));
            if text.contains("response.completed") {
                return Ok(());
            }
            if text.contains("response.failed") {
                return Err("上游返回 response.failed".into());
            }
            if text.len() > 512 * 1024 {
                break;
            }
        }
        if text.contains("response.completed") || text.contains("output_text") {
            Ok(())
        } else {
            Err("未收到完整响应，请确认当前订阅支持该模型".into())
        }
    }
}

pub fn headers(token: &str, account_id: &str) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut h = HeaderMap::new();
    for (name, value) in [
        ("authorization", format!("Bearer {token}")),
        ("chatgpt-account-id", account_id.to_string()),
        ("originator", ORIGINATOR.to_string()),
        ("version", CLIENT_VERSION.to_string()),
        ("user-agent", format!("codex_cli_rs/{CLIENT_VERSION}")),
    ] {
        h.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| "认证头名称无效")?,
            HeaderValue::from_str(&value).map_err(|_| "认证材料格式无效")?,
        );
    }
    Ok(h)
}

pub fn inference_url(base: &str, endpoint: &str) -> Result<String, String> {
    let base = base.trim_end_matches('/');
    let path = endpoint.split('?').next().unwrap_or(endpoint);
    let path = path
        .strip_prefix("/backend-api/codex")
        .or_else(|| path.strip_prefix("/v1"))
        .unwrap_or(path);
    match path {
        "/responses" => Ok(format!("{base}/responses")),
        "/responses/compact" => Ok(format!("{base}/responses/compact")),
        _ => Err("Codex 订阅仅支持 Responses 接口（/v1/responses 或 /backend-api/codex/responses）".into()),
    }
}

/// ChatGPT Codex 消费端接口强制要求 `store: false`、`stream: true` 和非空 `instructions`，
/// 且拒绝 `max_output_tokens`、`previous_response_id` 等平台侧参数。
pub fn normalize_responses_request(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    obj.insert("store".into(), json!(false));
    obj.insert("stream".into(), json!(true));
    let needs_instructions = obj
        .get("instructions")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty());
    if needs_instructions {
        obj.insert("instructions".into(), json!("You are a helpful assistant."));
    }
    if let Some(input_str) = obj.get("input").and_then(Value::as_str).map(str::to_owned) {
        obj.insert(
            "input".into(),
            json!([{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": input_str }]
            }]),
        );
    }
    for unsupported in [
        "max_output_tokens",
        "max_completion_tokens",
        "previous_response_id",
        "temperature",
        "top_p",
    ] {
        obj.remove(unsupported);
    }
}

pub fn upstream_id(model: &Model) -> Result<&str, String> {
    model
        .id
        .strip_prefix("codex/")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Codex 订阅模型路由 ID 无效".into())
}

pub fn validate_document_edit(
    current: &super::catalog::Document,
    proposed: &super::catalog::Document,
) -> Result<(), String> {
    for source in &proposed.providers {
        let previous = current.providers.iter().find(|p| p.id == source.id);
        if source.codex.as_ref() != previous.and_then(|p| p.codex.as_ref()) {
            return Err("Codex 订阅绑定只能通过登录或断开入口修改".into());
        }
        if source.codex.is_some() {
            let old = previous.ok_or("Codex 订阅来源缺失")?;
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
                return Err("Codex 订阅模型目录只能通过刷新获取，这里仅允许启停模型".into());
            }
        }
    }
    if current
        .providers
        .iter()
        .any(|p| p.codex.is_some() && !proposed.providers.iter().any(|next| next.id == p.id))
    {
        return Err("请使用 Codex 订阅的断开登录入口移除此来源".into());
    }
    Ok(())
}

fn required(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() < 16384)
        .map(str::to_owned)
        .ok_or_else(|| format!("OpenAI 响应缺少必要字段：{key}"))
}

async fn read_json(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => AUTH_EXPIRED.into(),
            403 => "Codex 订阅权限不足，请检查 ChatGPT Plus/Pro 订阅状态".into(),
            429 => "Codex 订阅正在限流或额度已达上限，请稍后再试".into(),
            _ => format!("Codex 服务返回 HTTP {}，请稍后重试", status.as_u16()),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取 Codex 服务响应失败")?
    {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("Codex 服务响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Codex 服务未返回有效 JSON".into())
}

pub fn parse_models(value: &Value) -> Result<Vec<Model>, String> {
    let entries = value
        .get("models")
        .and_then(Value::as_array)
        .or_else(|| value.get("data").and_then(Value::as_array))
        .filter(|a| a.len() <= 2000)
        .ok_or("Codex 模型目录格式无效")?;
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let any_visible = entries
        .iter()
        .any(|raw| raw.get("visibility").and_then(Value::as_str) != Some("hide"));
    for raw in entries {
        if any_visible && raw.get("visibility").and_then(Value::as_str) == Some("hide") {
            continue;
        }
        let Some(slug) = raw
            .get("slug")
            .or_else(|| raw.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= 190 && !id.chars().any(char::is_control))
        else {
            continue;
        };
        if !seen.insert(slug.to_string()) {
            continue;
        }
        let context_window = raw
            .get("context_window")
            .or_else(|| raw.get("max_context_window"))
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        let max_output_tokens = raw
            .get("max_output_tokens")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        let reasoning_levels: Option<Vec<String>> = raw
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.as_str()
                            .or_else(|| item.get("effort").and_then(Value::as_str))
                            .map(str::to_owned)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());
        let input_modalities: Option<Vec<String>> = raw
            .get("input_modalities")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());
        let image = input_modalities
            .as_ref()
            .is_some_and(|m| m.iter().any(|v| v.eq_ignore_ascii_case("image")));
        let default_reasoning_level = raw
            .get("default_reasoning_level")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let reasoning_summaries = raw
            .get("supports_reasoning_summaries")
            .and_then(Value::as_bool);
        let name = raw
            .get("display_name")
            .or_else(|| raw.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        models.push(Model {
            id: format!("codex/{slug}"),
            name,
            protocol_override: Some(Protocol::OpenaiResponses),
            context_window,
            max_output_tokens,
            reasoning: reasoning_levels.as_ref().map(|_| true),
            reasoning_levels,
            default_reasoning_level,
            reasoning_summaries,
            input_modalities,
            image,
            tools: Some(true),
            metadata: raw.clone(),
            enabled: true,
            ..Model::default()
        });
    }
    if models.is_empty() {
        return Err("当前 Codex 订阅没有可用模型，请确认订阅状态或稍后重试".into());
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_backend_catalog_and_hides_hidden_models() {
        let value = json!({
            "models": [
                {
                    "slug": "gpt-6-astra",
                    "display_name": "GPT-6 Astra",
                    "context_window": 1000000,
                    "visibility": "list",
                    "input_modalities": ["text", "image"],
                    "default_reasoning_level": "high",
                    "supports_reasoning_summaries": true,
                    "supported_reasoning_levels": [
                        { "effort": "low", "description": "fast" },
                        { "effort": "high", "description": "deep" }
                    ]
                },
                {
                    "slug": "internal-hidden",
                    "visibility": "hide"
                }
            ]
        });
        let models = parse_models(&value).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "codex/gpt-6-astra");
        assert_eq!(models[0].name.as_deref(), Some("GPT-6 Astra"));
        assert_eq!(models[0].context_window, Some(1_000_000));
        assert_eq!(models[0].protocol_override, Some(Protocol::OpenaiResponses));
        assert!(models[0].image);
        assert_eq!(
            models[0].reasoning_levels,
            Some(vec!["low".into(), "high".into()])
        );
    }

    #[test]
    fn normalizes_responses_request_for_chatgpt_backend() {
        let mut body = json!({
            "model": "gpt-6-astra",
            "input": "hello",
            "max_output_tokens": 256,
            "previous_response_id": "resp_123",
            "stream": false
        });
        normalize_responses_request(&mut body);
        assert_eq!(body["store"], json!(false));
        assert_eq!(body["stream"], json!(true));
        assert!(body["instructions"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("previous_response_id").is_none());
        assert!(body["input"].is_array());
    }

    #[test]
    fn imports_local_codex_auth_json() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "email": "user@example.com",
                "exp": chrono::Utc::now().timestamp() + 3600,
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct-test-123",
                    "chatgpt_plan_type": "pro"
                }
            }))
            .unwrap(),
        );
        let jwt = format!("eyJhbGciOiJub25lIn0.{payload}.sig");
        fs::write(
            home.join(".codex/auth.json"),
            serde_json::to_vec(&json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": jwt,
                    "refresh_token": "rt_fixture",
                    "id_token": jwt,
                    "account_id": "acct-test-123"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let parsed = read_local_codex_auth(home).unwrap();
        assert_eq!(parsed.account_id, "acct-test-123");
        assert_eq!(parsed.email.as_deref(), Some("user@example.com"));
        assert_eq!(parsed.plan.as_deref(), Some("pro"));
    }

    #[tokio::test]
    async fn gateway_serves_codex_models_and_forwards_to_codex_subscription() {
        use axum::{
            http::HeaderMap,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use std::sync::{Arc, Mutex as StdMutex};

        let captured = Arc::new(StdMutex::new(Vec::<(String, String, Value)>::new()));
        let captured_clone = captured.clone();
        let upstream_app = Router::new()
            .route(
                "/models",
                get(|| async {
                    Json(json!({
                        "models": [{
                            "slug": "gpt-6-astra",
                            "display_name": "GPT-6 Astra",
                            "context_window": 1000000,
                            "visibility": "list",
                            "input_modalities": ["text", "image"],
                            "supported_reasoning_levels": [{"effort": "high"}]
                        }]
                    }))
                }),
            )
            .route(
                "/responses",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let captured = captured_clone.clone();
                    async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        let acct = headers
                            .get("chatgpt-account-id")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        captured.lock().unwrap().push((auth, acct, body));
                        (
                            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"gpt-6-astra\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"OK\"}]}]}}\n\n",
                        )
                            .into_response()
                    }
                }),
            );
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_base = format!("http://{}", upstream_listener.local_addr().unwrap());
        let upstream_task = tokio::spawn(async move {
            axum::serve(upstream_listener, upstream_app).await.unwrap();
        });

        let tmp = tempfile::tempdir().unwrap();
        let manager = Arc::new(Manager::at(
            tmp.path().join("codex"),
            tmp.path().to_path_buf(),
            upstream_base.clone(),
            upstream_base.clone(),
        ));
        let grant_id = uuid::Uuid::new_v4().to_string();
        manager.state.lock().await.credential = Some(Credential {
            version: 1,
            account_id: "acct_live_1".into(),
            email: Some("owner@example.com".into()),
            plan: Some("pro".into()),
            source: "local_codex".into(),
            access_token: "tok_live_secret".into(),
            refresh_token: "rt_live".into(),
            id_token: None,
            expires_at: chrono::Utc::now().timestamp() + 3600,
            grant_id: grant_id.clone(),
        });

        let binding = Binding {
            account_id: "acct_live_1".into(),
            grant_id,
        };
        let mut source = Source {
            id: PROVIDER_ID.into(),
            name: "Codex 订阅".into(),
            base_url: API_ROOT.into(),
            key_env: String::new(),
            key_ref: None,
            azure_tenant_id: None,
            azure_client_id: None,
            copilot: None,
            codex: Some(binding),
            protocol: Protocol::OpenaiResponses,
            enabled: true,
            cost_estimation_enabled: true,
            models: vec![],
        };
        source.models = manager.discover(&source).await.unwrap();
        assert_eq!(source.models.len(), 1);
        assert_eq!(source.models[0].id, "codex/gpt-6-astra");

        let db = Arc::new(crate::database::Database::memory().unwrap());
        let mut doc = super::super::catalog::Document {
            providers: vec![source],
            ..Default::default()
        };
        doc.save(&db).unwrap();

        let server = crate::proxy::server::ProxyServer::new(
            crate::proxy::types::ProxyConfig {
                listen_address: "127.0.0.1".into(),
                listen_port: 0,
                ..Default::default()
            },
            db.clone(),
            None,
        )
        .with_manual_codex(manager.clone());
        let info = server.start().await.unwrap();
        let gw_base = format!("http://127.0.0.1:{}", info.port);

        let client = reqwest::Client::new();
        // 1. Live /backend-api/codex/models returns codex/ prefixed slug
        let catalog: Value = client
            .get(format!("{gw_base}/backend-api/codex/models"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            catalog["models"][0]["slug"].as_str(),
            Some("codex/gpt-6-astra")
        );

        // 2. Live /backend-api/codex/responses forwards to Codex subscription with normalized payload & headers
        let resp = client
            .post(format!("{gw_base}/backend-api/codex/responses"))
            .json(&json!({
                "model": "codex/gpt-6-astra",
                "input": "hello",
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let text = resp.text().await.unwrap();
        assert!(text.contains("response.completed"));

        let seen = captured.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "Bearer tok_live_secret");
        assert_eq!(seen[0].1, "acct_live_1");
        assert_eq!(seen[0].2["model"], json!("gpt-6-astra"));
        assert_eq!(seen[0].2["store"], json!(false));

        server.stop().await.unwrap();
        upstream_task.abort();
    }
}
