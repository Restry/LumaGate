//! Protocol completion is independent of transport EOF and token accounting.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    #[default]
    Unknown,
    Completed,
    Incomplete,
    Failed,
}
impl Completion {
    pub fn merge(self, next: Self) -> Self {
        use Completion::*;
        match (self, next) {
            (Failed, _) | (_, Failed) => Failed,
            (Incomplete, _) | (_, Incomplete) => Incomplete,
            (Completed, _) | (_, Completed) => Completed,
            _ => Unknown,
        }
    }
}
#[derive(Deserialize)]
struct Choice {
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    status: Option<String>,
    error: Option<serde::de::IgnoredAny>,
    response: Option<Box<Envelope>>,
    delta: Option<Delta>,
    stop_reason: Option<String>,
    choices: Option<Vec<Choice>>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Delta {
    Stop { stop_reason: Option<String> },
    Other(serde::de::IgnoredAny),
}
fn stopped(reason: &str) -> Completion {
    match reason {
        "length" | "content_filter" | "max_tokens" | "model_context_window_exceeded" => {
            Completion::Incomplete
        }
        "stop" | "tool_calls" | "function_call" | "end_turn" | "stop_sequence" | "tool_use"
        | "pause_turn" | "refusal" => Completion::Completed,
        _ => Completion::Unknown,
    }
}
/// Only protocol-envelope fields are inspected; quoted text/tools are ignored.
pub fn inspect(bytes: &[u8]) -> Result<Completion, serde_json::Error> {
    fn classify(value: &Envelope) -> Completion {
        if value.error.is_some()
            || matches!(value.kind.as_deref(), Some("response.failed" | "error"))
            || matches!(value.status.as_deref(), Some("failed" | "cancelled"))
        {
            return Completion::Failed;
        }
        let mut result = match (value.kind.as_deref(), value.status.as_deref()) {
            (Some("response.incomplete"), _) | (_, Some("incomplete")) => Completion::Incomplete,
            (Some("response.completed" | "message_stop"), _) | (_, Some("completed")) => {
                Completion::Completed
            }
            _ => Completion::Unknown,
        };
        if let Some(response) = &value.response {
            result = result.merge(classify(response));
        }
        if let Some(reason) = value.stop_reason.as_deref() {
            result = result.merge(stopped(reason));
        }
        if let Some(Delta::Stop {
            stop_reason: Some(reason),
        }) = &value.delta
        {
            result = result.merge(stopped(reason));
        }
        if let Some(choices) = &value.choices {
            if !choices.is_empty() {
                let states: Vec<_> = choices
                    .iter()
                    .map(|choice| {
                        choice
                            .finish_reason
                            .as_deref()
                            .map(stopped)
                            .unwrap_or_default()
                    })
                    .collect();
                if states.contains(&Completion::Incomplete) {
                    result = result.merge(Completion::Incomplete);
                } else if states.iter().all(|s| *s == Completion::Completed) {
                    result = result.merge(Completion::Completed);
                }
            }
        }
        result
    }
    serde_json::from_slice::<Envelope>(bytes).map(|value| classify(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_markers_not_output_words_or_null_errors_determine_completion() {
        assert_eq!(
            inspect(br#"{"type":"response.output_text.delta","delta":"response.completed"}"#)
                .unwrap(),
            Completion::Unknown
        );
        assert_eq!(
            inspect(br#"{"status":"completed","error":null}"#).unwrap(),
            Completion::Completed
        );
        assert_eq!(
            inspect(br#"{"choices":[{"finish_reason":"stop"}]}"#).unwrap(),
            Completion::Completed
        );
        assert_eq!(
            inspect(br#"{"choices":[{"finish_reason":"stop"},{"finish_reason":null}]}"#).unwrap(),
            Completion::Unknown
        );
        assert_eq!(
            inspect(br#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#).unwrap(),
            Completion::Incomplete
        );
        assert_eq!(inspect(br#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}}"#).unwrap(), Completion::Failed);
        assert_eq!(
            Completion::Failed.merge(Completion::Completed),
            Completion::Failed
        );
        assert_eq!(
            Completion::Incomplete.merge(Completion::Completed),
            Completion::Incomplete
        );
    }
}
