//! Explicit limits requested by the owner of this Manual build, not vendor capability claims.
//! Apply to the effective group after source aggregation; never rewrite the original declarations.
use super::catalog::Model;
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

pub const CONTEXT_WINDOW: u64 = 1_000_000;
pub const MAX_OUTPUT_TOKENS: u64 = 128_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSource {
    UserOverride,
}

pub fn matches(id: &str) -> bool {
    // A provider namespace and the existing FW aliases are supported; arbitrary embedded
    // substrings (e.g. not-kimi or gemini-gpt-6) are deliberately not classified as a family.
    let normalized = id
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let model = normalized
        .strip_prefix("fw-")
        .or_else(|| normalized.strip_prefix("fw_"))
        .unwrap_or(&normalized);
    static GPT: OnceLock<Regex> = OnceLock::new();
    let gpt =
        GPT.get_or_init(|| Regex::new(r"^gpt[-_ ]?([0-9]+)(?:\.([0-9]+))?(?:$|[-_: ])").unwrap());
    if let Some(parts) = gpt.captures(model) {
        let major = parts[1].parse::<u64>().unwrap_or(0);
        let minor = parts
            .get(2)
            .and_then(|value| value.as_str().parse::<u64>().ok());
        return major == 6 || (major == 5 && minor == Some(6));
    }
    static OTHER: OnceLock<Regex> = OnceLock::new();
    OTHER
        .get_or_init(|| Regex::new(r"^(?:kimi|glm)(?:$|[-_. ]|[0-9])|^kimik[0-9]").unwrap())
        .is_match(model)
}

pub fn apply(model: &mut Model) {
    model.limits_source = None;
    if !model.id.starts_with("copilot/") && matches(&model.id) {
        model.context_window = Some(CONTEXT_WINDOW);
        model.max_output_tokens = Some(MAX_OUTPUT_TOKENS);
        model.limits_source = Some(LimitSource::UserOverride);
    }
}

#[cfg(test)]
mod tests;
