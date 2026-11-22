//! The custom-tool interface (ARCHITECTURE §9.1). Implementations live in `dp-tools`.

use async_trait::async_trait;
use dp_platform::Platform;
use serde_json::Value;

/// Risk level of one tool call (SECURITY §4.2). The classifier arrives in M4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    Hard,
}

/// What a tool call returns to Claude.
#[derive(Clone, Debug)]
pub struct ToolOutput {
    pub text: String,
    /// True when the text came from the screen, clipboard or other untrusted sources,
    /// so the agent wraps it in `<untrusted_content>` (SECURITY §4.6).
    pub untrusted: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub message: String,
    pub retryable: bool,
}

/// Everything a tool may use while running.
#[derive(Clone, Debug)]
pub struct ToolContext {
    pub platform: Platform,
}

#[async_trait]
pub trait Tool: Send + Sync + std::fmt::Debug {
    /// Unique name; must not collide with computer-toolset member names.
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Strict JSON schema (`additionalProperties: false`).
    fn input_schema(&self) -> Value;
    fn risk(&self, input: &Value) -> RiskLevel;
    async fn call(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput, ToolError>;
}
