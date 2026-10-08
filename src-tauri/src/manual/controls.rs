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
        .find(|model| model.enabled && model.id == group.model.id)
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
