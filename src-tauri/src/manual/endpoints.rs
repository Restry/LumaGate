use super::catalog::{group_id, Document, Model, Protocol};
use regex::Regex;
use std::sync::OnceLock;

/// 这是用户指定的路由默认规则，不是根据名称断言上游模型能力。
pub fn automatic_protocol(id: &str) -> Protocol {
    if id.to_ascii_lowercase().contains("gemini") {
        return Protocol::OpenaiChat;
    }
    static GPT: OnceLock<Regex> = OnceLock::new();
    let pattern = GPT.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[/ :_-])gpt[-_ ]?([0-9]+)(?:\.([0-9]+))?(?:$|[-_:/ ])").unwrap()
    });
    if let Some(parts) = pattern.captures(id) {
        let major = parts[1].parse::<u64>().unwrap_or(0);
        let minor = parts
            .get(2)
            .and_then(|v| v.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        if major > 5 || (major == 5 && minor >= 5) {
            return Protocol::OpenaiResponses;
        }
    }
    Protocol::OpenaiChat
}

pub fn protocol(model: &Model) -> Protocol {
    model
        .protocol_override
        .clone()
        .unwrap_or_else(|| automatic_protocol(&model.id))
}

pub fn set_override(
    doc: &mut Document,
    id: &str,
    selected: Option<Protocol>,
) -> Result<(), String> {
    let group = doc
        .groups()
        .into_iter()
        .find(|g| g.id == id)
        .ok_or("模型组已变化，请刷新后重试")?;
    let mut candidate = doc.clone();
    for source in &mut candidate.providers {
        if group.provider_ids.contains(&source.id) {
            if let Some(model) = source.models.iter_mut().find(|m| m.id == group.model.id) {
                model.protocol_override = selected.clone();
            }
        }
    }
    let mut model = group.model.clone();
    model.protocol_override = selected;
    let new_id = group_id(&protocol(&model), &model);
    if new_id != id {
        if candidate
            .policies
            .get(&new_id)
            .is_some_and(|existing| existing != &group.policy)
        {
            return Err("目标模型组已有不同的路由策略，请先对齐策略再更改 Endpoint".into());
        }
        candidate.policies.remove(id);
        candidate.policies.insert(new_id.clone(), group.policy);
        for policy in candidate.policies.values_mut() {
            for target in &mut policy.fallbacks {
                if target == id {
                    *target = new_id.clone();
                }
            }
        }
    }
    candidate.tests.clear();
    candidate
        .validate()
        .map_err(|e| format!("Endpoint 未修改：{e}。请先处理相关 Fallback 策略"))?;
    *doc = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_use_numeric_versions_not_prefix_or_float_comparison() {
        for id in [
            "gpt-5.5",
            "gpt5.5",
            "openai/gpt-5.5-codex",
            "GPT-5.10",
            "gpt-6",
            "gpt-6.1-mini",
        ] {
            assert_eq!(automatic_protocol(id), Protocol::OpenaiResponses, "{id}");
        }
        for id in [
            "gpt-5.4",
            "gpt-5",
            "gpt-4.1",
            "gemini-3-pro",
            "google/gemini-gpt-5.5",
            "claude-opus",
            "mygpt-5.5",
            "qwen3",
        ] {
            assert_eq!(automatic_protocol(id), Protocol::OpenaiChat, "{id}");
        }
    }
    #[test]
    fn explicit_override_wins_and_can_be_cleared() {
        let mut model = Model {
            id: "gpt-5.5".into(),
            ..Model::default()
        };
        model.protocol_override = Some(Protocol::OpenaiChat);
        assert_eq!(protocol(&model), Protocol::OpenaiChat);
        model.protocol_override = Some(Protocol::Anthropic);
        assert_eq!(protocol(&model), Protocol::Anthropic);
        model.protocol_override = None;
        assert_eq!(protocol(&model), Protocol::OpenaiResponses);
    }
}
