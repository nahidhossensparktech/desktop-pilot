//! The agent loop for the Gemini API engine: same executor, safety rules, approvals,
//! pause/stop and follow-up screenshots as the Claude loop, with Gemini's function
//! calling. Model turns are replayed exactly as received (thought signatures).

use dp_llm::gemini::{GeminiClient, GeminiEvent, function_declaration};
use dp_llm::{LlmError, Usage};
use dp_platform::Platform;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::{
    AgentEvent, NOT_EXECUTED, Run, STOPPED_BEFORE, TaskConfig, TaskOutcome, computer, prompt,
};
use crate::state::AgentState;

const MAX_IMAGES: usize = 10;
const KEEP_IMAGES: usize = 4;

/// Conversation kept across follow-up prompts.
#[derive(Debug, Default)]
pub struct GeminiSession {
    system: Option<String>,
    contents: Vec<Value>,
    pub total_usage: Usage,
    /// The user allowed mouse and keyboard control for this session.
    pub control_approved: bool,
}

impl GeminiSession {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
pub struct GeminiContext<'a> {
    pub platform: &'a Platform,
    pub client: &'a GeminiClient,
    pub events: &'a mpsc::UnboundedSender<AgentEvent>,
    pub cancel: &'a CancellationToken,
}

fn declarations() -> Value {
    let decls: Vec<Value> = super::tool_definitions()
        .iter()
        .map(|t| {
            function_declaration(
                t["name"].as_str().unwrap_or_default(),
                t["description"].as_str().unwrap_or_default(),
                &t["inputSchema"],
            )
        })
        .collect();
    json!([{"functionDeclarations": decls}])
}

/// Executor output (text or API-style blocks) → (text for the function response,
/// image parts to add after the responses).
fn split_content(content: &Value) -> (String, Vec<Value>) {
    match content {
        Value::String(s) => (s.clone(), Vec::new()),
        Value::Array(items) => {
            let mut text = Vec::new();
            let mut images = Vec::new();
            for item in items {
                match item["type"].as_str() {
                    Some("image") => images.push(json!({"inlineData": {
                        "mimeType": item["source"]["media_type"],
                        "data": item["source"]["data"],
                    }})),
                    Some("text") => text.push(item["text"].as_str().unwrap_or_default().to_owned()),
                    _ => {}
                }
            }
            let mut t = text.join(" ");
            if t.is_empty() && !images.is_empty() {
                t = "Screenshot attached below.".to_owned();
            }
            (t, images)
        }
        other => (other.to_string(), Vec::new()),
    }
}

fn function_response(call: &Value, result: Result<String, String>) -> Value {
    let fc = &call["functionCall"];
    let response = match result {
        Ok(text) => json!({"result": text}),
        Err(e) => json!({"error": e}),
    };
    let mut fr = json!({"name": fc["name"], "response": response});
    if let Some(id) = fc.get("id") {
        fr["id"] = id.clone();
    }
    json!({"functionResponse": fr})
}

fn count_images(contents: &[Value]) -> usize {
    contents
        .iter()
        .flat_map(|c| c["parts"].as_array().into_iter().flatten())
        .filter(|p| p.get("inlineData").is_some())
        .count()
}

/// Replaces all but the newest `keep` screenshots in *user* turns with a text note.
/// Model turns are never touched (they carry thought signatures).
fn trim_images(contents: &mut [Value], keep: usize) {
    let mut to_remove = count_images(contents).saturating_sub(keep);
    for c in contents.iter_mut().filter(|c| c["role"] == "user") {
        if let Some(parts) = c["parts"].as_array_mut() {
            for p in parts.iter_mut() {
                if to_remove > 0 && p.get("inlineData").is_some() {
                    *p = json!({"text": "[older screenshot removed to save space]"});
                    to_remove -= 1;
                }
            }
        }
    }
}

/// After a stop, answer every function call of the last model turn so the
/// conversation stays valid for a follow-up prompt.
fn close_open_calls(session: &mut GeminiSession, done: Vec<Value>, reason: &str) {
    let Some(last) = session.contents.last() else {
        return;
    };
    if last["role"] != "model" {
        return;
    }
    let calls: Vec<Value> = last["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p.get("functionCall").is_some())
        .cloned()
        .collect();
    let mut parts = done;
    for call in calls.iter().skip(parts.len()) {
        parts.push(function_response(call, Err(reason.to_owned())));
    }
    if !parts.is_empty() {
        session
            .contents
            .push(json!({"role": "user", "parts": parts}));
    }
}

/// Runs one prompt with Gemini.
pub async fn run_gemini_task(
    ctx: GeminiContext<'_>,
    session: &mut GeminiSession,
    prompt_text: &str,
    cfg: &TaskConfig,
    pause: &mut watch::Receiver<bool>,
) -> TaskOutcome {
    let GeminiContext {
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
    match session.contents.last_mut() {
        Some(last) if last["role"] == "user" => {
            if let Some(parts) = last["parts"].as_array_mut() {
                parts.push(json!({"text": prompt_text}));
            }
        }
        _ => session
            .contents
            .push(json!({"role": "user", "parts": [{"text": prompt_text}]})),
    }

    let tools = declarations();
    let mut first_action_done = false;
    for step in 1..=cfg.max_steps {
        if !run.wait_if_paused(pause).await {
            return TaskOutcome::Stopped("stopped by user".to_owned());
        }
        if count_images(&session.contents) > MAX_IMAGES {
            trim_images(&mut session.contents, KEEP_IMAGES);
        }
        run.state(AgentState::Thinking);
        run.emit(AgentEvent::TurnStart { step });

        let body = json!({
            "systemInstruction": {"parts": [{"text": session.system.clone().unwrap_or_default()}]},
            "contents": session.contents,
            "tools": tools,
            "toolConfig": {"functionCallingConfig": {"mode": "AUTO"}},
        });
        let ev = events.clone();
        let mut on_event = move |e: GeminiEvent| {
            let _ = ev.send(match e {
                GeminiEvent::Text(t) => AgentEvent::Text(t),
                GeminiEvent::Retrying(r) => AgentEvent::Retrying(r),
            });
        };
        let turn = match client
            .stream(&cfg.model.id, &body, cancel, &mut on_event)
            .await
        {
            Ok(t) => t,
            Err(LlmError::Cancelled) => return TaskOutcome::Stopped("stopped by user".to_owned()),
            Err(e) => return TaskOutcome::Failed(e.to_string()),
        };
        session.total_usage.input_tokens += turn.prompt_tokens.saturating_sub(turn.cached_tokens);
        session.total_usage.cache_read_input_tokens += turn.cached_tokens;
        session.total_usage.output_tokens += turn.output_tokens;
        run.emit(AgentEvent::Usage(session.total_usage));
        session
            .contents
            .push(json!({"role": "model", "parts": turn.parts.clone()}));

        let calls: Vec<Value> = turn.function_calls().into_iter().cloned().collect();
        if calls.is_empty() {
            return match turn.finish_reason.as_deref() {
                None | Some("STOP") => TaskOutcome::Completed(turn.text()),
                Some("MAX_TOKENS") => {
                    TaskOutcome::Failed("Gemini's answer was cut off (too long)".to_owned())
                }
                Some(other) => {
                    let text = turn.text();
                    if text.is_empty() {
                        TaskOutcome::Failed(format!("Gemini stopped ({other})"))
                    } else {
                        TaskOutcome::Completed(text)
                    }
                }
            };
        }

        run.state(AgentState::Acting);
        let mut responses: Vec<Value> = Vec::new();
        let mut images: Vec<Value> = Vec::new();
        let mut failed = false;
        let mut last_changed = false;
        for call in &calls {
            let name = call["functionCall"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let input = call["functionCall"]["args"].clone();
            let input = if input.is_null() { json!({}) } else { input };
            if failed {
                responses.push(function_response(call, Err(NOT_EXECUTED.to_owned())));
                continue;
            }
            if cancel.is_cancelled() || !run.wait_if_paused(pause).await {
                close_open_calls(session, responses, STOPPED_BEFORE);
                return TaskOutcome::Stopped("stopped by user".to_owned());
            }
            if !dp_llm::COMPUTER_TOOLSET_MEMBERS.contains(&name.as_str()) {
                responses.push(function_response(
                    call,
                    Err(format!("unknown tool `{name}`")),
                ));
                failed = true;
                continue;
            }
            let description = computer::describe(&name, &input);
            if let Some(question) = run.approval_question(&name, &input, &description).await {
                match run.confirm(&question, computer::target_point(&input)).await {
                    Ok(()) => {
                        run.task_approved = true;
                        session.control_approved = true;
                    }
                    Err(reason) => {
                        if cancel.is_cancelled() {
                            close_open_calls(session, responses, STOPPED_BEFORE);
                            return TaskOutcome::Stopped("stopped by user".to_owned());
                        }
                        run.emit(AgentEvent::ActionFailed(format!("{description}: declined")));
                        responses.push(function_response(call, Err(reason)));
                        failed = true;
                        continue;
                    }
                }
            }
            run.emit(AgentEvent::Action(description.clone()));
            if !first_action_done {
                first_action_done = true;
                tokio::time::sleep(super::FIRST_ACTION_PAUSE).await;
            }
            match run.execute(&name, &input).await {
                Ok(content) => {
                    let (text, imgs) = split_content(&content);
                    responses.push(function_response(call, Ok(text)));
                    images.extend(imgs);
                    last_changed = !computer::is_read_only(&name);
                }
                Err(e) => {
                    let _ = platform.input.release_all();
                    if cancel.is_cancelled() {
                        close_open_calls(session, responses, STOPPED_BEFORE);
                        return TaskOutcome::Stopped("stopped by user".to_owned());
                    }
                    run.emit(AgentEvent::ActionFailed(format!("{description}: {e}")));
                    responses.push(function_response(call, Err(e)));
                    failed = true;
                }
            }
            if !computer::is_read_only(&name) && !cfg.action_delay.is_zero() {
                tokio::select! {
                    () = cancel.cancelled() => {}
                    () = tokio::time::sleep(cfg.action_delay) => {}
                }
            }
        }
        // Save a turn: show the screen after actions that changed something.
        if cfg.screenshot_after
            && !failed
            && last_changed
            && images.is_empty()
            && let Ok(content) = run.with_followup_screenshot(json!("OK")).await
        {
            images.extend(split_content(&content).1);
        }
        let mut parts = responses;
        if !images.is_empty() {
            parts.push(json!({"text": "Screenshot of the screen now:"}));
            parts.extend(images);
        }
        session
            .contents
            .push(json!({"role": "user", "parts": parts}));
    }
    TaskOutcome::Stopped(format!("reached the limit of {} steps", cfg.max_steps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_responses_carry_id_and_result_or_error() {
        let call = json!({"functionCall": {"name": "left_click", "args": {}, "id": "c7"}, "thoughtSignature": "s"});
        let ok = function_response(&call, Ok("OK".into()));
        assert_eq!(ok["functionResponse"]["id"], "c7");
        assert_eq!(ok["functionResponse"]["response"]["result"], "OK");
        let err = function_response(&call, Err("no".into()));
        assert_eq!(err["functionResponse"]["response"]["error"], "no");
    }

    #[test]
    fn images_become_inline_data() {
        let content = json!([{"type": "text", "text": "OK."},
                             {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "QUJD"}}]);
        let (text, images) = split_content(&content);
        assert_eq!(text, "OK.");
        assert_eq!(images[0]["inlineData"]["mimeType"], "image/png");
        assert_eq!(images[0]["inlineData"]["data"], "QUJD");
    }

    #[test]
    fn trimming_touches_only_user_turns() {
        let img = json!({"inlineData": {"mimeType": "image/png", "data": "x"}});
        let mut contents = vec![
            json!({"role": "user", "parts": [{"text": "go"}, img.clone()]}),
            json!({"role": "model", "parts": [{"functionCall": {"name": "screenshot"}, "thoughtSignature": "s"}]}),
            json!({"role": "user", "parts": [img.clone()]}),
        ];
        trim_images(&mut contents, 1);
        assert_eq!(count_images(&contents), 1);
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "s");
        assert!(contents[2]["parts"][0].get("inlineData").is_some());
    }

    #[test]
    fn declarations_cover_all_tools() {
        let d = declarations();
        assert_eq!(d[0]["functionDeclarations"].as_array().unwrap().len(), 17);
    }
}
