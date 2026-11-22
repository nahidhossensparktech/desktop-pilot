//! Google Gemini API client (REST `generateContent`, streaming, function calling).
//!
//! Model turns are kept exactly as received: Gemini 3/4 attach a `thoughtSignature` to
//! some parts and reject a history where those parts were changed, merged or dropped.

use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::LlmError;
use crate::sse::SseParser;

const BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const MAX_ATTEMPTS: u32 = 4;
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Where the Gemini key comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeminiKeySource {
    Env(&'static str),
    Saved,
}

impl std::fmt::Display for GeminiKeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env(name) => write!(f, "{name} environment variable"),
            Self::Saved => f.write_str("Gemini API key saved in Credential Manager"),
        }
    }
}

/// A live-listed Gemini model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeminiModel {
    pub id: String,
    pub display_name: String,
}

/// Live events while a response streams in.
#[derive(Clone, Debug)]
pub enum GeminiEvent {
    Text(String),
    Retrying(String),
}

/// One complete model turn.
#[derive(Clone, Debug, Default)]
pub struct GeminiTurn {
    /// The model's parts exactly as received (to replay in the next request).
    pub parts: Vec<Value>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

impl GeminiTurn {
    pub fn function_calls(&self) -> Vec<&Value> {
        self.parts
            .iter()
            .filter(|p| p.get("functionCall").is_some())
            .collect()
    }

    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter(|p| p.get("thought") != Some(&Value::Bool(true)))
            .filter_map(|p| p["text"].as_str())
            .collect::<String>()
            .trim()
            .to_owned()
    }
}

#[derive(Debug)]
pub struct GeminiClient {
    http: reqwest::Client,
    saved_key: Mutex<Option<SecretString>>,
}

fn env_key() -> Option<(SecretString, &'static str)> {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|v| (SecretString::from(v), name))
        })
}

fn api_error(status: u16, body: &str) -> LlmError {
    let v: Option<Value> = serde_json::from_str(body).ok();
    let message = v
        .as_ref()
        .and_then(|v| v.pointer("/error/message"))
        .and_then(Value::as_str)
        .map_or_else(|| body.chars().take(300).collect(), str::to_owned);
    let kind = v
        .as_ref()
        .and_then(|v| v.pointer("/error/status"))
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_owned();
    match status {
        429 => LlmError::RateLimited {
            retry_after_secs: None,
        },
        503 => LlmError::Overloaded,
        _ => LlmError::Api {
            status,
            kind,
            message,
        },
    }
}

fn is_chat_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.starts_with("gemini")
        && ![
            "embedding",
            "image",
            "tts",
            "live",
            "audio",
            "aqa",
            "veo",
            "imagen",
            "robotics",
        ]
        .iter()
        .any(|bad| id.contains(bad))
}

impl GeminiClient {
    pub fn new(saved_key: Option<SecretString>) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .user_agent(concat!("desktop-pilot/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(Self {
            http,
            saved_key: Mutex::new(saved_key),
        })
    }

    pub fn set_saved_key(&self, key: Option<SecretString>) {
        *self
            .saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = key;
    }

    pub fn key_source(&self) -> Option<GeminiKeySource> {
        if let Some((_, name)) = env_key() {
            return Some(GeminiKeySource::Env(name));
        }
        self.saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|_| GeminiKeySource::Saved)
    }

    fn key(&self) -> Result<SecretString, LlmError> {
        if let Some((k, _)) = env_key() {
            return Ok(k);
        }
        self.saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                LlmError::NoCredentials(
                    "no Gemini API key: add one in Settings (aistudio.google.com/apikey)".into(),
                )
            })
    }

    /// Lists chat-capable Gemini models available to this key.
    pub async fn list_models(&self) -> Result<Vec<GeminiModel>, LlmError> {
        let key = self.key()?;
        let resp = self
            .http
            .get(format!("{BASE}/models?pageSize=1000"))
            .header("x-goog-api-key", key.expose_secret())
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        if status != 200 {
            return Err(api_error(status, &body));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| LlmError::Stream(e.to_string()))?;
        let mut models: Vec<GeminiModel> = v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x == "generateContent"))
            })
            .filter_map(|m| {
                let id = m["name"].as_str()?.trim_start_matches("models/").to_owned();
                is_chat_model(&id).then(|| GeminiModel {
                    display_name: m["displayName"].as_str().unwrap_or(&id).to_owned(),
                    id,
                })
            })
            .collect();
        // "Pro" models first (that's what most users look for), then the rest.
        models.sort_by_key(|m| {
            (
                !m.id.contains("pro"),
                m.id.contains("preview"),
                m.id.clone(),
            )
        });
        Ok(models)
    }

    /// Streams one model turn, retrying transient failures.
    pub async fn stream(
        &self,
        model: &str,
        body: &Value,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(GeminiEvent) + Send),
    ) -> Result<GeminiTurn, LlmError> {
        let mut attempt = 0;
        loop {
            let result = tokio::select! {
                () = cancel.cancelled() => Err(LlmError::Cancelled),
                r = self.stream_once(model, body, on_event) => r,
            };
            match result {
                Err(e) if e.is_retryable() && attempt + 1 < MAX_ATTEMPTS => {
                    let wait = Duration::from_secs(2_u64.pow(attempt + 1));
                    attempt += 1;
                    on_event(GeminiEvent::Retrying(format!(
                        "{e}; retrying in {} s",
                        wait.as_secs()
                    )));
                    tokio::select! {
                        () = cancel.cancelled() => return Err(LlmError::Cancelled),
                        () = tokio::time::sleep(wait) => {}
                    }
                }
                other => return other,
            }
        }
    }

    async fn stream_once(
        &self,
        model: &str,
        body: &Value,
        on_event: &mut (dyn FnMut(GeminiEvent) + Send),
    ) -> Result<GeminiTurn, LlmError> {
        let key = self.key()?;
        let resp = self
            .http
            .post(format!(
                "{BASE}/models/{model}:streamGenerateContent?alt=sse"
            ))
            .header("x-goog-api-key", key.expose_secret())
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            return Err(api_error(status, &text));
        }
        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::default();
        let mut turn = GeminiTurn::default();
        loop {
            let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next())
                .await
                .map_err(|_| LlmError::Stream("no data from Gemini for 90 s".into()))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| LlmError::Network(e.to_string()))?;
            for ev in parser.push(&chunk) {
                let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
                    continue;
                };
                if let Some(err) = v.get("error") {
                    return Err(api_error(
                        u16::try_from(err["code"].as_u64().unwrap_or(500)).unwrap_or(500),
                        &v.to_string(),
                    ));
                }
                apply_chunk(&mut turn, &v, on_event);
            }
        }
        if turn.parts.is_empty() && turn.finish_reason.is_none() {
            return Err(LlmError::Stream("Gemini returned an empty response".into()));
        }
        Ok(turn)
    }
}

/// Adds one streamed chunk to the turn. Parts are appended as received (never merged),
/// so thought signatures stay on exactly the part that carried them.
fn apply_chunk(turn: &mut GeminiTurn, v: &Value, on_event: &mut (dyn FnMut(GeminiEvent) + Send)) {
    if let Some(c) = v["candidates"].get(0) {
        for part in c["content"]["parts"].as_array().into_iter().flatten() {
            if part.get("thought") != Some(&Value::Bool(true))
                && let Some(t) = part["text"].as_str()
                && !t.is_empty()
            {
                on_event(GeminiEvent::Text(t.to_owned()));
            }
            turn.parts.push(part.clone());
        }
        if let Some(r) = c["finishReason"].as_str() {
            turn.finish_reason = Some(r.to_owned());
        }
    }
    let u = &v["usageMetadata"];
    if let Some(n) = u["promptTokenCount"].as_u64() {
        turn.prompt_tokens = n;
    }
    if let Some(n) = u["candidatesTokenCount"].as_u64() {
        turn.output_tokens = n + u["thoughtsTokenCount"].as_u64().unwrap_or(0);
    }
    if let Some(n) = u["cachedContentTokenCount"].as_u64() {
        turn.cached_tokens = n;
    }
}

/// A Gemini function declaration from a JSON schema, keeping only keywords Gemini's
/// schema subset supports.
pub fn function_declaration(name: &str, description: &str, schema: &Value) -> Value {
    fn clean(v: &Value) -> Value {
        match v {
            Value::Object(o) => Value::Object(
                o.iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "additionalProperties" | "$schema" | "annotations"
                        )
                    })
                    .map(|(k, v)| (k.clone(), clean(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(clean).collect()),
            other => other.clone(),
        }
    }
    let params = clean(schema);
    let empty = params["properties"]
        .as_object()
        .is_none_or(serde_json::Map::is_empty);
    if empty {
        json!({"name": name, "description": description})
    } else {
        json!({"name": name, "description": description, "parameters": params})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_keep_parts_and_signatures() {
        let mut turn = GeminiTurn::default();
        let mut texts = Vec::new();
        let mut on = |e: GeminiEvent| {
            if let GeminiEvent::Text(t) = e {
                texts.push(t);
            }
        };
        apply_chunk(
            &mut turn,
            &json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Opening "}]}}]}),
            &mut on,
        );
        apply_chunk(
            &mut turn,
            &json!({"candidates": [{"content": {"parts": [{"text": "Notepad"}]}}]}),
            &mut on,
        );
        apply_chunk(
            &mut turn,
            &json!({"candidates": [{"content": {"parts": [{"functionCall": {"name": "key", "args": {"text": "super"}, "id": "c1"}, "thoughtSignature": "sig"}]},
                     "finishReason": "STOP"}],
                    "usageMetadata": {"promptTokenCount": 900, "candidatesTokenCount": 20, "thoughtsTokenCount": 5}}),
            &mut on,
        );
        assert_eq!(texts, vec!["Opening ", "Notepad"]);
        assert_eq!(turn.parts.len(), 3, "parts are never merged");
        assert_eq!(turn.parts[2]["thoughtSignature"], "sig");
        assert_eq!(turn.function_calls().len(), 1);
        assert_eq!(turn.text(), "Opening Notepad");
        assert_eq!((turn.prompt_tokens, turn.output_tokens), (900, 25));
    }

    #[test]
    fn declarations_drop_unsupported_keywords() {
        let schema = json!({"type": "object", "additionalProperties": false,
                            "properties": {"coordinate": {"type": "array", "items": {"type": "number"}}}, "required": ["coordinate"]});
        let d = function_declaration("left_click", "Click", &schema);
        assert!(d["parameters"].get("additionalProperties").is_none());
        assert_eq!(d["parameters"]["required"][0], "coordinate");
        let none = function_declaration(
            "screenshot",
            "Take one",
            &json!({"type": "object", "properties": {}}),
        );
        assert!(none.get("parameters").is_none());
    }

    #[test]
    fn model_filter() {
        assert!(is_chat_model("gemini-3.1-pro-preview"));
        assert!(is_chat_model("gemini-4-carbon"));
        assert!(!is_chat_model("gemini-embedding-001"));
        assert!(!is_chat_model("gemini-2.5-flash-image"));
        assert!(!is_chat_model("imagen-4"));
    }
}
