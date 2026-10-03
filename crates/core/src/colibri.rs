use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;

use crate::config::ColibriConfig;
use crate::error::{Error, Result};
use crate::telemetry::{ColibriStatus, ExpertSnapshot, Health, Profile};
use crate::types::{
    ChatMessage, ChatRequest, ChatResponse, Completion, ModelInfo, ModelList, Role, StreamEvent,
    ToolCall, Usage,
};

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE_WAIT_HEADER: &str = "x-colibri-queue-wait-ms";

#[derive(Debug, Clone)]
pub struct ColibriClient {
    http: reqwest::Client,
    config: ColibriConfig,
}

impl ColibriClient {
    pub fn new(config: ColibriConfig) -> Result<Self> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("aviary/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &ColibriConfig {
        &self.config
    }

    pub async fn models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.config.api_base());
        let response = self
            .send(self.http.get(&url).timeout(STATUS_TIMEOUT), &url)
            .await?;
        let list: ModelList = response.json().await?;
        Ok(list.data)
    }

    pub async fn health(&self) -> Result<Health> {
        self.get_root("/health").await
    }

    pub async fn experts(&self) -> Result<ExpertSnapshot> {
        self.get_root("/experts").await
    }

    pub async fn profile(&self) -> Result<Profile> {
        self.get_root("/profile").await
    }

    pub async fn status(&self) -> ColibriStatus {
        let base = self.config.api_base();
        let model = &self.config.model;
        let health = match self.health().await {
            Ok(health) => health,
            Err(err) => return ColibriStatus::unreachable(base, model, err.to_string()),
        };
        let (models, experts, profile) =
            tokio::join!(self.models(), self.experts(), self.profile());
        let mut status = ColibriStatus::assemble(
            base,
            model,
            health,
            models.as_ref().cloned().unwrap_or_default(),
            experts.ok(),
            profile.ok(),
        );
        if let Err(err) = models {
            status.error = Some(err.to_string());
        }
        status
    }

    pub async fn complete(&self, mut request: ChatRequest) -> Result<Completion> {
        request.stream = false;
        request.stream_options = None;
        let url = format!("{}/chat/completions", self.config.api_base());
        let builder = self
            .http
            .post(&url)
            .timeout(self.config.timeout())
            .json(&request);
        let response = self.send(builder, &url).await?;
        let queue_wait_ms = queue_wait(&response);
        let body: ChatResponse = response.json().await?;
        let choice = body
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("response contained no choices".into()))?;
        Ok(Completion {
            message: choice.message,
            finish_reason: choice.finish_reason,
            usage: body.usage.unwrap_or_default(),
            queue_wait_ms,
        })
    }

    pub async fn stream<F>(&self, mut request: ChatRequest, mut on_event: F) -> Result<Completion>
    where
        F: FnMut(StreamEvent),
    {
        request.stream = true;
        request.stream_options = Some(crate::types::StreamOptions {
            include_usage: true,
        });
        let url = format!("{}/chat/completions", self.config.api_base());
        let builder = self
            .http
            .post(&url)
            .header("accept", "text/event-stream")
            .json(&request);
        let response = self.send(builder, &url).await?;
        let queue_wait_ms = queue_wait(&response);

        let idle = self.config.timeout();
        let mut body = response.bytes_stream();
        let mut parser = SseParser::default();
        let mut acc = StreamAccumulator::default();

        'outer: loop {
            let chunk = match tokio::time::timeout(idle, body.next()).await {
                Ok(Some(chunk)) => chunk?,
                Ok(None) => break,
                Err(_) => {
                    return Err(Error::Protocol(format!(
                        "no data from Colibri for {}s",
                        idle.as_secs()
                    )));
                }
            };
            for payload in parser.push(&chunk) {
                if payload == "[DONE]" {
                    break 'outer;
                }
                let value: Value = serde_json::from_str(&payload)?;
                if let Some(err) = value.get("error") {
                    return Err(api_error_from_value(500, err));
                }
                for event in acc.apply(&value) {
                    on_event(event);
                }
            }
        }

        let completion = acc.finish(queue_wait_ms);
        on_event(StreamEvent::Done(completion.clone()));
        Ok(completion)
    }

    async fn get_root<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.config.server_root(), path);
        let response = self
            .send(self.http.get(&url).timeout(STATUS_TIMEOUT), &url)
            .await?;
        Ok(response.json().await?)
    }

    async fn send(&self, builder: RequestBuilder, url: &str) -> Result<Response> {
        let builder = match &self.config.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        };
        let response = builder.send().await.map_err(|source| {
            if source.is_connect() || source.is_timeout() {
                Error::Connect {
                    url: url.to_string(),
                    source,
                }
            } else {
                Error::Http(source)
            }
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let text = response.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<Value>(&text).ok();
        let error = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .map(|e| api_error_from_value(status.as_u16(), e))
            .unwrap_or_else(|| Error::Api {
                status: status.as_u16(),
                code: None,
                message: if text.is_empty() {
                    status.canonical_reason().unwrap_or("error").to_string()
                } else {
                    text
                },
            });
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::Busy(error.to_string()));
        }
        Err(error)
    }
}

fn queue_wait(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(QUEUE_WAIT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as u64)
}

fn api_error_from_value(status: u16, error: &Value) -> Error {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("unknown error")
        .to_string();
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .map(str::to_string);
    Error::Api {
        status,
        code,
        message,
    }
}

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() {
                    out.push(data.to_string());
                }
            }
        }
        out
    }
}

#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct StreamAccumulator {
    content: String,
    reasoning: String,
    calls: Vec<PartialCall>,
    finish_reason: Option<String>,
    usage: Usage,
}

impl StreamAccumulator {
    fn apply(&mut self, chunk: &Value) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            if let Ok(usage) = serde_json::from_value::<Usage>(usage.clone()) {
                self.usage = usage;
            }
        }
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return events;
        };
        for choice in choices {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                if !text.is_empty() {
                    self.reasoning.push_str(text);
                    events.push(StreamEvent::Reasoning(text.to_string()));
                }
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    self.content.push_str(text);
                    events.push(StreamEvent::Content(text.to_string()));
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    self.apply_call(call);
                }
            }
        }
        events
    }

    fn apply_call(&mut self, call: &Value) {
        let index = call
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or(self.calls.len());
        if self.calls.len() <= index {
            self.calls.resize_with(index + 1, PartialCall::default);
        }
        let slot = &mut self.calls[index];
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            slot.id = id.to_string();
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if slot.name.is_empty() {
                    slot.name = name.to_string();
                }
            }
            if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                slot.arguments.push_str(args);
            }
        }
    }

    fn finish(self, queue_wait_ms: Option<u64>) -> Completion {
        let calls: Vec<ToolCall> = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.name.is_empty())
            .map(|(i, c)| {
                let id = if c.id.is_empty() {
                    format!("call_{i}")
                } else {
                    c.id
                };
                ToolCall::new(id, c.name, c.arguments)
            })
            .collect();
        let mut message = ChatMessage::new(Role::Assistant, self.content);
        message.tool_calls = calls;
        if !self.reasoning.is_empty() {
            message.reasoning_content = Some(self.reasoning);
        }
        Completion {
            message,
            finish_reason: self.finish_reason,
            usage: self.usage,
            queue_wait_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sse_parser_handles_split_chunks_and_keepalives() {
        let mut parser = SseParser::default();
        assert!(parser.push(b": keepalive\n\ndata: {\"a\"").is_empty());
        assert_eq!(
            parser.push(b":1}\r\n\r\ndata: [DONE]\n\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
    }

    #[test]
    fn sse_parser_keeps_multibyte_characters_intact() {
        let mut parser = SseParser::default();
        let text = "data: \"Gr\u{fc}\u{df}e\"\n".as_bytes();
        let (a, b) = text.split_at(10);
        assert!(parser.push(a).is_empty());
        assert_eq!(parser.push(b), vec!["\"Gr\u{fc}\u{df}e\""]);
    }

    #[test]
    fn accumulator_merges_tool_call_deltas() {
        let mut acc = StreamAccumulator::default();
        acc.apply(&json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_a", "function": {"name": "read_file", "arguments": "{\"pa"}}
        ]}}]}));
        acc.apply(&json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "function": {"arguments": "th\":\"x\"}"}}
        ]}, "finish_reason": "tool_calls"}]}));
        acc.apply(&json!({"choices": [], "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}));
        let done = acc.finish(Some(12));
        assert_eq!(
            done.message.tool_calls,
            vec![ToolCall::new("call_a", "read_file", "{\"path\":\"x\"}")]
        );
        assert_eq!(done.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(done.usage.total_tokens, 5);
        assert_eq!(done.queue_wait_ms, Some(12));
    }

    #[test]
    fn accumulator_ignores_empty_keepalive_deltas() {
        let mut acc = StreamAccumulator::default();
        let events = acc.apply(&json!({"choices": [{"delta": {"reasoning_content": ""}}]}));
        assert!(events.is_empty());
        let events = acc.apply(&json!({"choices": [{"delta": {"content": "Hi"}}]}));
        assert_eq!(events, vec![StreamEvent::Content("Hi".into())]);
    }
}
