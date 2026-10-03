use super::catalog::Model;
use serde_json::{json, Value};

fn positive(row: &Value, paths: &[&str]) -> Option<u64> {
    paths.iter().find_map(|path| {
        row.pointer(path)
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            .filter(|v| *v > 0)
    })
}
fn boolean(row: &Value, paths: &[&str]) -> Option<bool> {
    paths
        .iter()
        .find_map(|path| row.pointer(path).and_then(Value::as_bool))
}
fn text(row: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        row.pointer(path)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })
}
fn strings(row: &Value, paths: &[&str]) -> Option<Vec<String>> {
    paths.iter().find_map(|path| {
        let values = row.pointer(path)?.as_array()?;
        let mut parsed: Vec<String> = values
            .iter()
            .filter_map(|v| {
                v.as_str().or_else(|| {
                    ["effort", "value", "id", "name"]
                        .iter()
                        .find_map(|key| v.get(key)?.as_str())
                })
            })
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect();
        // 声明了数组但元素无法识别，不把解析失败误报成“明确不支持”。
        if !values.is_empty() && parsed.is_empty() {
            return None;
        }
        parsed.sort();
        parsed.dedup();
        Some(parsed)
    })
}

/// /v1/models 是目录真值。除本机启用开关外，不把上次刷新残留的能力当作本次声明。
pub fn parse_model(row: &Value, enabled: bool) -> Result<Model, String> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or("模型条目缺少 id")?;
    if serde_json::to_vec(row).map_err(|_| "模型元数据无效")?.len() > 64 * 1024 {
        return Err("单模型元数据超过 64 KiB，目录未覆盖".into());
    }
    let input = strings(
        row,
        &[
            "/architecture/input_modalities",
            "/input_modalities",
            "/inputModalities",
            "/capabilities/input",
            "/capabilities/input_modalities",
            "/modalities/input",
            "/input",
        ],
    );
    let output = strings(
        row,
        &[
            "/architecture/output_modalities",
            "/output_modalities",
            "/outputModalities",
            "/capabilities/output",
            "/capabilities/output_modalities",
            "/modalities/output",
            "/output",
        ],
    );
    let parameters = strings(row, &["/supported_parameters", "/supportedParameters"]);
    let levels = strings(
        row,
        &[
            "/supported_reasoning_levels",
            "/supported_reasoning_efforts",
            "/reasoning_levels",
            "/reasoning_efforts",
            "/thinking_levels",
            "/reasoningLevels",
            "/reasoning/levels",
            "/reasoning/efforts",
            "/reasoning/effort/values",
            "/capabilities/reasoning/levels",
            "/capabilities/thinking/levels",
            "/capabilities/effort/levels",
        ],
    );
    let mut reasoning = boolean(
        row,
        &[
            "/reasoning",
            "/supports_reasoning",
            "/reasoning/supported",
            "/capabilities/reasoning",
            "/capabilities/reasoning/supported",
            "/capabilities/thinking/supported",
        ],
    );
    if reasoning.is_none()
        && (levels.as_ref().is_some_and(|v| !v.is_empty())
            || parameters.as_ref().is_some_and(|p| {
                p.iter()
                    .any(|v| matches!(v.as_str(), "reasoning" | "reasoning_effort"))
            }))
    {
        reasoning = Some(true);
    }
    if reasoning == Some(false) && levels.as_ref().is_some_and(|v| !v.is_empty()) {
        return Err(format!("{id} 同时声明不支持思考和思考层级，目录未覆盖"));
    }
    let tools = boolean(
        row,
        &[
            "/capabilities/tools",
            "/capabilities/tool_calling",
            "/supports_tools",
            "/supports_tool_calls",
            "/tool_call",
            "/tools",
        ],
    )
    .or_else(|| {
        parameters
            .as_ref()
            .filter(|p| p.iter().any(|v| v == "tools"))
            .map(|_| true)
    });
    let image = input
        .as_ref()
        .map(|values| values.iter().any(|v| v == "image"))
        .unwrap_or_else(|| {
            boolean(row, &["/vision", "/capabilities/vision", "/image"]).unwrap_or(false)
        });
    Ok(Model {
        id: id.into(),
        protocol_override: None,
        limits_source: None,
        name: text(row, &["/name", "/display_name", "/displayName"]),
        enabled,
        image,
        tools,
        context_window: positive(
            row,
            &[
                "/context_window",
                "/context_length",
                "/contextWindow",
                "/max_context_tokens",
                "/max_input_tokens",
                "/limits/context",
                "/limits/context_window",
                "/top_provider/context_length",
                "/context_window/max_input_tokens",
            ],
        ),
        max_output_tokens: positive(
            row,
            &[
                "/max_output_tokens",
                "/max_completion_tokens",
                "/maxTokens",
                "/max_tokens",
                "/limits/output",
                "/limits/max_output_tokens",
                "/top_provider/max_completion_tokens",
                "/context_window/max_output_tokens",
            ],
        ),
        reasoning,
        thinking_types: strings(row, &["/thinking_types", "/capabilities/thinking/types"]),
        reasoning_summaries: boolean(row, &["/supports_reasoning_summaries"]),
        reasoning_levels: levels,
        default_reasoning_level: text(
            row,
            &[
                "/default_reasoning_effort",
                "/default_reasoning_level",
                "/defaultReasoningLevel",
                "/reasoning/default",
                "/reasoning/default_effort",
                "/reasoning/defaultEffort",
                "/capabilities/effort/default_level",
            ],
        ),
        input_modalities: input,
        output_modalities: output,
        supported_parameters: parameters,
        // 只存数据，不把上游任意 compat / headers / shell 字段投影进 Agent 配置。
        metadata: row.clone(),
    })
}

pub fn capability_details(model: &Model) -> Value {
    json!({"maxOutputTokens":model.max_output_tokens,"reasoning":model.reasoning,"thinkingTypes":model.thinking_types,"reasoningSummaries":model.reasoning_summaries,"reasoningLevels":model.reasoning_levels,"defaultReasoningLevel":model.default_reasoning_level,"inputModalities":model.input_modalities,"outputModalities":model.output_modalities,"supportedParameters":model.supported_parameters})
}

fn client_input(model: &Model) -> Vec<&'static str> {
    match super::modalities::declared_input(model) {
        Some(values) => ["text", "image"]
            .into_iter()
            .filter(|kind| values.iter().any(|v| v.as_str() == *kind))
            .collect(),
        // 客户端输入通道描述本网关能力：未知来源只开放文本，不能继承模板的图片假设。
        None => vec!["text"],
    }
}

pub fn pi_model(model: &Model, id: &str) -> Value {
    let mut value = json!({"id":id,"name":model.name.as_deref().unwrap_or(&model.id),"input":client_input(model)});
    if let Some(context) = model.context_window {
        value["contextWindow"] = json!(context);
    }
    if let Some(output) = model.max_output_tokens {
        value["maxTokens"] = json!(output);
    }
    if let Some(reasoning) = model.reasoning {
        value["reasoning"] = json!(reasoning);
    }
    if let Some(levels) = &model.reasoning_levels {
        let mut map = serde_json::Map::new();
        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            let provider_value = if levels.iter().any(|v| v == level) {
                Some(level)
            } else if level == "off" && levels.iter().any(|v| v == "none") {
                Some("none")
            } else {
                None
            };
            map.insert(level.into(), json!(provider_value));
        }
        value["thinkingLevelMap"] = Value::Object(map);
    }
    // adaptive 是协议特性，必须由上游明确声明，不能通过名称或 high/max 档位推断。
    if model.reasoning == Some(true)
        && model
            .thinking_types
            .as_ref()
            .is_some_and(|v| v.iter().any(|t| t == "adaptive"))
    {
        value["compat"] = json!({"forceAdaptiveThinking":true});
    }
    value
}

pub fn enrich_codex_entry(entry: &mut Value, model: &Model) {
    if let Some(object) = entry.as_object_mut() {
        object.remove("context_window");
        object.remove("max_context_window");
    }
    if let Some(context) = model.context_window {
        entry["context_window"] = json!(context);
        entry["max_context_window"] = json!(context);
    }
    entry["supports_reasoning_summaries"] = json!(model.reasoning_summaries.unwrap_or(false));
    entry["input_modalities"] = json!(client_input(model));
    // 目录缺字段时不能沿用上游模板的推理档位，否则会凭空宣称 low/medium/high。
    if let Some(object) = entry.as_object_mut() {
        object.remove("default_reasoning_level");
        object.remove("supported_reasoning_levels");
    }
    entry["supported_reasoning_levels"] = json!([]);
    if let Some(levels) = &model.reasoning_levels {
        entry["supported_reasoning_levels"] = json!(levels
            .iter()
            .filter(
                |v| ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .contains(&v.as_str())
            )
            .map(|v| json!({"effort":v,"description":v}))
            .collect::<Vec<_>>());
        if let Some(default) = &model.default_reasoning_level {
            if levels.contains(default)
                && ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .contains(&default.as_str())
            {
                entry["default_reasoning_level"] = json!(default);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_openai_and_codex_metadata_without_losing_extensions() {
        let row = json!({"id":"model-a","name":"Model A","context_window":400000,"max_output_tokens":128000,"supported_reasoning_levels":[{"effort":"high","description":"详细"},{"effort":"low"}],"default_reasoning_level":"high","supported_parameters":["tools","reasoning_effort"],"vendor_extension":{"release":"v1"}});
        let model = parse_model(&row, true).unwrap();
        assert_eq!(model.context_window, Some(400000));
        assert_eq!(model.max_output_tokens, Some(128000));
        assert_eq!(model.reasoning, Some(true));
        assert_eq!(model.tools, Some(true));
        assert_eq!(
            model.reasoning_levels,
            Some(vec!["high".into(), "low".into()])
        );
        assert_eq!(model.metadata, row);
    }
    #[test]
    fn parses_nested_limits_and_anthropic_effort() {
        let model = parse_model(&json!({"id":"model-b","limits":{"context":200000,"output":64000},"capabilities":{"thinking":{"supported":true,"types":["adaptive"]},"effort":{"levels":["low","high","max"],"default_level":"high"}},"architecture":{"input_modalities":["image","text"]}}),true).unwrap();
        assert!(model.image);
        assert_eq!(model.context_window, Some(200000));
        assert_eq!(model.max_output_tokens, Some(64000));
        assert_eq!(model.reasoning, Some(true));
        let pi = pi_model(&model, "alias");
        assert_eq!(pi["thinkingLevelMap"]["max"], "max");
        assert!(pi["thinkingLevelMap"]["medium"].is_null());
        assert_eq!(pi["compat"]["forceAdaptiveThinking"], true);
    }
    #[test]
    fn unknown_metadata_stays_unknown_and_names_do_not_imply_thinking() {
        let model = parse_model(&json!({"id":"model-thinking-high"}), true).unwrap();
        assert_eq!(model.reasoning, None);
        assert_eq!(model.context_window, None);
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.reasoning_levels, None);
        let pi = pi_model(&model, "alias");
        assert!(pi.get("reasoning").is_none());
        assert!(pi.get("contextWindow").is_none());
        assert!(pi.get("thinkingLevelMap").is_none());
    }
    #[test]
    fn grouping_and_public_directory_use_declared_capabilities() {
        use super::super::catalog::{public_models, Document, Protocol, Source};
        let row = json!({"id":"same-model","context_length":262144,"max_output_tokens":64000,"reasoning":true,"reasoning_levels":["high","low"]});
        let mut other = row.clone();
        other["vendor_extension"] = json!("different-description");
        let make = |id: &str, value: &Value| Source {
            id: id.into(),
            name: id.into(),
            base_url: "https://example.com/v1".into(),
            key_env: String::new(),
            key_ref: None,
            copilot: None,
            protocol: Protocol::OpenaiResponses,
            enabled: true,
            models: vec![parse_model(value, true).unwrap()],
        };
        let mut doc = Document {
            providers: vec![make("a", &row), make("b", &other)],
            ..Document::default()
        };
        assert_eq!(doc.groups().len(), 1);
        let directory = public_models(&doc);
        assert_eq!(directory["data"][0]["context_window"], 262144);
        assert_eq!(directory["data"][0]["max_output_tokens"], 64000);
        assert_eq!(directory["data"][0]["reasoning"], true);
        doc.providers[1].models[0].reasoning_levels = Some(vec!["high".into()]);
        assert_eq!(doc.groups().len(), 1);
        assert_eq!(
            doc.groups()[0].model.reasoning_levels,
            Some(vec!["high".into()])
        );
    }

    #[test]
    fn client_projections_use_metadata_not_template_guesses() {
        let model=parse_model(&json!({"id":"model-a","context_window":400000,"max_output_tokens":128000,"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"high"}),true).unwrap();
        let pi = pi_model(&model, "alias");
        assert_eq!(pi["contextWindow"], 400000);
        assert_eq!(pi["maxTokens"], 128000);
        assert_eq!(pi["reasoning"], true);
        assert_eq!(pi["thinkingLevelMap"]["low"], "low");
        assert!(pi["thinkingLevelMap"]["medium"].is_null());
        let mut codex = json!({"context_window":128000,"max_context_window":128000,"default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"medium"}],"supports_reasoning_summaries":true});
        enrich_codex_entry(&mut codex, &model);
        assert_eq!(codex["context_window"], 400000);
        assert_eq!(codex["max_context_window"], 400000);
        assert_eq!(codex["default_reasoning_level"], "high");
        assert_eq!(
            codex["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(codex["supports_reasoning_summaries"], false);
        enrich_codex_entry(
            &mut codex,
            &parse_model(&json!({"id":"unknown"}), true).unwrap(),
        );
        assert!(codex.get("context_window").is_none());
        assert!(codex.get("default_reasoning_level").is_none());
        assert_eq!(codex["supported_reasoning_levels"], json!([]));
    }

    #[test]
    fn raw_metadata_cannot_inject_agent_configuration() {
        let model=parse_model(&json!({"id":"safe","headers":{"Authorization":"!bad-command"},"apiKey":"!bad-command","samplingParams":{"model":"wrong"}}),true).unwrap();
        let pi = pi_model(&model, "alias");
        assert!(pi.get("headers").is_none());
        assert!(pi.get("apiKey").is_none());
        assert!(pi.get("samplingParams").is_none());
    }
}
