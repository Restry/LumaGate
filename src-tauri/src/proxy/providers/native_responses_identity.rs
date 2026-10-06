//! Repair drifting Copilot Responses IDs without losing upstream continuation handles.
//! State is per stream; the small, account/session-scoped reverse map survives app restarts.

use bytes::Bytes;
use futures::{Stream, StreamExt};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

use crate::proxy::hyper_client::ProxyResponse;
use crate::{database::Database, provider::Provider};

pub(crate) const ENABLED: &str = "manual_native_responses_identity";

pub(crate) fn enabled(provider: &Provider, endpoint: &str) -> bool {
    provider.settings_config[ENABLED] == true
        && matches!(
            endpoint.split('?').next(),
            Some("/responses" | "/v1/responses")
        )
}

pub(crate) fn request_enabled(provider: &Provider, endpoint: &str) -> bool {
    enabled(provider, endpoint)
        || (provider.settings_config[ENABLED] == true
            && matches!(
                endpoint.split('?').next(),
                Some("/responses/compact" | "/v1/responses/compact")
            ))
}

pub(crate) fn scope(provider: &Provider, session: &str) -> String {
    crate::manual::catalog::hash(
        &serde_json::json!([
            provider.id,
            provider.settings_config["manual_credential_version"],
            session
        ])
        .to_string(),
    )
}

/// Separate local cache: opaque upstream handles must not enter synced settings or logs.
pub(crate) struct IdentityStore {
    db: Arc<Database>,
    connection: Mutex<Option<Connection>>,
}

impl IdentityStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            connection: Mutex::new(None),
        }
    }

    fn with_connection<T>(
        &self,
        action: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> io::Result<T> {
        let mut cached = self
            .connection
            .lock()
            .map_err(|_| io::Error::other("identity cache lock failed"))?;
        if cached.is_none() {
            let source = self
                .db
                .conn
                .lock()
                .map_err(|_| io::Error::other("database lock failed"))?;
            let file: String = source
                .query_row(
                    "SELECT file FROM pragma_database_list WHERE name='main'",
                    [],
                    |r| r.get(0),
                )
                .map_err(io::Error::other)?;
            drop(source);
            let conn = if file.is_empty() {
                Connection::open_in_memory()
            } else {
                let path = std::path::Path::new(&file)
                    .with_file_name("native-response-identities.sqlite3");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .mode(0o600)
                        .open(&path)?;
                }
                Connection::open(path)
            }
            .map_err(io::Error::other)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .map_err(io::Error::other)?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS aliases (
                scope TEXT NOT NULL, kind TEXT NOT NULL, client_id TEXT NOT NULL,
                upstream_id TEXT NOT NULL, PRIMARY KEY(scope, kind, client_id)
            );",
            )
            .map_err(io::Error::other)?;
            *cached = Some(conn);
        }
        action(cached.as_ref().unwrap()).map_err(io::Error::other)
    }

    fn remember(&self, scope: &str, kind: &str, client: &str, upstream: &str) -> io::Result<()> {
        self.with_connection(|conn| {
            if client == upstream {
                conn.execute("DELETE FROM aliases WHERE scope=?1 AND kind=?2 AND client_id=?3", params![scope, kind, client])?;
            } else {
                conn.execute("INSERT INTO aliases VALUES (?1,?2,?3,?4)
                    ON CONFLICT(scope,kind,client_id) DO UPDATE SET upstream_id=excluded.upstream_id",
                    params![scope, kind, client, upstream])?;
            }
            Ok(())
        })
    }

    /// Rewrite only protocol handles, never IDs embedded inside user/tool content.
    pub(crate) fn restore_request(&self, scope: &str, body: &mut Value) -> io::Result<()> {
        self.with_connection(|conn| {
            let restore = |kind: &str, value: &mut Value| -> rusqlite::Result<()> {
                if let Some(id) = value.as_str() {
                    let upstream: Option<String> = conn.query_row(
                        "SELECT upstream_id FROM aliases WHERE scope=?1 AND kind=?2 AND client_id=?3",
                        params![scope, kind, id], |r| r.get(0)).optional()?;
                    if let Some(upstream) = upstream { *value = Value::String(upstream); }
                }
                Ok(())
            };
            if let Some(id) = body.get_mut("previous_response_id") { restore("response", id)?; }
            if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
                for item in items {
                    if let Some(id) = item.get_mut("id") { restore("item", id)?; }
                }
            }
            Ok(())
        })
    }
}

struct StreamIdentity {
    response: Option<String>,
    items: HashMap<u64, String>,
    aliases: HashMap<String, u64>,
    last_alias: HashMap<u64, String>,
    store: Arc<IdentityStore>,
    scope: String,
}

impl StreamIdentity {
    fn new(store: Arc<IdentityStore>, scope: String) -> Self {
        Self {
            response: None,
            items: HashMap::new(),
            aliases: HashMap::new(),
            last_alias: HashMap::new(),
            store,
            scope,
        }
    }

    fn item(&mut self, index: u64, id: &mut Value, authoritative: bool) -> io::Result<()> {
        let upstream = id.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let canonical = self
            .items
            .entry(index)
            .or_insert_with(|| {
                upstream
                    .clone()
                    .filter(|id| !self.aliases.contains_key(id))
                    .unwrap_or_else(|| format!("lgitem_{}", uuid::Uuid::new_v4()))
            })
            .clone();
        if let Some(upstream) = upstream {
            // Keep only the first and latest handle per item. Copilot can return
            // a different opaque handle for every token; retaining all of them
            // would make memory grow with answer length rather than item count.
            if let Some(previous) = self.last_alias.insert(index, upstream.clone()) {
                if previous != canonical && self.aliases.get(&previous) == Some(&index) {
                    self.aliases.remove(&previous);
                }
            }
            self.aliases.entry(canonical.clone()).or_insert(index);
            self.aliases.entry(upstream.clone()).or_insert(index);
            if authoritative {
                self.store
                    .remember(&self.scope, "item", &canonical, &upstream)?;
            }
        }
        *id = Value::String(canonical);
        Ok(())
    }

    fn rewrite(&mut self, event: &mut Value) -> io::Result<()> {
        let typ = event["type"].as_str().unwrap_or("").to_owned();
        let terminal = matches!(
            typ.as_str(),
            "response.completed" | "response.failed" | "response.incomplete"
        );
        if let Some(response) = event.get_mut("response").filter(|v| v.is_object()) {
            let upstream = response["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let canonical = self
                .response
                .get_or_insert_with(|| {
                    upstream
                        .clone()
                        .unwrap_or_else(|| format!("lgresp_{}", uuid::Uuid::new_v4()))
                })
                .clone();
            if terminal {
                if let Some(upstream) = upstream {
                    self.store
                        .remember(&self.scope, "response", &canonical, &upstream)?;
                }
            }
            response["id"] = Value::String(canonical);
            if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
                for (index, item) in items.iter_mut().enumerate() {
                    self.item(index as u64, &mut item["id"], terminal)?;
                }
            }
        }
        if let Some(id) = event.get_mut("response_id") {
            if let Some(canonical) = &self.response {
                *id = Value::String(canonical.clone());
            }
        }
        let index = event["output_index"].as_u64().or_else(|| {
            event["item_id"]
                .as_str()
                .and_then(|id| self.aliases.get(id).copied())
        });
        if let Some(index) = index {
            if let Some(item) = event.get_mut("item").filter(|v| v.is_object()) {
                self.item(index, &mut item["id"], typ == "response.output_item.done")?;
            }
            if let Some(id) = event.get_mut("item_id") {
                self.item(index, id, false)?;
            }
        } else if event.get("item_id").is_some() && typ.starts_with("response.") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot associate Responses item without output_index",
            ));
        }
        Ok(())
    }

    fn frame(&mut self, raw: Vec<u8>) -> io::Result<Bytes> {
        let text = std::str::from_utf8(&raw)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE UTF-8"))?;
        let data = text
            .lines()
            .filter_map(|line| crate::proxy::sse::strip_sse_field(line, "data"))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(mut value) = serde_json::from_str::<Value>(&data) else {
            return Ok(Bytes::from(raw));
        };
        if !value["type"]
            .as_str()
            .is_some_and(|t| t.starts_with("response."))
        {
            return Ok(Bytes::from(raw));
        }
        let before = value.clone();
        self.rewrite(&mut value)?;
        if before == value {
            return Ok(Bytes::from(raw));
        }
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut rewritten = String::new();
        let mut written = false;
        for line in text.lines().filter(|line| !line.is_empty()) {
            if crate::proxy::sse::strip_sse_field(line, "data").is_some() {
                if !written {
                    rewritten.push_str(&format!("data: {value}{newline}"));
                    written = true;
                }
            } else {
                rewritten.push_str(line);
                rewritten.push_str(newline);
            }
        }
        rewritten.push_str(newline);
        Ok(Bytes::from(rewritten))
    }
}

pub(crate) fn stream(
    input: impl Stream<Item = Result<Bytes, io::Error>> + Send + 'static,
    store: Arc<IdentityStore>,
    scope: String,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Send {
    async_stream::try_stream! {
        let mut identity = StreamIdentity::new(store, scope);
        let mut buffer = Vec::new();
        futures::pin_mut!(input);
        while let Some(chunk) = input.next().await {
            buffer.extend_from_slice(&chunk?);
            loop {
                let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| i + 2);
                let crlf = buffer.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4);
                let Some(end) = lf.into_iter().chain(crlf).min() else { break; };
                let rest = buffer.split_off(end);
                let frame = std::mem::replace(&mut buffer, rest);
                yield identity.frame(frame)?;
            }
            if buffer.len() > 16 * 1024 * 1024 {
                Err(io::Error::new(io::ErrorKind::InvalidData, "Responses SSE frame exceeds 16 MiB"))?;
            }
        }
        if !buffer.is_empty() { yield identity.frame(buffer)?; }
    }
}

pub(crate) fn wrap(
    response: ProxyResponse,
    store: Arc<IdentityStore>,
    scope: String,
) -> ProxyResponse {
    if !response.status().is_success() || !response.is_sse() {
        return response;
    }
    let status = response.status();
    let mut headers = response.headers().clone();
    headers.remove(http::header::CONTENT_LENGTH);
    ProxyResponse::streamed(
        status,
        headers,
        stream(response.bytes_stream(), store, scope),
    )
}

#[cfg(test)]
#[path = "native_responses_identity_tests.rs"]
mod tests;
