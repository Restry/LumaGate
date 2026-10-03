//! Listener preferences are frozen per running server. Named credentials are read per request.
pub mod access_keys;
use super::{http_error, logs, Database, PORT};
use access_keys::{Identity, Registry};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

pub const SETTINGS_KEY: &str = "manual_gateway_network_v1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub revision: u64,
    pub listen_address: String,
    pub listen_port: u16,
    // Legacy v1 tag remains read-only. An existing (even empty) key registry takes precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_key_tag: Option<Vec<u8>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    pub revision: u64,
    pub listen_address: String,
    pub listen_port: u16,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub revision: u64,
    pub listen_address: String,
    pub listen_port: u16,
    pub allow_lan: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub saved: View,
    pub active: Option<View>,
    pub restart_required: bool,
    pub lan_addresses: Vec<String>,
    pub address_error: Option<String>,
    pub access_keys: access_keys::KeyList,
}
#[derive(Clone)]
pub struct Policy {
    settings: Settings,
    db: Arc<Database>,
}
impl Policy {
    pub fn new(settings: Settings, db: Arc<Database>) -> Self {
        Self { settings, db }
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            revision: 0,
            listen_address: "127.0.0.1".into(),
            listen_port: PORT,
            access_key_tag: None,
        }
    }
}
fn credential_mac(key: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"cc-switch-manual:lan-access:v1")
        .expect("HMAC accepts this constant key length");
    mac.update(key.as_bytes());
    mac
}
impl Settings {
    pub fn load(db: &Database) -> Result<Self, String> {
        let settings = match db.get_setting(SETTINGS_KEY).map_err(|e| e.to_string())? {
            Some(value) => serde_json::from_str(&value)
                .map_err(|_| "网关网络设置损坏，请检查本地数据库；没有开放监听".to_string())?,
            None => Self::default(),
        };
        settings.validate()?;
        Ok(settings)
    }
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.listen_address.as_str(), "127.0.0.1" | "0.0.0.0") {
            return Err("监听地址仅支持 127.0.0.1（本机）或 0.0.0.0（内网）".into());
        }
        if self.listen_port == 0 {
            return Err("端口必须是 1–65535 之间的整数".into());
        }
        if self
            .access_key_tag
            .as_ref()
            .is_some_and(|tag| tag.len() != 32)
        {
            return Err("旧版内网密钥校验值无效，请检查本地设置".into());
        }
        Ok(())
    }
    pub fn save_update(db: &Database, update: Update) -> Result<Self, String> {
        let current = Self::load(db)?;
        if update.revision != current.revision {
            return Err("网关设置已变化，请刷新设置后重新保存".into());
        }
        let next = Self {
            revision: current
                .revision
                .checked_add(1)
                .ok_or("网络设置版本超出上限")?,
            listen_address: update.listen_address,
            listen_port: update.listen_port,
            access_key_tag: current.access_key_tag,
        };
        next.validate()?;
        if next.allow_lan() && !Registry::load(db)?.has_enabled() {
            return Err("允许内网访问前，请先添加并启用至少一把访问密钥".into());
        }
        db.set_setting(
            SETTINGS_KEY,
            &serde_json::to_string(&next).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(next)
    }
    pub fn allow_lan(&self) -> bool {
        self.listen_address == "0.0.0.0"
    }
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.listen_port)
    }
    pub fn sync_base(&self, active: Option<&Self>) -> Result<String, String> {
        if active.is_some_and(|active| active.listen_port != self.listen_port) {
            return Err("新端口尚未生效，请先停止并启动网关，再重新预览 Agent 同步".into());
        }
        Ok(self.base_url())
    }
    pub fn view(&self) -> View {
        View {
            revision: self.revision,
            listen_address: self.listen_address.clone(),
            listen_port: self.listen_port,
            allow_lan: self.allow_lan(),
        }
    }
    pub fn restart_required(&self, active: &Self) -> bool {
        self.listen_address != active.listen_address || self.listen_port != active.listen_port
    }
    pub fn validate_listener(&self, ip: IpAddr) -> Result<(), String> {
        self.validate()?;
        if ip.is_loopback() || (self.allow_lan() && ip == IpAddr::V4(Ipv4Addr::UNSPECIFIED)) {
            Ok(())
        } else {
            Err("网关未授权该监听地址；请在设置中明确启用内网访问".into())
        }
    }
    pub fn snapshot(&self, active: Option<&Self>, registry: &Registry) -> Snapshot {
        let effective = active.unwrap_or(self);
        let (lan_addresses, address_error) = match if_addrs::get_if_addrs() {
            Ok(interfaces) => {
                let mut addresses: Vec<_> = interfaces
                    .into_iter()
                    .filter_map(|interface| match interface.ip() {
                        IpAddr::V4(ip) if private_ipv4(ip) => {
                            Some(format!("http://{ip}:{}/v1", effective.listen_port))
                        }
                        _ => None,
                    })
                    .collect();
                addresses.sort();
                addresses.dedup();
                (addresses, None)
            }
            Err(_) => (
                vec![],
                Some("无法读取网卡地址，请在系统网络设置中查看本机内网 IPv4".into()),
            ),
        };
        Snapshot {
            saved: self.view(),
            active: active.map(Self::view),
            restart_required: active.is_some_and(|active| self.restart_required(active)),
            lan_addresses,
            address_error,
            access_keys: registry.view(),
        }
    }
}
fn private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}
fn host_allowed(headers: &HeaderMap, lan: bool) -> bool {
    if headers.get_all("host").iter().count() != 1 {
        return false;
    }
    let Some(raw) = headers.get("host").and_then(|value| value.to_str().ok()) else {
        return false;
    };
    if raw.contains('@') {
        return false;
    }
    let Ok(authority) = raw.parse::<http::uri::Authority>() else {
        return false;
    };
    let host = authority.host();
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback() || (lan && private_ipv4(ip)))
}
fn identify(headers: &HeaderMap, registry: &Registry) -> Option<Identity> {
    if headers.get_all("authorization").iter().count() > 1
        || headers.get_all("x-api-key").iter().count() > 1
    {
        return None;
    }
    let mut values = Vec::new();
    if let Some(header) = headers.get("authorization") {
        let (scheme, value) = header.to_str().ok()?.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        values.push(value);
    }
    if let Some(header) = headers.get("x-api-key") {
        values.push(header.to_str().ok()?);
    }
    let first = registry.identify(values.first()?)?;
    for value in values.iter().skip(1) {
        if registry.identify(value)?.id != first.id {
            return None;
        }
    }
    Some(first)
}
pub async fn guard(State(policy): State<Policy>, request: Request, next: Next) -> Response {
    logs::set_caller(logs::Caller::rejected());
    let headers = request.headers();
    if headers.contains_key("origin") || !host_allowed(headers, policy.settings.allow_lan()) {
        return http_error(
            StatusCode::FORBIDDEN,
            "access_denied",
            "不允许网页跨源或非本机/内网 IP 的 Host，请使用设置中的地址",
        );
    }
    let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return http_error(
            StatusCode::FORBIDDEN,
            "peer_unavailable",
            "无法确认连接来源，已拒绝请求",
        );
    };
    if !peer.ip().is_loopback()
        && (!policy.settings.allow_lan()
            || !matches!(peer.ip(), IpAddr::V4(ip) if private_ipv4(ip)))
    {
        return http_error(
            StatusCode::FORBIDDEN,
            "lan_disabled",
            "仅允许本机或已启用的 IPv4 内网访问",
        );
    }
    // Re-read only the registry, not listener preferences. Disable/delete applies to the next
    // request on an existing keep-alive connection without interrupting an in-flight stream.
    let caller = if headers.contains_key("authorization") || headers.contains_key("x-api-key") {
        let registry = match Registry::load(&policy.db) {
            Ok(registry) => registry,
            Err(_) => {
                return http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "keys_unavailable",
                    "访问密钥暂不可用，请在设置中检查",
                )
            }
        };
        identify(headers, &registry)
    } else {
        None
    };
    if !peer.ip().is_loopback() && caller.is_none() {
        return http_error(
            StatusCode::UNAUTHORIZED,
            "invalid_lan_key",
            "内网访问需要有效且启用的访问密钥",
        );
    }
    logs::set_caller(match caller {
        Some(identity) => logs::Caller::key(identity),
        None => logs::Caller::local(),
    });
    next.run(request).await
}
#[cfg(test)]
mod tests;
