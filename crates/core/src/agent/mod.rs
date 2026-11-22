//! The agent loop (ARCHITECTURE §4–5): prompt + screenshot → Claude → computer actions →
//! results → repeat until done, stopped, or out of steps.

mod computer;
mod gemini;
mod prompt;
mod tools;

pub use gemini::{GeminiContext, GeminiSession, run_gemini_task};
pub use tools::tool_definitions;

use std::sync::Arc;
use std::time::Duration;

use dp_config::{ConfirmMode, ModelEntry};
use dp_llm::{ClaudeClient, LlmError, StreamEvent, Usage};
use dp_platform::Platform;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::screen::{ScreenMap, Shot};
use crate::state::AgentState;

const BETA_BLOCK_BINDING: &str = "thinking-binding-controls-2026-08-01";
const BETA_DISPLAY_UPDATES: &str = "thinking-display-updates-2026-08-18";
/// Keep at most this many screenshots in the history; trim older ones to this many
/// recent ones (ADR-006 amendment: trimming is a history edit, so requests set
/// `prefix_mismatch_behavior: drop_block` and the API drops the stale thinking blocks).
const MAX_IMAGES: usize = 12;
const KEEP_IMAGES: usize = 5;
const NOT_EXECUTED: &str = "Not executed: an earlier computer action in this turn failed.";
const STOPPED_BEFORE: &str = "Not executed: the user stopped the task.";

/// Settings for one task.
#[derive(Clone, Debug)]
pub struct TaskConfig {
    pub model: ModelEntry,
    pub confirm: ConfirmMode,
    pub max_steps: u32,
    pub action_delay: Duration,
    pub screenshot_long_edge: u32,
    /// Glide the mouse to targets over this long (zero = jump).
    pub mouse_motion: Duration,
    /// Attach a fresh screenshot after actions that change something (fewer turns).
    pub screenshot_after: bool,
}

/// How long the screen gets to settle before an automatic follow-up screenshot.
const SETTLE: Duration = Duration::from_millis(450);
/// Pause before the first action of a task, so the UI can shrink out of the way.
pub const FIRST_ACTION_PAUSE: Duration = Duration::from_millis(450);

/// Options for a [`ComputerSession`].
#[derive(Clone, Copy, Debug)]
pub struct ComputerOptions {
    pub screenshot_long_edge: u32,
    pub mouse_motion: Duration,
    pub screenshot_after: bool,
}

/// A screenshot for the UI, with an optional marker in screenshot coordinates.
#[derive(Clone, Debug)]
pub struct Preview {
    pub rgba: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub marker: Option<(i64, i64)>,
}

#[derive(Debug)]
pub enum Decision {
    Approve,
    Reject(String),
}

#[derive(Debug)]
pub struct ConfirmRequest {
    pub description: String,
    pub preview: Option<Preview>,
    pub reply: oneshot::Sender<Decision>,
}

#[derive(Debug)]
pub enum AgentEvent {
    State(AgentState),
    /// A new model response starts.
    TurnStart {
        step: u32,
    },
    Text(String),
    Thinking(String),
    Action(String),
    ActionFailed(String),
    Screenshot(Preview),
    Usage(Usage),
    Retrying(String),
    Confirm(ConfirmRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskOutcome {
    Completed(String),
    Stopped(String),
    Failed(String),
}

/// Conversation state kept across follow-up prompts.
#[derive(Debug, Default)]
pub struct Session {
    system: Option<String>,
    messages: Vec<Value>,
    pub total_usage: Usage,
    /// "Risky only" mode: the user allowed mouse and keyboard control for this
    /// session, so follow-up tasks do not ask again (security steps still do).
    pub control_approved: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Runs computer-use actions outside the agent loop, e.g. for the MCP server mode
/// where Claude Code is the client. Keeps the latest screenshot so coordinates map
/// back to the screen exactly as in the agent.
#[derive(Debug)]
pub struct ComputerSession {
    platform: Platform,
    cfg: TaskConfig,
    events: mpsc::UnboundedSender<AgentEvent>,
    map: Option<ScreenMap>,
    frame: Option<dp_platform::Frame>,
    last_shot: Option<Shot>,
}

impl ComputerSession {
    pub fn new(platform: Platform, options: ComputerOptions) -> Self {
        // Nobody listens to preview events here; sends are simply dropped.
        let (events, _) = mpsc::unbounded_channel();
        Self {
            platform,
            cfg: TaskConfig {
                model: dp_config::default_models().remove(0),
                confirm: ConfirmMode::Automatic,
                max_steps: 1,
                action_delay: Duration::ZERO,
                screenshot_long_edge: options.screenshot_long_edge,
                mouse_motion: options.mouse_motion,
                screenshot_after: options.screenshot_after,
            },
            events,
            map: None,
            frame: None,
            last_shot: None,
        }
    }

    /// Applies new options (e.g. after the user changed Settings).
    pub fn set_options(&mut self, options: ComputerOptions) {
        self.cfg.screenshot_long_edge = options.screenshot_long_edge;
        self.cfg.mouse_motion = options.mouse_motion;
        self.cfg.screenshot_after = options.screenshot_after;
    }

    /// Sends screenshot previews to `events` (for a UI) instead of dropping them.
    pub fn set_events(&mut self, events: mpsc::UnboundedSender<AgentEvent>) {
        self.events = events;
    }

    /// Size of the last screenshot (the coordinate space for actions).
    pub fn screenshot_size(&self) -> Option<(u32, u32)> {
        self.map.as_ref().map(|m| (m.model_w, m.model_h))
    }

    /// The last screenshot as a UI preview, with an optional marker.
    pub fn preview(&self, marker: Option<(i64, i64)>) -> Option<Preview> {
        self.last_shot.as_ref().map(|s| Preview {
            rgba: s.rgba.clone(),
            width: s.width,
            height: s.height,
            marker,
        })
    }

    /// Plain-language description of an action.
    pub fn describe(name: &str, input: &Value) -> String {
        computer::describe(name, input)
    }

    /// Whether an action only looks (screenshot, zoom, …) and changes nothing.
    pub fn is_read_only(name: &str) -> bool {
        computer::is_read_only(name)
    }

    /// The model-space point an action targets (for the approval preview).
    pub fn target_point(input: &Value) -> Option<(i64, i64)> {
        computer::target_point(input)
    }

    /// Runs one computer-use action (same names and inputs as the computer toolset).
    /// Returns `"OK"`, a text value, or an array of content blocks (API-style images).
    /// With `screenshot_after`, actions that change something also return a fresh
    /// screenshot, so the client needs no extra turn to look.
    pub async fn execute(
        &mut self,
        name: &str,
        input: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value, String> {
        let mut run = Run {
            platform: &self.platform,
            cfg: &self.cfg,
            events: &self.events,
            cancel,
            map: self.map.take(),
            frame: self.frame.take(),
            last_shot: self.last_shot.take(),
            task_approved: true,
        };
        let mut result = run.execute(name, input).await;
        if result.is_ok() && self.cfg.screenshot_after && !computer::is_read_only(name) {
            result = run
                .with_followup_screenshot(result.unwrap_or_default())
                .await;
        }
        if result.is_err() {
            let _ = self.platform.input.release_all();
        }
        self.map = run.map;
        self.frame = run.frame;
        self.last_shot = run.last_shot;
        result
    }
}

/// State of one running task.
struct Run<'a> {
    platform: &'a Platform,
    cfg: &'a TaskConfig,
    events: &'a mpsc::UnboundedSender<AgentEvent>,
    cancel: &'a CancellationToken,
    map: Option<ScreenMap>,
    frame: Option<dp_platform::Frame>,
    last_shot: Option<Shot>,
    /// In "risky only" mode: the user allowed control (for this session).
    task_approved: bool,
}

impl Run<'_> {
    /// Adds a fresh screenshot (after a short settle) to an action's result.
    async fn with_followup_screenshot(&mut self, result: Value) -> Result<Value, String> {
        tokio::select! {
            () = self.cancel.cancelled() => return Err(STOPPED_BEFORE.to_owned()),
            () = tokio::time::sleep(SETTLE) => {}
        }
        let shot = self.capture().await?;
        let text = match result {
            Value::String(s) => s,
            other => other.to_string(),
        };
        Ok(json!([
            {"type": "text", "text": format!("{text}. Screenshot after the action:")},
            shot.image_block(),
        ]))
    }

    /// Whether to ask the user before this action, and what to ask.
    async fn approval_question(
        &self,
        name: &str,
        input: &Value,
        description: &str,
    ) -> Option<String> {
        if computer::is_read_only(name) {
            return None;
        }
        match self.cfg.confirm {
            ConfirmMode::Automatic => None,
            ConfirmMode::EveryAction => Some(description.to_owned()),
            ConfirmMode::RiskyOnly => {
                if !self.task_approved {
                    return Some(format!(
                        "Allow Desktop Pilot to use your mouse and keyboard for this session? \
                         (It will only ask again for security-sensitive steps, or after New session.) \
                         First step: {description}"
                    ));
                }
                let title = self
                    .platform
                    .windows
                    .focused()
                    .await
                    .ok()
                    .flatten()
                    .map(|w| w.title)
                    .unwrap_or_default();
                crate::risk::assess(name, input, &title)
                    .map(|why| format!("{description}. Security check: this {why}."))
            }
        }
    }

    fn emit(&self, e: AgentEvent) {
        let _ = self.events.send(e);
    }

    fn state(&self, s: AgentState) {
        self.emit(AgentEvent::State(s));
    }

    /// Waits while paused. Returns false if stopped meanwhile.
    async fn wait_if_paused(&self, pause: &mut watch::Receiver<bool>) -> bool {
        if !*pause.borrow() {
            return !self.cancel.is_cancelled();
        }
        self.state(AgentState::Paused);
        while *pause.borrow() {
            tokio::select! {
                () = self.cancel.cancelled() => return false,
                r = pause.changed() => if r.is_err() { break },
            }
        }
        !self.cancel.is_cancelled()
    }

    /// Asks the user before a mutating action. Restores focus to the target window
    /// afterwards, because clicking the dialog moved focus to Desktop Pilot.
    async fn confirm(&self, description: &str, marker: Option<(i64, i64)>) -> Result<(), String> {
        let previous = self.platform.windows.focused().await.ok().flatten();
        let (tx, rx) = oneshot::channel();
        let preview = self.last_shot.as_ref().map(|s| Preview {
            rgba: s.rgba.clone(),
            width: s.width,
            height: s.height,
            marker,
        });
        self.state(AgentState::AwaitingConfirmation);
        self.emit(AgentEvent::Confirm(ConfirmRequest {
            description: description.to_owned(),
            preview,
            reply: tx,
        }));
        let decision = tokio::select! {
            () = self.cancel.cancelled() => return Err(STOPPED_BEFORE.to_owned()),
            d = rx => d.unwrap_or(Decision::Reject(String::new())),
        };
        self.state(AgentState::Acting);
        match decision {
            Decision::Approve => {
                if let Some(w) = previous
                    && w.pid != Some(std::process::id())
                {
                    let _ = self.platform.windows.focus(&w.id).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Ok(())
            }
            Decision::Reject(note) => {
                let note = note.trim();
                Err(if note.is_empty() {
                    "The user declined this action. Do not retry it; choose another way or ask the user.".to_owned()
                } else {
                    format!("The user declined this action and said: \"{note}\". Do not retry it.")
                })
            }
        }
    }
}

fn count_images(messages: &[Value]) -> usize {
    fn count(v: &Value) -> usize {
        match v {
            Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("image") => 1,
            Value::Object(o) => o.values().map(count).sum(),
            Value::Array(a) => a.iter().map(count).sum(),
            _ => 0,
        }
    }
    messages.iter().map(count).sum()
}

/// Replaces all but the newest `keep` images with a short text note.
fn trim_images(messages: &mut [Value], keep: usize) {
    let total = count_images(messages);
    let mut to_remove = total.saturating_sub(keep);
    fn walk(v: &mut Value, to_remove: &mut usize) {
        if *to_remove == 0 {
            return;
        }
        match v {
            Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("image") => {
                *v = json!({"type": "text", "text": "[older screenshot removed to save space]"});
                *to_remove -= 1;
            }
            Value::Object(o) => o.values_mut().for_each(|c| walk(c, to_remove)),
            Value::Array(a) => a.iter_mut().for_each(|c| walk(c, to_remove)),
            _ => {}
        }
    }
    for m in messages.iter_mut() {
        walk(m, &mut to_remove);
    }
}

fn build_request(cfg: &TaskConfig, session: &Session) -> (Value, Vec<&'static str>) {
    let mut thinking = json!({
        "type": "adaptive",
        "block_binding": {"prefix_mismatch_behavior": "drop_block"},
    });
    let mut betas = vec![BETA_BLOCK_BINDING];
    if let Some(display) = &cfg.model.thinking_display {
        thinking["display"] = json!(display);
        if display == "updates" {
            betas.push(BETA_DISPLAY_UPDATES);
        }
    }
    let body = json!({
        "model": cfg.model.id,
        "max_tokens": cfg.model.max_tokens,
        "stream": true,
        "system": session.system.clone().unwrap_or_default(),
        "tools": [{"type": dp_llm::COMPUTER_TOOLSET_TYPE}],
        "thinking": thinking,
        "output_config": {"effort": cfg.model.effort},
        "cache_control": {"type": "ephemeral"},
        "messages": session.messages,
    });
    (body, betas)
}

/// A minimal request with exactly the agent's settings (tools, thinking, betas,
/// caching) but a trivial text prompt. Used by `--test-connection` to catch API
/// contract problems cheaply before a real task.
pub fn probe_request(model: &ModelEntry) -> (Value, Vec<&'static str>) {
    let cfg = TaskConfig {
        model: ModelEntry {
            max_tokens: 2_000,
            ..model.clone()
        },
        confirm: ConfirmMode::EveryAction,
        max_steps: 1,
        action_delay: Duration::ZERO,
        screenshot_long_edge: 1280,
        mouse_motion: Duration::ZERO,
        screenshot_after: false,
    };
    let session = Session {
        system: Some("You are a connectivity check. Do not use any tools.".to_owned()),
        messages: vec![json!({"role": "user", "content": "Reply with exactly: OK"})],
        total_usage: Usage::default(),
        control_approved: false,
    };
    build_request(&cfg, &session)
}

fn final_text(content: &[Value]) -> String {
    content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn tool_result(tool_use: &Value, content: Value, is_error: bool) -> Value {
    let mut r = json!({
        "type": "tool_result",
        "tool_use_id": tool_use["id"],
        "content": content,
    });
    if let Some(ts) = tool_use.get("toolset_name") {
        r["toolset_name"] = ts.clone();
    }
    if is_error {
        r["is_error"] = json!(true);
    }
    r
}

/// Appends results for every unanswered `tool_use` of the last assistant message, so
/// the history stays valid for a follow-up prompt.
fn close_open_tool_uses(session: &mut Session, done: &[Value], reason: &str) {
    let Some(last) = session.messages.last() else {
        return;
    };
    if last["role"] != "assistant" {
        return;
    }
    let answered: Vec<&Value> = done.iter().map(|r| &r["tool_use_id"]).collect();
    let mut results = done.to_vec();
    if let Some(content) = last["content"].as_array() {
        for block in content.iter().filter(|b| b["type"] == "tool_use") {
            if !answered.contains(&&block["id"]) {
                results.push(tool_result(block, json!(reason), true));
            }
        }
    }
    if !results.is_empty() {
        session
            .messages
            .push(json!({"role": "user", "content": results}));
    }
}

/// What a task needs from its surroundings.
#[derive(Debug)]
pub struct TaskContext<'a> {
    pub platform: &'a Platform,
    pub client: &'a ClaudeClient,
    pub events: &'a mpsc::UnboundedSender<AgentEvent>,
    pub cancel: &'a CancellationToken,
}

/// Runs one prompt to completion. The session keeps the conversation for follow-ups.
pub async fn run_task(
    ctx: TaskContext<'_>,
    session: &mut Session,
    prompt: &str,
    cfg: &TaskConfig,
    pause: &mut watch::Receiver<bool>,
) -> TaskOutcome {
    let TaskContext {
        platform,
        client,
        events,
        cancel,
    } = ctx;
    let mut run = Run {
        platform,
        cfg,
        events,
        cancel,
        map: None,
        frame: None,
        last_shot: None,
        task_approved: session.control_approved,
    };
    run.state(AgentState::Thinking);

    // No screenshot up front: a plain question is answered in chat without one, and for
    // a computer task Claude takes one itself. The system prompt only needs the size.
    if session.system.is_none() {
        let monitors = platform.screen.monitors().await.unwrap_or_default();
        let (w, h) = monitors
            .iter()
            .find(|m| m.is_primary)
            .or(monitors.first())
            .map(|m| {
                crate::screen::model_size(
                    m.size_physical.0,
                    m.size_physical.1,
                    cfg.screenshot_long_edge,
                )
            })
            .unwrap_or((1280, 720));
        session.system = Some(prompt::system_prompt(&platform.info, w, h));
    }
    let user_content = vec![json!({"type": "text", "text": prompt})];
    let mut first_action_done = false;
    // After a stopped task the history may end with a user message; extend it rather
    // than sending two user messages in a row.
    match session.messages.last_mut() {
        Some(last) if last["role"] == "user" => {
            if let Some(arr) = last["content"].as_array_mut() {
                arr.extend(user_content);
            }
        }
        _ => session
            .messages
            .push(json!({"role": "user", "content": user_content})),
    }

    for step in 1..=cfg.max_steps {
        if !run.wait_if_paused(pause).await {
            return TaskOutcome::Stopped("stopped by user".to_owned());
        }
        if count_images(&session.messages) > MAX_IMAGES {
            trim_images(&mut session.messages, KEEP_IMAGES);
        }
        run.state(AgentState::Thinking);
        run.emit(AgentEvent::TurnStart { step });

        let (body, betas) = build_request(cfg, session);
        let ev = events.clone();
        let mut on_event = move |e: StreamEvent| {
            let _ = ev.send(match e {
                StreamEvent::Text(t) => AgentEvent::Text(t),
                StreamEvent::Thinking(t) => AgentEvent::Thinking(t),
                StreamEvent::Retrying { reason, wait } => AgentEvent::Retrying(format!(
                    "{reason}; retrying in {} s",
                    wait.as_secs().max(1)
                )),
            });
        };
        let result = match client
            .stream_message(&body, &betas, cancel, &mut on_event)
            .await
        {
            Ok(r) => r,
            Err(LlmError::Cancelled) => return TaskOutcome::Stopped("stopped by user".to_owned()),
            Err(e) => return TaskOutcome::Failed(e.to_string()),
        };
        session.total_usage.input_tokens += result.usage.input_tokens;
        session.total_usage.output_tokens += result.usage.output_tokens;
        session.total_usage.cache_read_input_tokens += result.usage.cache_read_input_tokens;
        session.total_usage.cache_creation_input_tokens += result.usage.cache_creation_input_tokens;
        run.emit(AgentEvent::Usage(session.total_usage));

        session
            .messages
            .push(json!({"role": "assistant", "content": result.content}));
        match result.stop_reason.as_deref() {
            Some("tool_use") => {}
            Some("pause_turn") => continue,
            Some("end_turn" | "stop_sequence") | None => {
                return TaskOutcome::Completed(final_text(&result.content));
            }
            Some("refusal") => {
                let category = result
                    .stop_details
                    .as_ref()
                    .and_then(|d| d["category"].as_str())
                    .unwrap_or("unspecified");
                return TaskOutcome::Failed(format!(
                    "Claude declined this request (category: {category})"
                ));
            }
            Some("max_tokens") => {
                close_open_tool_uses(session, &[], "Not executed: the response was cut off.");
                return TaskOutcome::Failed(
                    "Claude's response was too long and was cut off".to_owned(),
                );
            }
            Some(other) => return TaskOutcome::Failed(format!("unexpected stop reason `{other}`")),
        }

        // Execute the tool calls in order (batch rule, TRD §3.1).
        run.state(AgentState::Acting);
        let tool_uses: Vec<Value> = result
            .content
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .cloned()
            .collect();
        let mut results: Vec<Value> = Vec::with_capacity(tool_uses.len());
        let mut failed = false;
        for tu in &tool_uses {
            if failed {
                results.push(tool_result(tu, json!(NOT_EXECUTED), true));
                continue;
            }
            if cancel.is_cancelled() || !run.wait_if_paused(pause).await {
                close_open_tool_uses(session, &results, STOPPED_BEFORE);
                return TaskOutcome::Stopped("stopped by user".to_owned());
            }
            run.state(AgentState::Acting);
            let name = tu["name"].as_str().unwrap_or_default();
            let input = &tu["input"];
            if tu.get("toolset_name").and_then(Value::as_str) != Some(dp_llm::COMPUTER_TOOLSET_NAME)
            {
                results.push(tool_result(
                    tu,
                    json!(format!("Unknown tool `{name}`")),
                    true,
                ));
                failed = true;
                continue;
            }
            let description = computer::describe(name, input);
            if let Some(question) = run.approval_question(name, input, &description).await {
                match run.confirm(&question, computer::target_point(input)).await {
                    Ok(()) => {
                        run.task_approved = true;
                        session.control_approved = true;
                    }
                    Err(reason) => {
                        if cancel.is_cancelled() {
                            close_open_tool_uses(session, &results, STOPPED_BEFORE);
                            return TaskOutcome::Stopped("stopped by user".to_owned());
                        }
                        run.emit(AgentEvent::ActionFailed(format!("{description}: declined")));
                        results.push(tool_result(tu, json!(reason), true));
                        failed = true;
                        continue;
                    }
                }
            }
            run.emit(AgentEvent::Action(description.clone()));
            if !first_action_done {
                // The UI switches to its small control bar on the first action; give it
                // a moment so it is out of the way before the first screenshot or click.
                first_action_done = true;
                tokio::time::sleep(FIRST_ACTION_PAUSE).await;
            }
            match run.execute(name, input).await {
                Ok(content) => results.push(tool_result(tu, content, false)),
                Err(e) => {
                    let _ = platform.input.release_all();
                    if cancel.is_cancelled() {
                        close_open_tool_uses(session, &results, STOPPED_BEFORE);
                        return TaskOutcome::Stopped("stopped by user".to_owned());
                    }
                    run.emit(AgentEvent::ActionFailed(format!("{description}: {e}")));
                    results.push(tool_result(tu, json!(e), true));
                    failed = true;
                }
            }
            if !computer::is_read_only(name) && !cfg.action_delay.is_zero() {
                tokio::select! {
                    () = cancel.cancelled() => {}
                    () = tokio::time::sleep(cfg.action_delay) => {}
                }
            }
        }
        // Save a turn: if the batch ended with a successful action that changed
        // something, attach a fresh screenshot to its result.
        let last_changed = tool_uses
            .last()
            .and_then(|tu| tu["name"].as_str())
            .is_some_and(|n| !computer::is_read_only(n));
        if cfg.screenshot_after
            && !failed
            && last_changed
            && let Some(last) = results.last_mut()
        {
            let content = last["content"].take();
            match run.with_followup_screenshot(content.clone()).await {
                Ok(with_shot) => last["content"] = with_shot,
                Err(_) => last["content"] = content,
            }
        }
        session
            .messages
            .push(json!({"role": "user", "content": results}));
    }
    TaskOutcome::Stopped(format!("reached the limit of {} steps", cfg.max_steps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TaskConfig {
        TaskConfig {
            model: dp_config::default_models().remove(0),
            confirm: ConfirmMode::EveryAction,
            max_steps: 10,
            action_delay: Duration::ZERO,
            screenshot_long_edge: 1280,
            mouse_motion: Duration::ZERO,
            screenshot_after: false,
        }
    }

    fn image() -> Value {
        json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "x"}})
    }

    #[test]
    fn request_has_toolset_effort_and_no_sampling_params() {
        let mut s = Session::new();
        s.system = Some("sys".into());
        s.messages.push(json!({"role": "user", "content": "hi"}));
        let (body, betas) = build_request(&cfg(), &s);
        assert_eq!(
            body["tools"],
            json!([{"type": "computer_toolset_20260801"}])
        );
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "updates");
        assert!(body.get("temperature").is_none() && body.get("tool_choice").is_none());
        assert!(betas.contains(&BETA_BLOCK_BINDING) && betas.contains(&BETA_DISPLAY_UPDATES));
    }

    #[test]
    fn haiku_request_has_no_display_beta() {
        let mut c = cfg();
        c.model = dp_config::default_models()
            .into_iter()
            .find(|m| m.id.contains("haiku"))
            .unwrap();
        let (body, betas) = build_request(&c, &Session::new());
        assert!(body["thinking"].get("display").is_none());
        assert!(!betas.contains(&BETA_DISPLAY_UPDATES));
    }

    #[test]
    fn trimming_keeps_newest_images() {
        let mut msgs = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "go"}, image()]}),
            json!({"role": "assistant", "content": [{"type": "tool_use", "id": "a"}]}),
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "a", "content": [image()]}]}),
            json!({"role": "assistant", "content": [{"type": "tool_use", "id": "b"}]}),
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "b", "content": [image()]}]}),
        ];
        assert_eq!(count_images(&msgs), 3);
        trim_images(&mut msgs, 1);
        assert_eq!(count_images(&msgs), 1);
        assert_eq!(msgs[0]["content"][1]["type"], "text");
        assert_eq!(msgs[4]["content"][0]["content"][0]["type"], "image");
    }

    #[test]
    fn tool_results_echo_toolset_name() {
        let tu = json!({"type": "tool_use", "id": "t1", "name": "left_click", "toolset_name": "computer"});
        let r = tool_result(&tu, json!("OK"), false);
        assert_eq!(r["toolset_name"], "computer");
        assert_eq!(r["tool_use_id"], "t1");
        assert!(r.get("is_error").is_none());
        assert_eq!(tool_result(&tu, json!("x"), true)["is_error"], true);
    }

    #[test]
    fn closing_open_tool_uses_answers_every_call() {
        let mut s = Session::new();
        s.messages.push(json!({"role": "user", "content": "go"}));
        s.messages.push(json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "a", "name": "left_click", "toolset_name": "computer"},
            {"type": "tool_use", "id": "b", "name": "type", "toolset_name": "computer"},
        ]}));
        let done = vec![tool_result(
            &s.messages[1]["content"][0],
            json!("OK"),
            false,
        )];
        close_open_tool_uses(&mut s, &done, STOPPED_BEFORE);
        let results = s.messages.last().unwrap()["content"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(results.len(), 2);
        assert_eq!(results[1]["tool_use_id"], "b");
        assert_eq!(results[1]["is_error"], true);
    }
}
