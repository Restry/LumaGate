use super::catalog::Model;

fn shared_limit(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    a.zip(b).map(|(a, b)| a.min(b))
}

fn shared_flag(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn shared_values(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> Option<Vec<String>> {
    let (a, b) = a.as_ref().zip(b.as_ref())?;
    let mut values: Vec<_> = a
        .iter()
        .filter(|value| b.contains(value))
        .cloned()
        .collect();
    values.sort();
    values.dedup();
    Some(values)
}

/// 同名部署不能因为目录字段缺失而拆散；也不能把某一路的声明承诺给所有线路。
pub fn merge(model: &mut Model, next: &Model) {
    model.context_window = shared_limit(model.context_window, next.context_window);
    model.max_output_tokens = shared_limit(model.max_output_tokens, next.max_output_tokens);
    model.tools = shared_flag(model.tools, next.tools);
    model.reasoning = shared_flag(model.reasoning, next.reasoning);
    model.reasoning_summaries = shared_flag(model.reasoning_summaries, next.reasoning_summaries);
    model.thinking_types = shared_values(&model.thinking_types, &next.thinking_types);
    model.reasoning_levels = shared_values(&model.reasoning_levels, &next.reasoning_levels);
    // 模态展示可服务的合集，转发时再按具体请求过滤来源，不能被未知字段抹掉。
    model.input_modalities = super::modalities::available_union(
        super::modalities::declared_input(model),
        super::modalities::declared_input(next),
    );
    model.output_modalities = super::modalities::available_union(
        model.output_modalities.clone(),
        next.output_modalities.clone(),
    );
    model.supported_parameters =
        shared_values(&model.supported_parameters, &next.supported_parameters);
    model.image = model
        .input_modalities
        .as_ref()
        .is_some_and(|values| values.iter().any(|v| v == "image"));
    if model.default_reasoning_level != next.default_reasoning_level
        || model.default_reasoning_level.as_ref().is_some_and(|level| {
            model
                .reasoning_levels
                .as_ref()
                .is_some_and(|levels| !levels.contains(level))
        })
    {
        model.default_reasoning_level = None;
    }
    if model.reasoning == Some(false) {
        model.default_reasoning_level = None;
        model.reasoning_levels = None;
        model.thinking_types = None;
    }
    if model.name != next.name {
        model.name = Some(model.id.clone());
    }
    // 原始数据仍保留在各 Source；聚合行不冒充任何一个 Provider 的完整声明。
    model.metadata = serde_json::Value::Null;
}
