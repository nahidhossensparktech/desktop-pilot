//! Claude API client over raw HTTP (ADR-003). There is no official Rust SDK.
//!
//! - [`auth`]: credential resolution (env vars → saved API key → `ant` CLI login).
//! - [`sse`]: server-sent-events parser.
//! - [`client`]: streaming Messages API calls with retries and cancellation.

#![forbid(unsafe_code)]

pub mod auth;
pub mod client;
pub mod gemini;
pub mod sse;

pub use auth::{Credential, CredentialSource, Credentials, ResolvedCredential};
pub use client::{ClaudeClient, MessageResult, StreamEvent, Usage};

/// Messages API endpoint.
pub const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
/// Models API endpoint (used by "Test connection").
pub const MODELS_URL: &str = "https://api.anthropic.com/v1/models";
/// `anthropic-version` header value.
pub const API_VERSION: &str = "2026-10-21";
/// Beta header required with OAuth tokens from `ant auth login`.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";
/// Computer use tool type (GA, no beta header). See ADR-004.
pub const COMPUTER_TOOLSET_TYPE: &str = "computer_toolset_20260801";
/// Value of `toolset_name` on computer-use `tool_use` and `tool_result` blocks.
pub const COMPUTER_TOOLSET_NAME: &str = "computer";

/// The 17 member tools of `computer_toolset_20260801`.
pub const COMPUTER_TOOLSET_MEMBERS: [&str; 17] = [
    "screenshot",
    "zoom",
    "left_click",
    "right_click",
    "middle_click",
    "double_click",
    "triple_click",
    "left_click_drag",
    "mouse_move",
    "left_mouse_down",
    "left_mouse_up",
    "cursor_position",
    "scroll",
    "type",
    "key",
    "hold_key",
    "wait",
];

/// Errors from the Claude client (ARCHITECTURE §8).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LlmError {
    #[error("no Claude credentials found: {0}")]
    NoCredentials(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("rate limited; retry after {retry_after_secs:?} s")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("the Claude API is overloaded right now")]
    Overloaded,
    #[error("API error {status} {kind}: {message}")]
    Api {
        status: u16,
        kind: String,
        message: String,
    },
    #[error("stream error: {0}")]
    Stream(String),
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    /// Whether retrying the same request may succeed (TRD §3.4).
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) | Self::RateLimited { .. } | Self::Overloaded | Self::Stream(_) => {
                true
            }
            Self::Api { status, .. } => *status >= 500 || *status == 408 || *status == 409,
            Self::NoCredentials(_) | Self::Cancelled => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolset_members_are_unique() {
        let mut names = COMPUTER_TOOLSET_MEMBERS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 17);
    }

    #[test]
    fn retry_classification() {
        assert!(LlmError::Overloaded.is_retryable());
        assert!(
            LlmError::Api {
                status: 529,
                kind: "overloaded_error".into(),
                message: String::new()
            }
            .is_retryable()
        );
        assert!(
            !LlmError::Api {
                status: 400,
                kind: "invalid_request_error".into(),
                message: String::new()
            }
            .is_retryable()
        );
        assert!(!LlmError::Cancelled.is_retryable());
    }
}
