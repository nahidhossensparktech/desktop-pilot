//! Training data: one JSON record per task, appended to `tasks-YYYY-MM-DD.jsonl` in
//! [`dp_config::training_dir`]. Text only (prompt, Claude's replies and thinking,
//! actions, result, token usage); screenshots are never saved. Local only.
//!
//! Secrets are masked before anything is written: words that look like a password,
//! API key, token or card number (`dp_core::risk::looks_secret`), and the whole text of
//! a "Type" action that looks like one.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dp_core::agent::{AgentEvent, TaskOutcome};
use dp_core::risk::looks_secret;
use dp_llm::Usage;
use serde_json::{Value, json};

const REDACTED: &str = "[REDACTED]";
/// Bumped when the record layout changes.
const FORMAT_VERSION: u32 = 1;

/// Collects one task's events while it runs.
#[derive(Debug)]
pub(crate) struct Recorder {
    started: Instant,
    started_unix_ms: u64,
    header: Value,
    events: Vec<Value>,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (UTC).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn file_name(unix_ms: u64) -> String {
    let (y, m, d) = civil_from_days(i64::try_from(unix_ms / 86_400_000).unwrap_or(0));
    format!("tasks-{y:04}-{m:02}-{d:02}.jsonl")
}

/// Replaces every whitespace-separated word that looks secret.
pub(crate) fn mask(text: &str) -> String {
    if !text.split_whitespace().any(looks_secret) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            out.push_str(if looks_secret(word) { REDACTED } else { word });
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Masks an action description; a `Type "…"` action whose text looks secret is hidden
/// completely (card numbers contain spaces, so a word check is not enough).
pub(crate) fn mask_action(description: &str) -> String {
    if let Some(inner) = description
        .strip_prefix("Type \"")
        .and_then(|r| r.strip_suffix('"'))
        && looks_secret(inner.trim_end_matches('…'))
    {
        return format!("Type \"{REDACTED}\"");
    }
    mask(description)
}

impl Recorder {
    pub(crate) fn start(prompt: &str, engine: &str, model: &str, os: &str) -> Self {
        let started_unix_ms = unix_ms();
        Self {
            started: Instant::now(),
            started_unix_ms,
            header: json!({
                "format_version": FORMAT_VERSION,
                "app_version": env!("CARGO_PKG_VERSION"),
                "started_unix_ms": started_unix_ms,
                "os": os,
                "engine": engine,
                "model": model,
                "prompt": mask(prompt),
                "input": "typed",
            }),
            events: Vec::new(),
        }
    }

    fn at_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Appends streamed text to the previous event of the same kind, or starts one.
    fn stream(&mut self, kind: &str, chunk: &str) {
        if let Some(last) = self.events.last_mut()
            && last["kind"] == kind
            && let Some(Value::String(text)) = last.get_mut("text")
        {
            text.push_str(chunk);
            return;
        }
        let at = self.at_ms();
        self.events
            .push(json!({ "at_ms": at, "kind": kind, "text": chunk }));
    }

    pub(crate) fn on_event(&mut self, event: &AgentEvent) {
        let at = self.at_ms();
        match event {
            AgentEvent::Text(t) => self.stream("claude_text", t),
            AgentEvent::Thinking(t) => self.stream("thinking", t),
            AgentEvent::Action(d) => self
                .events
                .push(json!({ "at_ms": at, "kind": "action", "text": mask_action(d) })),
            AgentEvent::ActionFailed(d) => self
                .events
                .push(json!({ "at_ms": at, "kind": "action_failed", "text": mask(d) })),
            AgentEvent::Confirm(req) => self.events.push(json!({
                "at_ms": at,
                "kind": "approval_asked",
                "text": mask_action(&req.description),
            })),
            AgentEvent::State(_)
            | AgentEvent::TurnStart { .. }
            | AgentEvent::Screenshot(_)
            | AgentEvent::Usage(_)
            | AgentEvent::Retrying(_) => {}
        }
    }

    /// Marks the task as started by a voice command.
    pub(crate) fn mark_voice(&mut self) {
        self.header["input"] = json!("voice");
    }

    /// Notes the user's answer to an approval request.
    pub(crate) fn on_decision(&mut self, approved: bool) {
        let at = self.at_ms();
        self.events
            .push(json!({ "at_ms": at, "kind": "approval_answer", "approved": approved }));
    }

    /// Builds the finished record.
    fn record(self, outcome: &TaskOutcome, usage: &Usage) -> Value {
        let (status, detail) = match outcome {
            TaskOutcome::Completed(s) => ("completed", s),
            TaskOutcome::Stopped(s) => ("stopped", s),
            TaskOutcome::Failed(s) => ("failed", s),
        };
        let actions = self.events.iter().filter(|e| e["kind"] == "action").count();
        let mut record = self.header;
        record["duration_ms"] =
            json!(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX));
        record["outcome"] = json!({ "status": status, "detail": mask(detail) });
        record["action_count"] = json!(actions);
        record["usage"] = json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        });
        record["events"] = Value::Array(self.events);
        record
    }

    /// Appends the record as one line to today's file in `dir`. Returns the file path.
    pub(crate) fn save(
        self,
        dir: &Path,
        outcome: &TaskOutcome,
        usage: &Usage,
    ) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let path = dir.join(file_name(self.started_unix_ms));
        let mut line =
            serde_json::to_string(&self.record(outcome, usage)).map_err(std::io::Error::other)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?
            .write_all(line.as_bytes())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(file_name(1_795_305_600_000), "tasks-2026-11-22.jsonl");
    }

    #[test]
    fn secrets_are_masked() {
        assert_eq!(
            mask("use key sk-ant-abc123 please"),
            "use key [REDACTED] please"
        );
        assert_eq!(mask("open notes"), "open notes");
        assert_eq!(
            mask_action("Type \"4111 1111 1111 1111\""),
            "Type \"[REDACTED]\""
        );
        assert_eq!(mask_action("Type \"hello\""), "Type \"hello\"");
        assert_eq!(mask_action("Left-click at (3, 4)"), "Left-click at (3, 4)");
    }

    #[test]
    fn record_merges_streamed_text_and_saves_one_line() {
        let mut r = Recorder::start("Open TextEdit", "Claude API", "claude-sonnet-5-5", "macos");
        r.on_event(&AgentEvent::Text("Sure, ".into()));
        r.on_event(&AgentEvent::Text("opening it.".into()));
        r.on_event(&AgentEvent::Action("Type \"sk-ant-secret\"".into()));
        r.on_decision(true);
        let dir = std::env::temp_dir().join(format!("dp-training-test-{}", std::process::id()));
        let path = r
            .save(
                &dir,
                &TaskOutcome::Completed("done".into()),
                &Usage::default(),
            )
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(text.lines().count(), 1);
        let v: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["prompt"], "Open TextEdit");
        assert_eq!(v["events"][0]["text"], "Sure, opening it.");
        assert_eq!(v["events"][1]["text"], "Type \"[REDACTED]\"");
        assert_eq!(v["events"][2]["approved"], true);
        assert_eq!(v["action_count"], 1);
        assert_eq!(v["outcome"]["status"], "completed");
        assert!(!text.contains("sk-ant-secret"));
    }
}
