use super::{
    catalog::{self, Document, Model, Source, TestResult},
    test_model_once,
};
use std::time::Instant;

pub fn test_target<'a>(
    doc: &'a Document,
    group_id: &str,
    provider_id: &str,
) -> Result<(&'a Source, &'a Model), String> {
    let group = doc
        .groups()
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or("模型已变化，请刷新后重试")?;
    if group.blocked {
        return Err("模型已屏蔽，请先恢复".into());
    }
    if !group.provider_ids.iter().any(|id| id == provider_id) {
        return Err("该 Provider 不属于此模型的启用来源".into());
    }
    let source = doc
        .providers
        .iter()
        .find(|source| source.id == provider_id && source.enabled)
        .ok_or("Provider 已停用或删除")?;
    let model = source
        .models
        .iter()
        .find(|model| {
            model.enabled
                && (model.id == group.model.id || group.model_ids.contains(&model.id))
                && super::endpoints::protocol(model) == group.protocol
        })
        .ok_or("该来源的模型已停用或删除")?;
    Ok((source, model))
}

pub async fn test_deployment(
    doc: &Document,
    group_id: &str,
    provider_id: &str,
) -> Result<TestResult, String> {
    let (source, model) = test_target(doc, group_id, provider_id)?;
    let start = Instant::now();
    // 来源测试刻意不经过路由器，否则备用成功会掩盖当前来源的故障。
    let response = test_model_once(source, model).await;
    Ok(TestResult {
        success: response.is_ok(),
        latency_ms: start.elapsed().as_millis() as u64,
        checked_at: chrono::Utc::now().to_rfc3339(),
        detail: response.err().unwrap_or_else(|| "测试通过".into()),
        provider_id: Some(source.id.clone()),
        provider_name: Some(source.name.clone()),
        model_id: Some(model.id.clone()),
    })
}

pub fn clear_source_tests(doc: &mut Document, provider_id: &str) {
    doc.tests.retain(|key, result| {
        result.provider_id.as_deref() != Some(provider_id)
            && serde_json::from_str::<(String, String)>(key)
                .map(|(_, id)| id != provider_id)
                .unwrap_or(true)
    });
}

pub fn record_test(doc: &mut Document, group_id: &str, provider_id: &str, result: TestResult) {
    // 旧记录未标注来源，不把它当成所有部署的结果；其他来源的独立记录保留。
    doc.tests.remove(group_id);
    doc.tests
        .insert(catalog::test_key(group_id, provider_id), result);
}

pub fn set_blocked(doc: &mut Document, model_id: &str, blocked: bool) -> Result<(), String> {
    if !doc
        .providers
        .iter()
        .any(|s| s.models.iter().any(|m| m.id == model_id))
        && (blocked || !doc.blocked_models.contains(model_id))
    {
        return Err("模型不存在，请刷新后重试".into());
    }
    let mut candidate = doc.clone();
    if blocked {
        candidate.blocked_models.insert(model_id.into());
    } else {
        candidate.blocked_models.remove(model_id);
    }
    candidate.validate()?;
    *doc = candidate;
    Ok(())
}

/// 改本网关的部署候选。只代表是否参与路由候选，
/// 不代表上游已经部署或已验证可用，也不改来源自身的启停。
pub fn set_model_enabled(
    doc: &mut Document,
    provider_id: &str,
    model_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let source = doc
        .providers
        .iter_mut()
        .find(|s| s.id == provider_id)
        .ok_or("来源不存在，请刷新后重试")?;
    let model = source
        .models
        .iter_mut()
        .find(|m| m.id == model_id)
        .ok_or("该来源没有这个模型，请刷新模型目录")?;
    model.enabled = enabled;
    doc.validate()
}

/// 批量启用或停用指定来源的模型。
pub fn set_models_enabled_batch(
    doc: &mut Document,
    provider_id: &str,
    model_ids: Option<&[String]>,
    enabled: bool,
) -> Result<usize, String> {
    let source = doc
        .providers
        .iter_mut()
        .find(|s| s.id == provider_id)
        .ok_or("来源不存在，请刷新后重试")?;
    let mut changed = 0usize;
    for model in &mut source.models {
        let targeted = match model_ids {
            Some(ids) => ids.iter().any(|id| id == &model.id),
            None => true,
        };
        if targeted && model.enabled != enabled {
            model.enabled = enabled;
            changed += 1;
        }
    }
    doc.validate()?;
    Ok(changed)
}

/// 按给定模型名列表（如本地 Codex 当前模型目录）匹配并启用对应模型，未匹配的自动停用。
/// 返回成功匹配并启用的去重模型数量。
pub fn match_enabled_models(doc: &mut Document, target_names: &[String]) -> Result<usize, String> {
    use std::collections::HashSet;
    let normalized: HashSet<String> = target_names
        .iter()
        .map(|raw| {
            let s = raw.trim();
            super::catalog::canonical_model_id(s)
                .rsplit('/')
                .next()
                .unwrap_or(s)
                .to_ascii_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect();
    if normalized.is_empty() {
        return Err("未提供可匹配的模型名称列表".into());
    }
    let exact: HashSet<String> = target_names
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let mut matched = HashSet::new();
    for source in &mut doc.providers {
        for model in &mut source.models {
            let canonical = super::catalog::canonical_model_id(&model.id).to_ascii_lowercase();
            let bare = canonical.rsplit('/').next().unwrap_or(&canonical);
            let is_match = exact.contains(&model.id.to_ascii_lowercase())
                || exact.contains(&canonical)
                || normalized.contains(&canonical)
                || normalized.contains(bare);
            model.enabled = is_match;
            if is_match {
                doc.blocked_models.remove(&model.id);
                matched.insert(canonical);
            }
        }
    }
    doc.validate()?;
    Ok(matched.len())
}

pub fn set_cost_estimation(
    doc: &mut Document,
    provider_id: &str,
    enabled: bool,
    revision: u64,
) -> Result<(), String> {
    if doc.revision != revision {
        return Err("配置已变化，请刷新后重试".into());
    }
    doc.providers
        .iter_mut()
        .find(|s| s.id == provider_id)
        .ok_or("Provider 不存在")?
        .cost_estimation_enabled = enabled;
    Ok(())
}
