//! Azure AD OAuth authentication manager.
//!
//! Uses the OAuth 2.0 Client Credentials flow to obtain access tokens
//! for Azure AI Foundry / Azure OpenAI resources. Tokens are cached
//! and refreshed automatically before expiry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const TOKEN_REFRESH_BUFFER_SECS: i64 = 120;
const DEFAULT_TOKEN_LIFETIME_SECS: i64 = 3600;

#[derive(Debug, thiserror::Error)]
pub enum AzureAdError {
    #[error("Azure AD Token 获取失败: {0}")]
    TokenFetchFailed(String),
    #[error("网络错误: {0}")]
    NetworkError(String),
    #[error("解析错误: {0}")]
    ParseError(String),
    #[error("配置错误: {0}")]
    ConfigError(String),
}

impl From<reqwest::Error> for AzureAdError {
    fn from(err: reqwest::Error) -> Self {
        Self::NetworkError(err.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<i64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AzureAdConfig {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: i64,
}

impl CachedToken {
    fn is_valid(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        now < self.expires_at - TOKEN_REFRESH_BUFFER_SECS
    }
}

/// Azure AD Client Credentials token manager.
///
/// Caches tokens per (tenant_id, client_id) pair and refreshes them
/// automatically before expiry.
pub struct AzureAdManager {
    client: reqwest::Client,
    tokens: RwLock<HashMap<String, CachedToken>>,
}

impl AzureAdManager {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            tokens: RwLock::new(HashMap::new()),
        }
    }

    fn cache_key(config: &AzureAdConfig) -> String {
        format!("{}:{}", config.tenant_id, config.client_id)
    }

    fn token_endpoint(config: &AzureAdConfig) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            config.tenant_id
        )
    }

    /// Get an access token, using the cache when possible.
    pub async fn get_access_token(
        &self,
        config: &AzureAdConfig,
    ) -> Result<String, AzureAdError> {
        let key = Self::cache_key(config);

        // Fast path: return cached token if still valid
        {
            let tokens = self.tokens.read().await;
            if let Some(cached) = tokens.get(&key) {
                if cached.is_valid() {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Slow path: fetch new token
        let token = self.fetch_token(config).await?;
        let expires_in = token.expires_in.unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);
        let expires_at = chrono::Utc::now().timestamp() + expires_in;

        let mut tokens = self.tokens.write().await;
        tokens.insert(
            key,
            CachedToken {
                access_token: token.access_token.clone(),
                expires_at,
            },
        );

        Ok(token.access_token)
    }

    /// Invalidate cached token for a config (e.g. after auth error).
    pub async fn invalidate(&self, config: &AzureAdConfig) {
        let key = Self::cache_key(config);
        let mut tokens = self.tokens.write().await;
        tokens.remove(&key);
    }

    async fn fetch_token(
        &self,
        config: &AzureAdConfig,
    ) -> Result<TokenResponse, AzureAdError> {
        if config.tenant_id.is_empty() || config.client_id.is_empty() || config.client_secret.is_empty() {
            return Err(AzureAdError::ConfigError(
                "Azure AD 配置不完整（需要 tenant_id, client_id, client_secret）".to_string(),
            ));
        }

        let endpoint = Self::token_endpoint(config);
        let scope = "https://cognitiveservices.azure.com/.default";

        let response = self
            .client
            .post(&endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", config.client_id.as_str()),
                ("client_secret", config.client_secret.as_str()),
                ("scope", scope),
            ])
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(AzureAdError::TokenFetchFailed(format!(
                "HTTP {status}: {body}"
            )));
        }

        serde_json::from_str(&body).map_err(|e| {
            AzureAdError::ParseError(format!("解析 token 响应失败: {e}"))
        })
    }
}

/// Thread-safe wrapper.
pub type SharedAzureAdManager = Arc<AzureAdManager>;

/// Create a new shared Azure AD manager.
pub fn new_shared(client: reqwest::Client) -> SharedAzureAdManager {
    Arc::new(AzureAdManager::new(client))
}

/// 一次性取 token，不缓存。目录发现等低频路径使用。
pub async fn fetch_access_token(
    client: &reqwest::Client,
    config: &AzureAdConfig,
) -> Result<String, AzureAdError> {
    let manager = AzureAdManager::new(client.clone());
    manager.get_access_token(config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key() {
        let config = AzureAdConfig {
            tenant_id: "tenant-1".to_string(),
            client_id: "client-1".to_string(),
            client_secret: "secret".to_string(),
        };
        assert_eq!(AzureAdManager::cache_key(&config), "tenant-1:client-1");
    }

    #[test]
    fn test_token_endpoint() {
        let config = AzureAdConfig {
            tenant_id: "my-tenant".to_string(),
            client_id: "my-client".to_string(),
            client_secret: "secret".to_string(),
        };
        assert_eq!(
            AzureAdManager::token_endpoint(&config),
            "https://login.microsoftonline.com/my-tenant/oauth2/v2.0/token"
        );
    }

    #[test]
    fn test_cached_token_validity() {
        let future = CachedToken {
            access_token: "tok".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
        };
        assert!(future.is_valid());

        let past = CachedToken {
            access_token: "tok".to_string(),
            expires_at: chrono::Utc::now().timestamp() - 10,
        };
        assert!(!past.is_valid());

        // Within refresh buffer should be invalid
        let near_expiry = CachedToken {
            access_token: "tok".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 60,
        };
        assert!(!near_expiry.is_valid());
    }

    #[test]
    fn test_config_error_on_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = AzureAdManager::new(reqwest::Client::new());
        let config = AzureAdConfig {
            tenant_id: "".to_string(),
            client_id: "c".to_string(),
            client_secret: "s".to_string(),
        };
        let result = rt.block_on(manager.get_access_token(&config));
        assert!(result.is_err());
    }
}
