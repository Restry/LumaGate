//! Observe usage in the final selected upstream response, before protocol conversion.
//! No tokenization, synthetic zeroes, request modification, or unbounded SSE buffering.
use super::{completion::Completion, logs};
use crate::{provider::Provider, proxy::hyper_client::ProxyResponse};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

// 200 records can be added by the JS UI without exceeding Number.MAX_SAFE_INTEGER.
const MAX_COUNT: u64 = 9_007_199_254_740_991 / 200;
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub state: UsageState,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub reason: Option<&'static str>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageState {
    Pending,
    Reported,
    Partial,
    Unavailable,
}
impl Usage {
    fn empty(state: UsageState, reason: Option<&'static str>) -> Self {
        Self {
            state,
            reason,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wire {
    Chat,
    Responses,
    Anthropic,
}
#[derive(Default)]
struct TrackerState {
    wire: Option<Wire>,
    streaming: bool,
    body_read: bool,
    usage: Option<Usage>,
    completion: Completion,
}
#[derive(Clone, Default)]
pub struct Tracker(Arc<Mutex<TrackerState>>);
impl Tracker {
    pub fn snapshot(&self) -> Option<Usage> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .usage
            .clone()
    }
    pub fn closed_snapshot(&self) -> Option<Usage> {
        self.snapshot().map(|usage| {
            if usage.state == UsageState::Pending {
                Usage::empty(UsageState::Unavailable, Some("not_reported"))
            } else {
                usage
            }
        })
    }
    pub fn completion(&self) -> Completion {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).completion
    }
    fn publish_completion(&self, value: Completion) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).completion = value;
    }
    fn begin(&self, wire: Wire, streaming: bool) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *state = TrackerState {
            wire: Some(wire),
            streaming,
            body_read: false,
            usage: Some(Usage::empty(UsageState::Pending, None)),
            completion: Completion::Unknown,
        };
    }
    fn publish(&self, usage: Usage) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).usage = Some(usage);
    }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
enum Kind {
    #[serde(rename = "message")]
    Message,
    #[serde(rename = "message_start")]
    MessageStart,
    #[serde(rename = "message_delta")]
    MessageDelta,
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "response.completed")]
    Completed,
    #[serde(rename = "response.incomplete")]
    Incomplete,
    #[serde(rename = "response.failed")]
    Failed,
    #[serde(rename = "error")]
    Error,
    #[default]
    #[serde(other)]
    Other,
}
impl Kind {
    fn from_event(value: &[u8]) -> Self {
        match value {
            b"message_start" => Self::MessageStart,
            b"message_delta" => Self::MessageDelta,
            b"message_stop" => Self::MessageStop,
            b"response.completed" => Self::Completed,
            b"response.incomplete" => Self::Incomplete,
            b"response.failed" => Self::Failed,
            b"error" => Self::Error,
            _ => Self::Other,
        }
    }
}
#[derive(Default, Deserialize)]
struct Details {
    cached_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
}
#[derive(Default, Deserialize)]
struct CacheWrite {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}
#[derive(Default, Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    input_tokens_details: Option<Details>,
    output_tokens_details: Option<Details>,
    prompt_tokens_details: Option<Details>,
    completion_tokens_details: Option<Details>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    prompt_cache_hit_tokens: Option<u64>,
    cache_creation: Option<CacheWrite>,
}
#[derive(Deserialize)]
struct Nested {
    usage: Option<RawUsage>,
}
#[derive(Deserialize)]
enum StopReason {
    #[serde(rename = "end_turn")]
    EndTurn,
    #[serde(rename = "max_tokens")]
    MaxTokens,
    #[serde(rename = "stop_sequence")]
    StopSequence,
    #[serde(rename = "tool_use")]
    ToolUse,
    #[serde(rename = "pause_turn")]
    PauseTurn,
    #[serde(rename = "refusal")]
    Refusal,
    #[serde(rename = "model_context_window_exceeded")]
    ContextLimit,
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Delta {
    Stop { stop_reason: Option<StopReason> },
    Other(serde::de::IgnoredAny),
}
#[derive(Deserialize)]
struct Envelope {
    #[serde(default, rename = "type")]
    kind: Kind,
    usage: Option<RawUsage>,
    response: Option<Nested>,
    message: Option<Nested>,
    delta: Option<Delta>,
    // IgnoredAny skips response text/tools without allocating or retaining their content.
    choices: Option<Vec<serde::de::IgnoredAny>>,
}
#[derive(Default)]
struct Accumulator {
    input: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
    read: Option<u64>,
    write: Option<u64>,
    reasoning: Option<u64>,
    seen: bool,
    invalid: bool,
    final_seen: bool,
    limited: bool,
}
impl Accumulator {
    fn update(&mut self, usage: RawUsage, wire: Wire, start: bool) {
        self.seen = true;
        let input = usage.prompt_tokens.or(usage.input_tokens);
        let output = usage.completion_tokens.or(usage.output_tokens);
        if let Some(value) = input {
            self.input = Some(value);
        }
        // Anthropic message_start's output=0 is an initial counter, not a final output total.
        if !start {
            if let Some(value) = output {
                self.output = Some(value);
            }
        }
        self.total = usage.total_tokens;
        let read = usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .or_else(|| {
                usage
                    .input_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens)
            })
            .or(usage.prompt_cache_hit_tokens)
            .or(usage.cache_read_input_tokens);
        if let Some(value) = read {
            self.read = Some(value);
        }
        let write = usage.cache_creation_input_tokens.or_else(|| {
            let nested = usage.cache_creation.as_ref()?;
            if nested.ephemeral_5m_input_tokens.is_none()
                && nested.ephemeral_1h_input_tokens.is_none()
            {
                return None;
            }
            match nested
                .ephemeral_5m_input_tokens
                .unwrap_or(0)
                .checked_add(nested.ephemeral_1h_input_tokens.unwrap_or(0))
            {
                Some(value) => Some(value),
                None => {
                    self.invalid = true;
                    None
                }
            }
        });
        if let Some(value) = write {
            self.write = Some(value);
        }
        let reasoning = usage
            .output_tokens_details
            .and_then(|d| d.reasoning_tokens)
            .or_else(|| {
                usage
                    .completion_tokens_details
                    .and_then(|d| d.reasoning_tokens)
            });
        if let Some(value) = reasoning {
            self.reasoning = Some(value);
        }
        if [
            self.input,
            self.output,
            self.total,
            self.read,
            self.write,
            self.reasoning,
        ]
        .into_iter()
        .flatten()
        .any(|v| v > MAX_COUNT)
        {
            self.invalid = true;
        }
        // OpenAI cache/reasoning counts are subsets, not additional billable-token totals.
        if wire != Wire::Anthropic
            && self
                .read
                .zip(self.input)
                .is_some_and(|(read, input)| read > input)
        {
            self.invalid = true;
        }
        if self
            .reasoning
            .zip(self.output)
            .is_some_and(|(reasoning, output)| reasoning > output)
        {
            self.invalid = true;
        }
    }
    fn result(&self, wire: Wire, clean_end: bool) -> Usage {
        // A skipped event could have carried a newer cumulative counter. Never certify an
        // earlier snapshot merely because a later [DONE] marker survived the size limit.
        if self.limited {
            return Usage::empty(UsageState::Unavailable, Some("event_too_large"));
        }
        if self.invalid {
            return Usage::empty(UsageState::Unavailable, Some("invalid_usage"));
        }
        let mut input = self.input;
        if wire == Wire::Anthropic {
            input = input
                .and_then(|v| v.checked_add(self.read.unwrap_or(0)))
                .and_then(|v| v.checked_add(self.write.unwrap_or(0)));
        }
        let sum = input.zip(self.output).and_then(|(a, b)| a.checked_add(b));
        if input.is_some_and(|v| v > MAX_COUNT)
            || sum.is_some_and(|v| v > MAX_COUNT)
            || self
                .total
                .zip(sum)
                .is_some_and(|(reported, sum)| reported != sum)
        {
            return Usage::empty(UsageState::Unavailable, Some("inconsistent_usage"));
        }
        let total = self.total.or(sum);
        let complete = input.is_some()
            && self.output.is_some()
            && total.is_some()
            && (clean_end || self.final_seen);
        let state = if complete {
            UsageState::Reported
        } else if input.is_some() || self.output.is_some() || total.is_some() {
            UsageState::Partial
        } else {
            UsageState::Unavailable
        };
        Usage {
            state,
            input_tokens: input,
            output_tokens: self.output,
            total_tokens: total,
            cache_read_tokens: self.read,
            cache_write_tokens: self.write,
            reasoning_tokens: self.reasoning,
            reason: if complete {
                None
            } else if self.limited {
                Some("event_too_large")
            } else if !clean_end && !self.final_seen {
                Some("incomplete_stream")
            } else if self.seen {
                Some("partial_usage")
            } else {
                Some("not_reported")
            },
        }
    }
}

/// Called once on decoded, non-streaming upstream bytes before a handler transforms them.
pub fn record_body(bytes: &[u8], still_encoded: bool) {
    let Some(tracker) = logs::usage_tracker() else {
        return;
    };
    let wire = {
        let mut state = tracker.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.streaming || state.body_read {
            return;
        }
        let Some(wire) = state.wire else {
            return;
        };
        state.body_read = true;
        wire
    };
    if still_encoded {
        tracker.publish(Usage::empty(
            UsageState::Unavailable,
            Some("encoded_response"),
        ));
        return;
    }
    tracker.publish_completion(super::completion::inspect(bytes).unwrap_or_default());
    let mut accumulator = Accumulator::default();
    match serde_json::from_slice::<Envelope>(bytes) {
        Ok(envelope) => {
            if let Some(usage) = envelope.usage {
                accumulator.update(usage, wire, false);
            }
        }
        Err(_) => {
            accumulator.invalid = true;
        }
    }
    tracker.publish(accumulator.result(wire, true));
}

struct SseCollector {
    wire: Wire,
    line: Vec<u8>,
    data: Vec<u8>,
    event: Kind,
    discard: bool,
    accumulator: Accumulator,
    completion: Completion,
    completion_invalid: bool,
}
impl SseCollector {
    fn new(wire: Wire) -> Self {
        Self {
            wire,
            line: vec![],
            data: vec![],
            event: Kind::Other,
            discard: false,
            accumulator: Accumulator::default(),
            completion: Completion::Unknown,
            completion_invalid: false,
        }
    }
    fn completion(&self) -> Completion {
        if self.completion == Completion::Failed {
            return Completion::Failed;
        }
        if self.accumulator.limited || self.completion_invalid {
            Completion::Unknown
        } else {
            self.completion
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.line();
                self.line.clear();
            } else if self.line.len() < MAX_EVENT_BYTES {
                self.line.push(byte);
            } else {
                self.discard = true;
                self.accumulator.limited = true;
            }
        }
    }
    fn line(&mut self) {
        let line = self.line.strip_suffix(b"\r").unwrap_or(&self.line);
        let line = line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line);
        if line.is_empty() {
            self.dispatch();
            self.discard = false;
            self.event = Kind::Other;
            return;
        }
        if self.discard {
            return;
        }
        if let Some(event) = line.strip_prefix(b"event:") {
            self.event = Kind::from_event(event.trim_ascii());
        }
        if let Some(data) = line.strip_prefix(b"data:") {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            if self.data.len() + data.len() + 1 > MAX_EVENT_BYTES {
                self.discard = true;
                self.accumulator.limited = true;
                self.data.clear();
            } else {
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(data);
            }
        }
    }
    fn dispatch(&mut self) {
        let data = std::mem::take(&mut self.data);
        if self.discard || data.is_empty() {
            return;
        }
        if data.trim_ascii() == b"[DONE]" {
            self.accumulator.final_seen = true;
            self.completion = self.completion.merge(Completion::Completed);
            return;
        }
        match super::completion::inspect(&data) {
            Ok(value) => {
                let named = match self.event {
                    Kind::Completed | Kind::MessageStop => Completion::Completed,
                    Kind::Failed | Kind::Error => Completion::Failed,
                    Kind::Incomplete => Completion::Incomplete,
                    _ => Completion::Unknown,
                };
                self.completion = self.completion.merge(value).merge(named);
            }
            Err(_) => self.completion_invalid = true,
        }
        match serde_json::from_slice::<Envelope>(&data) {
            Ok(mut envelope) => {
                if envelope.kind == Kind::Other {
                    envelope.kind = self.event;
                }
                match envelope.kind {
                    Kind::MessageStart => {
                        if let Some(usage) = envelope.message.and_then(|m| m.usage) {
                            self.accumulator.update(usage, self.wire, true);
                        }
                    }
                    Kind::MessageDelta => {
                        if let Some(usage) = envelope.usage {
                            self.accumulator.update(usage, self.wire, false);
                        }
                        if envelope.delta.is_some_and(|delta| matches!(delta,
                            Delta::Stop { stop_reason: Some(reason) } if !matches!(reason, StopReason::Other)))
                        {
                            self.accumulator.final_seen = true;
                        }
                    }
                    Kind::MessageStop => self.accumulator.final_seen = true,
                    Kind::Completed | Kind::Incomplete | Kind::Failed => {
                        if let Some(usage) = envelope.response.and_then(|r| r.usage) {
                            self.accumulator.update(usage, self.wire, false);
                        }
                        self.accumulator.final_seen = true;
                    }
                    _ if self.wire == Wire::Chat => {
                        if let Some(usage) = envelope.usage {
                            self.accumulator.update(usage, self.wire, false);
                            if envelope.choices.is_some_and(|values| values.is_empty()) {
                                self.accumulator.final_seen = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(_) => self.accumulator.invalid = true,
        }
    }
    fn finish(&mut self, _clean: bool) -> Usage {
        if !self.line.is_empty() {
            self.line();
            self.line.clear();
        }
        // A clean TCP EOF alone does not prove model completion; require a protocol final marker.
        self.dispatch();
        self.accumulator.result(self.wire, false)
    }
}
struct ObservedStream {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    collector: SseCollector,
    tracker: Tracker,
    finished: bool,
}
impl ObservedStream {
    fn finish(&mut self, clean: bool) {
        if !self.finished {
            self.finished = true;
            self.tracker.publish(self.collector.finish(clean));
            self.tracker.publish_completion(self.collector.completion());
        }
    }
}
impl Stream for ObservedStream {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = this.stream.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(Some(Ok(bytes))) => {
                this.collector.feed(bytes);
                this.tracker.publish_completion(this.collector.completion());
                if this.collector.accumulator.seen
                    || this.collector.accumulator.invalid
                    || this.collector.accumulator.limited
                {
                    this.tracker.publish(
                        this.collector
                            .accumulator
                            .result(this.collector.wire, false),
                    );
                }
            }
            Poll::Ready(Some(Err(_))) => this.finish(false),
            Poll::Ready(None) => this.finish(true),
            _ => {}
        }
        polled
    }
}
impl Drop for ObservedStream {
    fn drop(&mut self) {
        self.finish(false);
    }
}

/// An identity operation outside Manual request logging. The selected raw response is observed,
/// not a synthetic response produced by an adapter; byte chunks and backpressure are preserved.
pub fn observe(response: ProxyResponse, provider: &Provider) -> ProxyResponse {
    if provider
        .settings_config
        .get("manual_upstream_model")
        .is_none()
    {
        return response;
    }
    let Some(tracker) = logs::usage_tracker() else {
        return response;
    };
    let wire = match provider
        .settings_config
        .get("api_format")
        .and_then(|v| v.as_str())
    {
        Some("anthropic") => Wire::Anthropic,
        Some("openai_chat") => Wire::Chat,
        Some("openai_responses") => Wire::Responses,
        _ => return response,
    };
    let streaming = response.content_type().is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("text/event-stream")
    });
    tracker.begin(wire, streaming);
    if !streaming {
        return response;
    }
    if response
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
    {
        tracker.publish(Usage::empty(
            UsageState::Unavailable,
            Some("encoded_response"),
        ));
        return response;
    }
    let status = response.status();
    let headers = response.headers().clone();
    let stream = ObservedStream {
        stream: Box::pin(response.bytes_stream()),
        collector: SseCollector::new(wire),
        tracker,
        finished: false,
    };
    ProxyResponse::streamed(status, headers, stream)
}

#[cfg(test)]
mod tests;
