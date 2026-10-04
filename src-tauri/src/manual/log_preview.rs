use serde_json::{json, Map, Value};
use std::sync::LazyLock;

pub const PREVIEW_LIMIT: usize = 8192;
const FRAME_LIMIT: usize = 65536;
static SENSITIVE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)(?:bearer\s+[^\s\"']+|sk-[a-z0-9_-]{8,}|(?:api[_-]?key|access_token|refresh_token|password|secret|encrypted_content|\btoken\b)\s*[:=]\s*[\"']?[^\s\"',}]+|data:[^\s\"']+|[a-z0-9+/=_-]{96,})"#).unwrap()
});

fn text(value: &str, secrets: &[String]) -> String {
    let mut value = value.chars().take(FRAME_LIMIT).collect::<String>();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        value = value.replace(secret, "[凭据已隐藏]");
    }
    SENSITIVE
        .replace_all(&value, "[敏感数据已隐藏]")
        .chars()
        .take(PREVIEW_LIMIT)
        .collect()
}
fn scrub(value: &Value, secrets: &[String], depth: usize) -> Value {
    if depth > 8 {
        return json!("[嵌套内容已省略]");
    }
    match value {
        Value::String(value) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(if value.len() <= FRAME_LIMIT {
                value
            } else {
                ""
            }) {
                if parsed.is_object() || parsed.is_array() {
                    return scrub(&parsed, secrets, depth + 1);
                }
            }
            json!(text(value, secrets))
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(16)
                .map(|value| scrub(value, secrets, depth + 1))
                .collect(),
        ),
        Value::Object(values) => {
            if values
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "image"
                            | "image_url"
                            | "input_image"
                            | "output_image"
                            | "image_generation_call"
                            | "input_audio"
                            | "audio"
                            | "video"
                            | "input_file"
                    )
                })
            {
                return json!({"type":values.get("type"),"content":"[媒体内容已省略]"});
            }
            Value::Object(
                values
                    .iter()
                    .take(48)
                    .map(|(key, value)| {
                        let normalized = key.to_ascii_lowercase();
                        let hidden = [
                            "api_key",
                            "apikey",
                            "authorization",
                            "access_token",
                            "refresh_token",
                            "password",
                            "secret",
                            "encrypted_content",
                            "credentials",
                            "base64",
                            "image_url",
                        ]
                        .iter()
                        .any(|name| normalized.contains(name))
                            || normalized == "token";
                        (
                            text(key, secrets),
                            if hidden {
                                json!("[已隐藏]")
                            } else {
                                scrub(value, secrets, depth + 1)
                            },
                        )
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}
pub fn sanitize(value: &Value, secrets: &[String]) -> Value {
    let value = scrub(value, secrets, 0);
    let rendered = serde_json::to_string_pretty(&value).unwrap_or_default();
    if rendered.chars().count() > PREVIEW_LIMIT {
        json!(format!(
            "{}\n[内容已截断]",
            rendered.chars().take(PREVIEW_LIMIT).collect::<String>()
        ))
    } else {
        value
    }
}
pub fn last_turn(body: &Value) -> Value {
    let value = body.get("messages").or_else(|| body.get("input"));
    match value {
        Some(Value::Array(items)) => {
            let start = items
                .iter()
                .rposition(|item| item.get("role").and_then(Value::as_str) == Some("user"))
                .unwrap_or_else(|| items.len().saturating_sub(1));
            let mut selected = items[start..].iter().take(1).collect::<Vec<_>>();
            if items.len() > start + 1 {
                selected.extend(items[(start + 1).max(items.len().saturating_sub(11))..].iter());
            }
            sanitize(&json!(selected), &[])
        }
        Some(value) => sanitize(value, &[]),
        None => Value::Null,
    }
}
fn response_fields(value: &Value) -> Value {
    let mut result = Map::new();
    for key in [
        "id", "model", "status", "output", "choices", "usage", "error", "message", "detail",
    ] {
        if let Some(value) = value.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    Value::Object(result)
}

pub struct Collector {
    streaming: bool,
    pending: Vec<u8>,
    skipping: bool,
    text: String,
    result: Value,
    tools: Vec<Value>,
    truncated: bool,
}
impl Collector {
    pub fn new(streaming: bool) -> Self {
        Self {
            streaming,
            pending: vec![],
            skipping: false,
            text: String::new(),
            result: Value::Null,
            tools: vec![],
            truncated: false,
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        if !self.streaming {
            let room = FRAME_LIMIT.saturating_sub(self.pending.len());
            self.pending
                .extend_from_slice(&bytes[..bytes.len().min(room)]);
            self.truncated |= bytes.len() > room;
            return;
        }
        for byte in bytes {
            if *byte == b'\n' {
                if !self.skipping {
                    self.line();
                }
                self.pending.clear();
                self.skipping = false;
            } else if !self.skipping {
                if self.pending.len() < FRAME_LIMIT {
                    self.pending.push(*byte);
                } else {
                    self.pending.clear();
                    self.skipping = true;
                    self.truncated = true;
                }
            }
        }
    }
    fn line(&mut self) {
        let line = String::from_utf8_lossy(&self.pending);
        let Some(data) = line.trim().strip_prefix("data:") else {
            return;
        };
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            return;
        };
        let event = value.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(
            event,
            "response.completed" | "response.failed" | "response.incomplete" | "message_stop"
        ) {
            if let Some(response) = value.get("response") {
                self.result = response_fields(response);
            }
        }
        if matches!(event, "response.failed" | "error" | "response.incomplete") {
            if !self.result.is_object() {
                self.result = json!({});
            }
            self.result["status"] = json!(if event == "response.incomplete" {
                "incomplete"
            } else {
                "failed"
            });
        }
        if event == "response.output_item.done" && self.tools.len() < 8 {
            if let Some(item) = value.get("item").filter(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "function_call" | "custom_tool_call"))
            }) {
                self.tools.push(sanitize(item, &[]));
            }
        }
        if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
            if !self.result.is_object() {
                self.result = json!({});
            }
            self.result["error"] = error.clone();
        }
        let delta = if event == "response.output_text.delta" {
            value.get("delta").and_then(Value::as_str)
        } else {
            value
                .pointer("/choices/0/delta/content")
                .or_else(|| value.pointer("/delta/text"))
                .and_then(Value::as_str)
        };
        if let Some(delta) = delta {
            let room = PREVIEW_LIMIT.saturating_sub(self.text.chars().count());
            self.text.extend(delta.chars().take(room));
            self.truncated |= delta.chars().count() > room;
        }
        if let Some(usage) = value.get("usage").filter(|value| !value.is_null()) {
            if !self.result.is_object() {
                self.result = json!({});
            }
            self.result["usage"] = usage.clone();
        }
    }
    pub fn interrupted(&mut self) {
        if !self.streaming {
            self.truncated = true;
        }
    }
    pub fn finish(&mut self, secrets: &[String]) -> Value {
        let value = if self.streaming {
            if !self.pending.is_empty() && !self.skipping {
                self.line();
            }
            if !self.result.is_object() {
                self.result = json!({});
            }
            if !self.text.is_empty() && self.result.get("output").is_none() {
                self.result["text"] = json!(self.text);
            }
            if !self.tools.is_empty() && self.result.get("output").is_none() {
                self.result["tool_calls"] = json!(self.tools);
            }
            if self.truncated {
                self.result["truncated"] = json!(true);
            }
            self.result.clone()
        } else {
            match serde_json::from_slice::<Value>(&self.pending) {
                Ok(value) => response_fields(&value),
                Err(_) if self.truncated => json!("[响应不完整或超过预览限制，未保留原始片段]"),
                Err(_) => json!(String::from_utf8_lossy(&self.pending)),
            }
        };
        sanitize(&value, secrets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_only_keeps_last_turn_and_hides_media_and_credentials() {
        let body = json!({"messages":[{"role":"system","content":"old-system"},{"role":"user","content":"old-user"},{"role":"assistant","content":"old-answer"},{"role":"user","content":[{"type":"text","text":"latest"},{"type":"image_url","image_url":{"url":"data:image/png;base64,private-image"}}]},{"role":"tool","content":"{\"api_key\":\"private-key\"}"}]});
        let result = last_turn(&body).to_string();
        assert!(result.contains("latest"));
        for private in [
            "old-system",
            "old-user",
            "old-answer",
            "private-image",
            "private-key",
        ] {
            assert!(!result.contains(private));
        }
        let value = sanitize(
            &json!({"text":"provider-credential-fixture"}),
            &["provider-credential-fixture".into()],
        );
        assert!(!value.to_string().contains("provider-credential-fixture"));
    }
    #[test]
    fn split_sse_is_formatted_and_redacted_after_delta_assembly() {
        let events = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"type":"response.output_text.delta","delta":"你好 provider-"}),
            json!({"type":"response.output_text.delta","delta":"credential-fixture"}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","name":"run","arguments":"{\"api_key\":\"private-tool-key\"}"}})
        );
        let mut collector = Collector::new(true);
        for chunk in events.as_bytes().chunks(3) {
            collector.feed(chunk);
        }
        let result = collector.finish(&["provider-credential-fixture".into()]);
        assert!(result["text"].as_str().unwrap().contains("你好"));
        assert_eq!(result["tool_calls"][0]["name"], "run");
        assert!(!result.to_string().contains("provider-credential-fixture"));
        assert!(!result.to_string().contains("private-tool-key"));
    }
    #[test]
    fn response_preview_is_bounded_and_keeps_errors() {
        let mut collector = Collector::new(false);
        collector.feed(br#"{"error":{"message":"failed","authorization":"private"},"instructions":"omit-system"}"#);
        let result = collector.finish(&[]).to_string();
        assert!(result.contains("failed"));
        assert!(!result.contains("private"));
        assert!(!result.contains("omit-system"));
        let long = sanitize(&json!({"text":"abc ".repeat(10000)}), &[]);
        assert!(long.to_string().len() < PREVIEW_LIMIT * 2);
    }
}
