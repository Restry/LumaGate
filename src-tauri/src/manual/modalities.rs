use super::catalog::Model;
use serde_json::Value;
use std::collections::BTreeSet;

pub fn declared_input(model: &Model) -> Option<Vec<String>> {
    model.input_modalities.clone().or_else(|| {
        // 兼容只声明 vision/image 的旧目录，沿用 Pi 投影对该字段的解释。
        model.image.then(|| vec!["text".into(), "image".into()])
    })
}

pub fn available_union(a: Option<Vec<String>>, b: Option<Vec<String>>) -> Option<Vec<String>> {
    if a.is_none() && b.is_none() {
        return None;
    }
    Some(
        a.into_iter()
            .flatten()
            .chain(b.into_iter().flatten())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    )
}

fn content_requirements(value: &Value, required: &mut BTreeSet<String>) {
    match value {
        Value::String(text) if !text.is_empty() => {
            required.insert("text".into());
        }
        Value::Array(items) => {
            for item in items {
                content_requirements(item, required);
            }
        }
        Value::Object(item) => match item.get("type").and_then(Value::as_str) {
            Some("image" | "image_url" | "input_image") => {
                required.insert("image".into());
            }
            Some("input_audio" | "audio") => {
                required.insert("audio".into());
            }
            Some("video" | "video_url" | "input_video") => {
                required.insert("video".into());
            }
            Some("text" | "input_text" | "output_text") => {
                required.insert("text".into());
            }
            Some("message" | "tool_result" | "function_call_output") | None => {
                if let Some(content) = item.get("content") {
                    content_requirements(content, required);
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
                    if let Some(output) = item.get("output") {
                        content_requirements(output, required);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn supports(declared: Option<&[String]>, required: &BTreeSet<String>) -> bool {
    required.iter().all(|kind| match declared {
        Some(values) => values.contains(kind),
        // 未声明模态的兼容文本接口仍保留；非文本输入不能碰运气跨来源发送。
        None => kind == "text",
    })
}

pub fn supports_request(model: &Model, body: &Value) -> bool {
    let mut input = BTreeSet::new();
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for message in messages {
            if let Some(content) = message.get("content") {
                content_requirements(content, &mut input);
            }
        }
    }
    if let Some(value) = body.get("input") {
        content_requirements(value, &mut input);
    }
    let output: BTreeSet<_> = body
        .get("modalities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| BTreeSet::from(["text".into()]));
    supports(declared_input(model).as_deref(), &input)
        && supports(model.output_modalities.as_deref(), &output)
}
