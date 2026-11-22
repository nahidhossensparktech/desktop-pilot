//! Streaming Messages API client with retries (TRD §3.3–3.4).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::auth::{Credential, Credentials};
use crate::sse::SseParser;
use crate::{API_VERSION, LlmError, MESSAGES_URL, MODELS_URL, OAUTH_BETA};

const MAX_ATTEMPTS: u32 = 5;
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);
/// No bytes (not even `ping`) for this long → treat the stream as dead.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Token usage of one response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_input_tokens
            + self.cache_creation_input_tokens
    }
}

/// Live updates while a response streams in.
#[derive(Clone, Debug)]
pub enum StreamEvent {
    /// Visible text from Claude.
    Text(String),
    /// A progress note between tool calls (`thinking.display: "updates"`).
    Thinking(String),
    /// A retryable failure; the request is retried after `wait`.
    Retrying { reason: String, wait: Duration },
}

/// A complete response.
#[derive(Clone, Debug)]
pub struct MessageResult {
    /// Content blocks exactly as received (to be replayed unchanged).
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub stop_details: Option<Value>,
    pub usage: Usage,
}

#[derive(Debug)]
pub struct ClaudeClient {
    http: reqwest::Client,
    credentials: Arc<Credentials>,
}

fn usage_from(v: &Value, into: &mut Usage) {
    let get = |k: &str| v.get(k).and_then(Value::as_u64);
    if let Some(n) = get("input_tokens") {
        into.input_tokens = n;
    }
    if let Some(n) = get("output_tokens") {
        into.output_tokens = n;
    }
    if let Some(n) = get("cache_read_input_tokens") {
        into.cache_read_input_tokens = n;
    }
    if let Some(n) = get("cache_creation_input_tokens") {
        into.cache_creation_input_tokens = n;
    }
}

fn api_error(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let kind = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown_error")
        .to_owned();
    let message = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/message"))
        .and_then(Value::as_str)
        .map_or_else(|| body.chars().take(300).collect(), str::to_owned);
    match status {
        429 => LlmError::RateLimited {
            retry_after_secs: retry_after,
        },
        529 => LlmError::Overloaded,
        _ => LlmError::Api {
            status,
            kind,
            message,
        },
    }
}

/// Exponential backoff with jitter, honoring `retry-after` (TRD §3.4).
fn backoff(attempt: u32, err: &LlmError) -> Duration {
    if let LlmError::RateLimited {
        retry_after_secs: Some(s),
    } = err
    {
        return Duration::from_secs((*s).min(120));
    }
    let exp = BACKOFF_BASE
        .saturating_mul(1 << attempt.min(6))
        .min(BACKOFF_CAP);
    // Cheap jitter without a RNG crate: sub-second clock noise.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let jitter = f64::from(nanos % 1000) / 1000.0; // 0..1
    exp.mul_f64(0.5 + jitter / 2.0)
}

/// Accumulates SSE events into content blocks.
#[derive(Debug, Default)]
struct Accumulator {
    blocks: Vec<Value>,
    partial_json: HashMap<usize, String>,
    stop_reason: Option<String>,
    stop_details: Option<Value>,
    usage: Usage,
}

impl Accumulator {
    fn handle(
        &mut self,
        event: &str,
        data: &Value,
        on: &mut dyn FnMut(StreamEvent),
    ) -> Result<(), LlmError> {
        match event {
            "message_start" => {
                if let Some(u) = data.pointer("/message/usage") {
                    usage_from(u, &mut self.usage);
                }
            }
            "content_block_start" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                let block = data["content_block"].clone();
                if self.blocks.len() <= index {
                    self.blocks.resize(index + 1, Value::Null);
                }
                self.blocks[index] = block;
            }
            "content_block_delta" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                let delta = &data["delta"];
                let Some(block) = self.blocks.get_mut(index) else {
                    return Err(LlmError::Stream(format!("delta for unknown block {index}")));
                };
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = delta["text"].as_str().unwrap_or_default();
                        append_str(block, "text", t);
                        on(StreamEvent::Text(t.to_owned()));
                    }
                    "thinking_delta" => {
                        let t = delta["thinking"].as_str().unwrap_or_default();
                        append_str(block, "thinking", t);
                        if !t.is_empty() {
                            on(StreamEvent::Thinking(t.to_owned()));
                        }
                    }
                    "signature_delta" => {
                        block["signature"] = delta["signature"].clone();
                    }
                    "input_json_delta" => {
                        self.partial_json
                            .entry(index)
                            .or_default()
                            .push_str(delta["partial_json"].as_str().unwrap_or_default());
                    }
                    "citations_delta" => {
                        if !block["citations"].is_array() {
                            block["citations"] = json!([]);
                        }
                        if let Some(arr) = block["citations"].as_array_mut() {
                            arr.push(delta["citation"].clone());
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                if let Some(raw) = self.partial_json.remove(&index)
                    && let Some(block) = self.blocks.get_mut(index)
                {
                    let input = if raw.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&raw).map_err(|e| {
                            LlmError::Stream(format!("tool input is not valid JSON: {e}"))
                        })?
                    };
                    block["input"] = input;
                }
            }
            "message_delta" => {
                if let Some(r) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(r.to_owned());
                }
                if let Some(d) = data.pointer("/delta/stop_details")
                    && !d.is_null()
                {
                    self.stop_details = Some(d.clone());
                }
                if let Some(u) = data.get("usage") {
                    usage_from(u, &mut self.usage);
                }
            }
            "error" => {
                let kind = data
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or("error");
                let message = data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return Err(if kind == "overloaded_error" {
                    LlmError::Overloaded
                } else {
                    LlmError::Stream(format!("{kind}: {message}"))
                });
            }
            _ => {} // ping, message_stop, unknown future events
        }
        Ok(())
    }
}

fn append_str(block: &mut Value, key: &str, text: &str) {
    let current = block[key].as_str().unwrap_or_default().to_owned();
    block[key] = Value::String(current + text);
}

impl ClaudeClient {
    pub fn new(credentials: Arc<Credentials>) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .user_agent(concat!("desktop-pilot/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(Self { http, credentials })
    }

    pub fn credentials(&self) -> &Arc<Credentials> {
        &self.credentials
    }

    async fn headers(&self, betas: &[&str], refresh: bool) -> Result<HeaderMap, LlmError> {
        let resolved = self.credentials.resolve(refresh).await?;
        let mut h = HeaderMap::new();
        h.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        let mut beta_list: Vec<&str> = betas.to_vec();
        let bad = |_| LlmError::NoCredentials("credential contains invalid characters".into());
        match &resolved.credential {
            Credential::ApiKey(k) => {
                let mut v = HeaderValue::from_str(k.expose_secret()).map_err(bad)?;
                v.set_sensitive(true);
                h.insert("x-api-key", v);
            }
            Credential::OAuthToken(t) => {
                let mut v =
                    HeaderValue::from_str(&format!("Bearer {}", t.expose_secret())).map_err(bad)?;
                v.set_sensitive(true);
                h.insert("authorization", v);
                beta_list.push(OAUTH_BETA);
            }
        }
        if !beta_list.is_empty() {
            let joined = beta_list.join(",");
            h.insert(
                "anthropic-beta",
                HeaderValue::from_str(&joined).map_err(|e| LlmError::Stream(e.to_string()))?,
            );
        }
        Ok(h)
    }

    /// Checks credentials with a cheap request. Returns a short description.
    pub async fn test_connection(&self) -> Result<String, LlmError> {
        for refresh in [false, true] {
            let headers = self.headers(&[], refresh).await?;
            let resp = self
                .http
                .get(format!("{MODELS_URL}?limit=1"))
                .headers(headers)
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| LlmError::Network(e.to_string()))?;
            let status = resp.status().as_u16();
            if status == 401 && !refresh {
                continue;
            }
            let body = resp.text().await.unwrap_or_default();
            if status == 200 {
                let source = self
                    .credentials
                    .preferred_source()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                return Ok(format!("Connected using {source}"));
            }
            return Err(api_error(status, &body, None));
        }
        Err(LlmError::NoCredentials("authentication failed".into()))
    }

    /// Sends a streaming Messages request, retrying transient failures.
    pub async fn stream_message(
        &self,
        body: &Value,
        betas: &[&str],
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<MessageResult, LlmError> {
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let result = tokio::select! {
                () = cancel.cancelled() => Err(LlmError::Cancelled),
                r = self.stream_once(body, betas, refreshed, on_event) => r,
            };
            match result {
                Ok(r) => return Ok(r),
                Err(LlmError::Api { status: 401, .. }) if !refreshed => {
                    // Token may have expired: ask `ant` for a fresh one once.
                    refreshed = true;
                }
                Err(e) if e.is_retryable() && attempt + 1 < MAX_ATTEMPTS => {
                    let wait = backoff(attempt, &e);
                    attempt += 1;
                    on_event(StreamEvent::Retrying {
                        reason: e.to_string(),
                        wait,
                    });
                    tokio::select! {
                        () = cancel.cancelled() => return Err(LlmError::Cancelled),
                        () = tokio::time::sleep(wait) => {}
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn stream_once(
        &self,
        body: &Value,
        betas: &[&str],
        refresh: bool,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<MessageResult, LlmError> {
        let headers = self.headers(betas, refresh).await?;
        let resp = self
            .http
            .post(MESSAGES_URL)
            .headers(headers)
            .header("accept", "text/event-stream")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok());
            let text = resp.text().await.unwrap_or_default();
            return Err(api_error(status, &text, retry_after));
        }

        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::default();
        let mut acc = Accumulator::default();
        let mut finished = false;
        while !finished {
            let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next())
                .await
                .map_err(|_| LlmError::Stream("no data from the API for 90 s".into()))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| LlmError::Network(e.to_string()))?;
            for ev in parser.push(&chunk) {
                if ev.event == "message_stop" {
                    finished = true;
                }
                let data: Value = serde_json::from_str(&ev.data).unwrap_or(Value::Null);
                acc.handle(&ev.event, &data, on_event)?;
            }
        }
        if !finished {
            return Err(LlmError::Stream("stream ended before message_stop".into()));
        }
        Ok(MessageResult {
            content: acc.blocks.into_iter().filter(|b| !b.is_null()).collect(),
            stop_reason: acc.stop_reason,
            stop_details: acc.stop_details,
            usage: acc.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(events: &[(&str, Value)]) -> (Accumulator, Vec<String>) {
        let mut acc = Accumulator::default();
        let mut texts = Vec::new();
        for (name, data) in events {
            acc.handle(name, data, &mut |e| {
                if let StreamEvent::Text(t) | StreamEvent::Thinking(t) = e {
                    texts.push(t);
                }
            })
            .unwrap();
        }
        (acc, texts)
    }

    #[test]
    fn accumulates_text_thinking_and_tool_use() {
        let (acc, texts) = feed(&[
            (
                "message_start",
                json!({"message": {"usage": {"input_tokens": 100, "cache_read_input_tokens": 50}}}),
            ),
            (
                "content_block_start",
                json!({"index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            ),
            (
                "content_block_delta",
                json!({"index": 0, "delta": {"type": "thinking_delta", "thinking": "Opening Notepad"}}),
            ),
            (
                "content_block_delta",
                json!({"index": 0, "delta": {"type": "signature_delta", "signature": "sig=="}}),
            ),
            ("content_block_stop", json!({"index": 0})),
            (
                "content_block_start",
                json!({"index": 1, "content_block": {"type": "text", "text": ""}}),
            ),
            (
                "content_block_delta",
                json!({"index": 1, "delta": {"type": "text_delta", "text": "Hel"}}),
            ),
            (
                "content_block_delta",
                json!({"index": 1, "delta": {"type": "text_delta", "text": "lo"}}),
            ),
            ("content_block_stop", json!({"index": 1})),
            (
                "content_block_start",
                json!({"index": 2, "content_block": {"type": "tool_use", "id": "tu_1", "name": "left_click", "toolset_name": "computer", "input": {}}}),
            ),
            (
                "content_block_delta",
                json!({"index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"coordinate\": [1"}}),
            ),
            (
                "content_block_delta",
                json!({"index": 2, "delta": {"type": "input_json_delta", "partial_json": "0, 20]}"}}),
            ),
            ("content_block_stop", json!({"index": 2})),
            (
                "message_delta",
                json!({"delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 42}}),
            ),
        ]);
        assert_eq!(texts, vec!["Opening Notepad", "Hel", "lo"]);
        assert_eq!(acc.blocks[0]["signature"], "sig==");
        assert_eq!(acc.blocks[1]["text"], "Hello");
        assert_eq!(acc.blocks[2]["input"], json!({"coordinate": [10, 20]}));
        assert_eq!(acc.blocks[2]["toolset_name"], "computer");
        assert_eq!(acc.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(acc.usage.output_tokens, 42);
        assert_eq!(acc.usage.cache_read_input_tokens, 50);
    }

    #[test]
    fn empty_tool_input_becomes_empty_object() {
        let (acc, _) = feed(&[
            (
                "content_block_start",
                json!({"index": 0, "content_block": {"type": "tool_use", "id": "t", "name": "screenshot", "input": {}}}),
            ),
            ("content_block_stop", json!({"index": 0})),
        ]);
        assert_eq!(acc.blocks[0]["input"], json!({}));
    }

    #[test]
    fn stream_error_event_is_reported() {
        let mut acc = Accumulator::default();
        let err = acc
            .handle(
                "error",
                &json!({"error": {"type": "overloaded_error", "message": "busy"}}),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, LlmError::Overloaded));
    }

    #[test]
    fn api_errors_are_classified() {
        let e = api_error(429, "{}", Some(7));
        assert!(matches!(
            e,
            LlmError::RateLimited {
                retry_after_secs: Some(7)
            }
        ));
        assert_eq!(backoff(0, &e), Duration::from_secs(7));
        let e = api_error(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#,
            None,
        );
        assert!(matches!(e, LlmError::Api { status: 400, ref message, .. } if message == "bad"));
        let b = backoff(3, &LlmError::Overloaded);
        assert!(b >= Duration::from_secs(4) && b <= Duration::from_secs(8));
    }
}
