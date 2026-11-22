//! The "Hey Mouse" voice assistant in the window, and running in the background.
//!
//! - The assistant (`dp-voice`) listens on its own thread and sends events here.
//! - A command goes through the normal chat: same agent, approvals, STOP, training
//!   data. While a task runs, only "Hey Mouse, stop" is accepted.
//! - Replies to voice commands are spoken (Settings ▸ Voice ▸ Talk back).
//! - Background mode: closing the window shrinks it to an invisible 1×1 window (a
//!   hidden window would stop the UI loop on macOS) and the tray / menu-bar icon
//!   brings it back.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dp_core::agent::TaskOutcome;
use dp_voice::{Assistant, AssistantConfig, Language, ModelSize, VoiceEvent, is_stop_command};
use eframe::egui;

use super::overlay::{Mode, View};
use super::{LineKind, PilotApp, UiEvent};
use crate::platform_init::{Tray, TrayAction};

/// How long the heard command stays in the overlay before the task takes over.
const HEARD_SHOWN: Duration = Duration::from_secs(6);
/// Longest spoken reply, in characters.
const MAX_SPOKEN: usize = 240;
/// Added to voice commands so Claude knows the text came from speech recognition.
const VOICE_HINT: &str = "\n\n(Spoken command from the voice assistant, transcribed by speech recognition: it may contain recognition errors, or Bangla written in Latin letters; interpret it by how it sounds. Your reply is read aloud: answer in one or two short sentences, in the language the user spoke.)";

/// Where the assistant is.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum VoiceStatus {
    Off,
    Downloading {
        done: u64,
        total: u64,
    },
    Starting,
    /// Listening for "Hey Mouse".
    Ready,
    /// Heard "Hey Mouse"; listening for the command.
    Listening,
    Understanding,
    Error(String),
}

/// Voice assistant state owned by the window.
#[derive(Debug)]
pub(super) struct Voice {
    assistant: Option<Assistant>,
    pub(super) status: VoiceStatus,
    /// Microphone level (f32 bits), written by the assistant thread.
    level: Arc<AtomicU32>,
    /// The last command, shown in the overlay for a moment.
    heard: Option<(String, Instant)>,
    /// The next prompt comes from voice, in this language.
    pub(super) next_prompt_lang: Option<&'static str>,
    /// The running task came from voice: speak its result in this language.
    task_lang: Option<&'static str>,
    /// Claude's text in the latest turn of a voice task (spoken at the end).
    reply: String,
    pub(super) tray: Option<Tray>,
    /// Shrunk to the invisible background window.
    pub(super) background: bool,
    /// Window position and size before going to the background.
    saved: Option<(egui::Pos2, egui::Vec2)>,
    /// A task's control bar was opened from the background: return there afterwards.
    pub(super) compact_from_background: bool,
    /// Quit was chosen: let the window close for real.
    allow_close: bool,
    told_background: bool,
    /// What the tray status line shows now (updated only on change).
    tray_status: String,
    /// Talk was asked for while the model was still loading.
    listen_when_ready: bool,
    /// The registered voice shortcut.
    shortcut: Option<dp_platform::HotkeyId>,
    /// A voice conversation is open: after each reply, listen for a follow-up.
    conversation: bool,
    /// Start listening for the follow-up once the reply has been spoken.
    follow_up_pending: bool,
    /// The current listening is for a follow-up (shown in the overlay).
    follow_up: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            assistant: None,
            status: VoiceStatus::Off,
            level: Arc::new(AtomicU32::new(0)),
            heard: None,
            next_prompt_lang: None,
            task_lang: None,
            reply: String::new(),
            tray: None,
            background: false,
            saved: None,
            compact_from_background: false,
            allow_close: false,
            told_background: false,
            tray_status: String::new(),
            listen_when_ready: false,
            shortcut: None,
            conversation: false,
            follow_up_pending: false,
            follow_up: false,
        }
    }
}

/// The first sentences of `text`, without markdown, short enough to speak.
pub(super) fn spoken_summary(text: &str) -> String {
    let plain: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '>' | '|'))
        .collect();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for sentence in plain.split_inclusive(['.', '!', '?', '।']) {
        if !out.is_empty() && out.chars().count() + sentence.chars().count() > MAX_SPOKEN {
            break;
        }
        out.push_str(sentence);
        if out.chars().count() >= MAX_SPOKEN / 2 {
            break;
        }
    }
    if out.chars().count() > MAX_SPOKEN {
        out = out.chars().take(MAX_SPOKEN).collect::<String>() + "…";
    }
    out.trim().to_owned()
}

fn fallback_reply(outcome: &TaskOutcome, bangla: bool) -> &'static str {
    match (outcome, bangla) {
        (TaskOutcome::Completed(_), false) => "Done.",
        (TaskOutcome::Completed(_), true) => "হয়ে গেছে।",
        (TaskOutcome::Stopped(_), false) => "Stopped.",
        (TaskOutcome::Stopped(_), true) => "থামানো হয়েছে।",
        (TaskOutcome::Failed(_), false) => "Sorry, that didn't work.",
        (TaskOutcome::Failed(_), true) => "দুঃখিত, কাজটা হয়নি।",
    }
}

impl PilotApp {
    // ---------- assistant lifecycle ----------

    fn voice_model(&self) -> ModelSize {
        ModelSize::from_id(&self.settings.voice_model).unwrap_or(ModelSize::Turbo)
    }

    /// "Hey Mouse" mode: the microphone stays open.
    pub(super) fn always_listening(&self) -> bool {
        self.settings.voice_mode == "wake"
    }

    /// The voice shortcut for people: "Option+Space" on macOS, "Alt+Space" elsewhere.
    pub(super) fn shortcut_label(&self) -> String {
        let mac = self.platform.info.os == dp_platform::OsKind::MacOs;
        self.settings
            .voice_shortcut
            .split('+')
            .map(
                |part| match (part.trim().to_ascii_lowercase().as_str(), mac) {
                    ("alt" | "option", true) => "Option".to_owned(),
                    ("alt" | "option", false) => "Alt".to_owned(),
                    ("ctrl" | "control", true) => "Control".to_owned(),
                    ("ctrl" | "control", false) => "Ctrl".to_owned(),
                    ("super" | "cmd" | "command" | "win", true) => "Cmd".to_owned(),
                    ("super" | "cmd" | "command" | "win", false) => "Win".to_owned(),
                    ("shift", _) => "Shift".to_owned(),
                    (_, _) => {
                        let mut c = part.trim().chars();
                        c.next()
                            .map(|f| f.to_uppercase().chain(c).collect())
                            .unwrap_or_default()
                    }
                },
            )
            .collect::<Vec<String>>()
            .join("+")
    }

    /// Called once at startup.
    pub(super) fn voice_init(&mut self) {
        let ctx = self.ctx.clone();
        let talk = format!("Talk now ({})", self.shortcut_label());
        self.voice.tray = Tray::create(
            self.settings.voice_enabled && self.always_listening(),
            &talk,
            move || ctx.request_repaint(),
        );
        self.register_voice_shortcut();
        if self.settings.voice_enabled {
            self.voice_enable();
        }
    }

    /// (Re)registers the global voice shortcut from the settings.
    pub(super) fn register_voice_shortcut(&mut self) {
        if let Some(id) = self.voice.shortcut.take() {
            let _ = self.platform.hotkeys.unregister(id);
        }
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let combo = dp_platform::HotkeyCombo(self.settings.voice_shortcut.clone());
        match self.platform.hotkeys.register(
            &combo,
            Box::new(move || {
                let _ = tx.send(UiEvent::VoiceShortcut);
                ctx.request_repaint();
            }),
        ) {
            Ok(id) => self.voice.shortcut = Some(id),
            Err(e) => self.push(
                LineKind::Info,
                format!(
                    "Voice shortcut {} not available: {e}",
                    self.shortcut_label()
                ),
            ),
        }
    }

    /// Switches between "Hey Mouse" (always listening) and tap / shortcut only.
    pub(super) fn voice_set_mode(&mut self, always: bool) {
        // Re-tick the menu in any case: macOS unticks an item clicked a second time.
        if let Some(t) = &self.voice.tray {
            t.set_voice(always);
        }
        if always == self.always_listening() && self.voice.assistant.is_some() {
            return;
        }
        self.settings.voice_mode = if always { "wake" } else { "tap" }.to_owned();
        self.voice_end_conversation();
        // Restart so the microphone opens or closes now.
        self.voice.assistant = None;
        self.voice.status = VoiceStatus::Off;
        self.voice_enable();
        let _ = self.settings.save();
    }

    /// Turns the assistant on: downloads the model first if needed.
    pub(super) fn voice_enable(&mut self) {
        self.settings.voice_enabled = true;
        if let Some(t) = &self.voice.tray {
            t.set_voice(self.always_listening());
        }
        if self.voice.assistant.is_some()
            || matches!(self.voice.status, VoiceStatus::Downloading { .. })
        {
            return;
        }
        let dir = match dp_config::models_dir() {
            Ok(d) => d,
            Err(e) => {
                self.voice.status = VoiceStatus::Error(e.to_string());
                return;
            }
        };
        let model = self.voice_model();
        if model.is_downloaded(&dir) {
            self.voice_start(model.path_in(&dir));
            return;
        }
        self.voice.status = VoiceStatus::Downloading {
            done: 0,
            total: model.bytes(),
        };
        self.push(
            LineKind::Info,
            format!(
                "Downloading the speech model ({} MB, once)…",
                model.bytes() / 1_000_000
            ),
        );
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let last = std::sync::Mutex::new(Instant::now());
            let progress = |done: u64, total: u64| {
                let mut l = super::lock(&last);
                if l.elapsed() >= Duration::from_millis(250) || done == total {
                    *l = Instant::now();
                    let _ = tx.send(UiEvent::VoiceProgress(done, total));
                    ctx.request_repaint();
                }
            };
            let result = dp_voice::download(model, &dir, progress)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(UiEvent::VoiceModel(result));
            ctx.request_repaint();
        });
    }

    fn voice_start(&mut self, model: std::path::PathBuf) {
        self.voice.status = VoiceStatus::Starting;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let level = self.voice.level.clone();
        self.voice.assistant = Some(Assistant::start(
            AssistantConfig {
                model,
                language: Language::from_id(&self.settings.voice_language),
                always_listening: self.always_listening(),
            },
            move |e| {
                // Levels are polled by the overlay, not sent: no repaint storm.
                if let VoiceEvent::Level(l) = e {
                    level.store(l.to_bits(), Ordering::Relaxed);
                    return;
                }
                let _ = tx.send(UiEvent::Voice(e));
                ctx.request_repaint();
            },
        ));
    }

    pub(super) fn voice_disable(&mut self) {
        self.settings.voice_enabled = false;
        self.voice.assistant = None;
        self.voice.status = VoiceStatus::Off;
        if let Some(t) = &self.voice.tray {
            t.set_voice(false);
        }
    }

    /// Talk now (orb, mic button, shortcut, tray): open the microphone for one command.
    /// If the assistant is still starting, it listens as soon as it is ready. Pressed
    /// again while listening, it closes the conversation.
    pub(super) fn voice_push_to_talk(&mut self) {
        if self.voice.status == VoiceStatus::Listening {
            self.voice_end_conversation();
            return;
        }
        match &self.voice.assistant {
            Some(a) if self.voice.status == VoiceStatus::Ready => a.listen_now(),
            Some(_) => self.voice.listen_when_ready = true,
            None => {
                self.voice.listen_when_ready = true;
                self.voice_enable();
                self.push(
                    LineKind::Info,
                    "Getting the voice assistant ready; it will listen in a moment.".to_owned(),
                );
            }
        }
    }

    /// Closes the voice conversation: stops listening (the microphone closes in
    /// shortcut mode) and does not listen for a follow-up.
    pub(super) fn voice_end_conversation(&mut self) {
        self.voice.conversation = false;
        self.voice.follow_up_pending = false;
        self.voice.follow_up = false;
        if let Some(a) = &self.voice.assistant {
            a.cancel();
        }
    }

    /// Whether the assistant is listening for a command right now.
    pub(super) fn voice_listening(&self) -> bool {
        self.voice.status == VoiceStatus::Listening
    }

    pub(super) fn voice_set_language(&mut self) {
        if let Some(a) = &self.voice.assistant {
            a.set_language(Language::from_id(&self.settings.voice_language));
        }
    }

    // ---------- events ----------

    pub(super) fn on_voice_progress(&mut self, done: u64, total: u64) {
        if matches!(self.voice.status, VoiceStatus::Downloading { .. }) {
            self.voice.status = VoiceStatus::Downloading { done, total };
        }
    }

    pub(super) fn on_voice_model(&mut self, result: Result<std::path::PathBuf, String>) {
        match result {
            Ok(path) if self.settings.voice_enabled => {
                self.push(LineKind::Good, "Speech model ready.".to_owned());
                self.voice_start(path);
            }
            Ok(_) => self.voice.status = VoiceStatus::Off,
            Err(e) => {
                self.push(LineKind::Bad, format!("Speech model: {e}"));
                self.voice.status = VoiceStatus::Error(e);
            }
        }
    }

    pub(super) fn on_voice_event(&mut self, event: VoiceEvent) {
        match event {
            VoiceEvent::Ready => {
                self.voice.status = VoiceStatus::Ready;
                let how = if self.always_listening() {
                    "say “Hey Mouse, …”".to_owned()
                } else {
                    format!(
                        "tap the orb or press {} (the microphone is off until then)",
                        self.shortcut_label()
                    )
                };
                self.push(
                    LineKind::Info,
                    format!("Voice assistant ready: {how}. Bangla or English."),
                );
                if std::mem::take(&mut self.voice.listen_when_ready)
                    && let Some(a) = &self.voice.assistant
                {
                    a.listen_now();
                }
            }
            VoiceEvent::Level(_) => {}
            VoiceEvent::WakeHeard => self.voice.status = VoiceStatus::Listening,
            VoiceEvent::Transcribing => self.voice.status = VoiceStatus::Understanding,
            // Nothing said (or nothing usable): the conversation ends.
            VoiceEvent::Idle | VoiceEvent::NotUnderstood => {
                self.voice.status = VoiceStatus::Ready;
                self.voice.conversation = false;
                self.voice.follow_up = false;
            }
            VoiceEvent::Heard { text, lang } => {
                self.voice.status = VoiceStatus::Ready;
                self.voice.follow_up = false;
                self.on_voice_command(text, lang);
            }
            VoiceEvent::Error(e) => {
                self.push(LineKind::Bad, format!("Voice assistant stopped: {e}"));
                self.voice.assistant = None;
                self.voice.status = VoiceStatus::Error(e);
            }
        }
    }

    fn on_voice_command(&mut self, text: String, lang: &'static str) {
        let bangla = lang == "bn";
        if self.running.is_some() {
            if is_stop_command(&text) {
                self.stop("voice command");
            } else {
                self.push(
                    LineKind::Info,
                    format!(
                        "Heard “{text}”, but a task is running. Say “Hey Mouse, stop” to cancel it."
                    ),
                );
            }
            return;
        }
        if is_stop_command(&text) {
            return;
        }
        self.voice.heard = Some((text.clone(), Instant::now()));
        self.prompt = text;
        if let Err(why) = self.can_run() {
            self.push(LineKind::Bad, format!("Can't run the voice command: {why}"));
            self.speak(
                if bangla {
                    "দুঃখিত, এখন কাজটা চালাতে পারছি না।"
                } else {
                    "Sorry, I can't run that right now."
                },
                lang,
            );
            return;
        }
        self.voice.next_prompt_lang = Some(lang);
        self.start_agent();
        if self.running.is_some() {
            self.voice.task_lang = Some(lang);
            self.voice.reply.clear();
            self.voice.conversation = true;
        }
        self.voice.next_prompt_lang = None;
    }

    /// The prompt as sent to Claude: voice commands get [`VOICE_HINT`].
    pub(super) fn voice_prompt(&self, prompt: &str) -> String {
        if self.voice.next_prompt_lang.is_some() {
            format!("{prompt}{VOICE_HINT}")
        } else {
            prompt.to_owned()
        }
    }

    /// "You" or "You, by voice" for the chat line.
    pub(super) fn you_label(&self) -> &'static str {
        if self.voice.next_prompt_lang.is_some() {
            "You, by voice"
        } else {
            "You"
        }
    }

    /// Claude's streamed text during a voice task.
    pub(super) fn voice_on_text(&mut self, text: &str) {
        if self.voice.task_lang.is_some() {
            self.voice.reply.push_str(text);
        }
    }

    /// A new model turn: only the last turn's text is spoken.
    pub(super) fn voice_on_turn(&mut self) {
        if self.voice.task_lang.is_some() {
            self.voice.reply.clear();
        }
    }

    /// A task ended: speak the result of a voice task.
    pub(super) fn voice_on_task_done(&mut self, outcome: &TaskOutcome) {
        let Some(lang) = self.voice.task_lang.take() else {
            return;
        };
        self.voice.heard = None;
        let reply = std::mem::take(&mut self.voice.reply);
        let summary = match outcome {
            TaskOutcome::Completed(_) => spoken_summary(&reply),
            _ => String::new(),
        };
        let text = if summary.is_empty() {
            fallback_reply(outcome, lang == "bn").to_owned()
        } else {
            summary
        };
        // Speak with the voice of the reply's script: Claude often answers Banglish
        // (Latin) commands in Bangla, which an English voice cannot read.
        let voice = if super::fonts::has_bangla(&text) {
            "bn"
        } else {
            "en"
        };
        self.speak(&text, voice);
        // Like Siri: stay in the conversation and listen for a follow-up after the
        // reply (unless the user stopped the task). Silence ends it.
        if self.voice.conversation && !matches!(outcome, TaskOutcome::Stopped(_)) {
            self.voice.follow_up_pending = true;
        } else {
            self.voice.conversation = false;
        }
    }

    fn speak(&self, text: &str, lang: &str) {
        if !self.settings.voice_talk_back {
            return;
        }
        if let Err(e) = self.platform.speech.speak(text, lang) {
            tracing::warn!("talk back: {e}");
        }
    }

    /// Every frame: mute the microphone while speaking, so the assistant does not hear
    /// itself; let the heard command expire; handle tray clicks.
    pub(super) fn voice_tick(&mut self) {
        let speaking = self.platform.speech.is_speaking();
        if let Some(a) = &self.voice.assistant {
            a.set_muted(speaking);
        }
        if speaking {
            self.ctx.request_repaint_after(Duration::from_millis(150));
        } else if self.voice.follow_up_pending
            && self.running.is_none()
            && self.voice.status == VoiceStatus::Ready
            && let Some(a) = &self.voice.assistant
        {
            // The reply has been spoken: listen for the next question.
            self.voice.follow_up_pending = false;
            self.voice.follow_up = true;
            a.listen_now();
        }
        if self
            .voice
            .heard
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > HEARD_SHOWN && self.running.is_none())
        {
            self.voice.heard = None;
        }
        self.update_tray_status();
        while let Some(action) = self.voice.tray.as_ref().and_then(Tray::poll) {
            tracing::info!("tray menu: {action:?}");
            match action {
                TrayAction::Show => self.leave_background(),
                TrayAction::Talk => self.voice_push_to_talk(),
                TrayAction::ModeShortcut => self.voice_set_mode(false),
                TrayAction::ModeWake => self.voice_set_mode(true),
                TrayAction::Stop => self.stop("tray menu"),
                TrayAction::Quit => {
                    self.voice.allow_close = true;
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Keeps the tray status line in step with what is happening.
    fn update_tray_status(&mut self) {
        let running = self.running.is_some();
        let text = if running {
            "Working on a task…"
        } else {
            match self.voice.status {
                VoiceStatus::Off => "Voice assistant off",
                VoiceStatus::Downloading { .. } => "Downloading the speech model…",
                VoiceStatus::Starting => "Starting the voice assistant…",
                VoiceStatus::Ready if self.always_listening() => "Listening for “Hey Mouse”",
                VoiceStatus::Ready => "Ready (microphone off)",
                VoiceStatus::Listening => "Listening…",
                VoiceStatus::Understanding => "Understanding…",
                VoiceStatus::Error(_) => "Voice assistant stopped (see Settings)",
            }
        };
        let key = format!("{text}{running}");
        if key != self.voice.tray_status {
            if let Some(t) = &self.voice.tray {
                t.set_status(text, running);
            }
            self.voice.tray_status = key;
        }
    }

    /// The overlay for listening / understanding / a just-heard command (task overlays
    /// are handled by the caller).
    pub(super) fn voice_overlay(&self) -> Option<View> {
        let level = f32::from_bits(self.voice.level.load(Ordering::Relaxed));
        match self.voice.status {
            VoiceStatus::Listening => Some(View {
                mode: Mode::Listening,
                detail: if self.voice.follow_up {
                    "Anything else? Stay quiet to finish".to_owned()
                } else {
                    String::new()
                },
                level,
            }),
            VoiceStatus::Understanding => Some(View {
                mode: Mode::Understanding,
                detail: String::new(),
                level,
            }),
            _ => self.voice.heard.as_ref().map(|(text, _)| View {
                mode: Mode::Working,
                detail: format!("“{text}”"),
                level,
            }),
        }
    }

    // ---------- background mode ----------

    /// Closing the window: keep running in the background (if enabled and there is a
    /// tray icon to come back), else close.
    pub(super) fn on_close_requested(&mut self) {
        // Closing the window ends a voice conversation.
        self.voice_end_conversation();
        if self.voice.allow_close || !self.settings.run_in_background || self.voice.tray.is_none() {
            return;
        }
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.running.is_none() && !self.compact {
            self.enter_background();
        }
    }

    pub(super) fn enter_background(&mut self) {
        if self.voice.background {
            return;
        }
        let (pos, size) = self.ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect.map(|r| r.min), v.inner_rect.map(|r| r.size()))
        });
        if let (Some(p), Some(s)) = (pos, size) {
            self.voice.saved = Some((p, s));
        }
        self.make_tiny();
        if !self.voice.told_background {
            self.voice.told_background = true;
            let how = if self.voice.assistant.is_some() {
                "Desktop Pilot keeps listening for “Hey Mouse”."
            } else {
                "Desktop Pilot keeps running."
            };
            self.push(
                LineKind::Info,
                format!("{how} Use the menu-bar / tray icon to open it again or quit."),
            );
        }
    }

    /// Shrinks the window to an invisible, click-through 1×1 window. (Hidden or
    /// minimized windows stop the UI loop on macOS, so voice events would wait.)
    pub(super) fn make_tiny(&mut self) {
        self.voice.background = true;
        let corner = self
            .primary_rect_points()
            .map_or(egui::pos2(0.0, 0.0), |r| {
                r.right_bottom() - egui::vec2(1.0, 1.0)
            });
        for cmd in [
            egui::ViewportCommand::Decorations(false),
            egui::ViewportCommand::MousePassthrough(true),
            egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal),
            egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)),
            egui::ViewportCommand::OuterPosition(corner),
        ] {
            self.ctx.send_viewport_cmd(cmd);
        }
    }

    /// Makes the window clickable again (before showing it). The panel has no title
    /// bar, so decorations stay off.
    pub(super) fn unmake_tiny(&mut self) {
        self.voice.background = false;
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::MousePassthrough(false));
    }

    /// Microphone level 0–1 (for the orb).
    pub(super) fn voice_level(&self) -> f32 {
        f32::from_bits(self.voice.level.load(Ordering::Relaxed))
    }

    pub(super) fn leave_background(&mut self) {
        if !self.voice.background {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        self.unmake_tiny();
        let (pos, size) = self
            .voice
            .saved
            .take()
            .unwrap_or((egui::pos2(120.0, 120.0), egui::vec2(420.0, 680.0)));
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    // ---------- settings ----------

    pub(super) fn voice_settings_ui(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;

        ui.strong("Voice assistant");
        let mut on = self.settings.voice_enabled;
        if ui
            .checkbox(&mut on, "Voice assistant (talk to Desktop Pilot)")
            .changed()
        {
            if on {
                self.voice_enable();
            } else {
                self.voice_disable();
            }
        }
        let status = match &self.voice.status {
            VoiceStatus::Off => "Off".to_owned(),
            VoiceStatus::Downloading { done, total } => format!(
                "Downloading speech model… {} / {} MB",
                done / 1_000_000,
                total / 1_000_000
            ),
            VoiceStatus::Starting => {
                "Starting (loading the model, opening the microphone)…".to_owned()
            }
            VoiceStatus::Ready if self.always_listening() => {
                "Listening for “Hey Mouse” (microphone on)".to_owned()
            }
            VoiceStatus::Ready => format!(
                "Ready. Microphone off until you tap the orb or press {}",
                self.shortcut_label()
            ),
            VoiceStatus::Listening => "Listening for your command…".to_owned(),
            VoiceStatus::Understanding => "Understanding…".to_owned(),
            VoiceStatus::Error(e) => format!("Error: {e}"),
        };
        ui.label(RichText::new(status).small());
        if let VoiceStatus::Downloading { done, total } = self.voice.status {
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(260.0));
        }
        if self.voice.assistant.is_some() {
            let level = f32::from_bits(self.voice.level.load(Ordering::Relaxed));
            ui.horizontal(|ui| {
                ui.label(RichText::new("Microphone").small());
                ui.add(egui::ProgressBar::new(level).desired_width(160.0));
            });
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }

        ui.label("When to listen");
        let always = self.always_listening();
        if ui
            .radio(
                !always,
                format!(
                    "Only when I tap the orb / mic or press {} (microphone off otherwise)",
                    self.shortcut_label()
                ),
            )
            .clicked()
            && always
        {
            self.voice_set_mode(false);
        }
        if ui
            .radio(
                always,
                "Always listen for “Hey Mouse” (microphone stays on; macOS shows the orange dot)",
            )
            .clicked()
            && !always
        {
            self.voice_set_mode(true);
        }
        ui.horizontal(|ui| {
            ui.label("Shortcut");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.settings.voice_shortcut).desired_width(120.0),
            );
            if edit.lost_focus() {
                self.register_voice_shortcut();
            }
            ui.label(RichText::new("e.g. alt+Space, ctrl+shift+Space").small());
        });
        let model_before = self.settings.voice_model.clone();
        egui::ComboBox::from_label("Speech model")
            .selected_text(self.voice_model().label())
            .show_ui(ui, |ui| {
                for m in ModelSize::ALL {
                    ui.selectable_value(
                        &mut self.settings.voice_model,
                        m.id().to_owned(),
                        m.label(),
                    );
                }
            });
        if self.settings.voice_model != model_before && self.settings.voice_enabled {
            // Restart with the new model (downloads it if needed).
            self.voice.assistant = None;
            self.voice.status = VoiceStatus::Off;
            self.voice_enable();
        }
        let lang_before = self.settings.voice_language.clone();
        egui::ComboBox::from_label("Language")
            .selected_text(Language::from_id(&self.settings.voice_language).label())
            .show_ui(ui, |ui| {
                for l in [Language::Auto, Language::Bangla, Language::English] {
                    ui.selectable_value(
                        &mut self.settings.voice_language,
                        l.id().to_owned(),
                        l.label(),
                    );
                }
            });
        if self.settings.voice_language != lang_before {
            self.voice_set_language();
        }
        ui.checkbox(
            &mut self.settings.voice_talk_back,
            "Talk back: speak replies to voice commands",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(
                    "Voice sounds robotic? Download a natural voice (Premium or Enhanced), \
                     e.g. Bangla “Piya (Enhanced)” and English “Ava (Premium)”: System \
                     voice ▸ Manage Voices. It is used automatically.",
                )
                .small(),
            );
            if ui.small_button("Open voice settings").clicked()
                && let Err(e) = crate::platform_init::open_voice_settings()
            {
                self.push(LineKind::Bad, format!("Could not open voice settings: {e}"));
            }
        });
        if self.voice.tray.is_some() {
            ui.checkbox(
                &mut self.settings.run_in_background,
                "Keep running in the background when the window is closed",
            );
        }
        ui.weak(
            RichText::new(
                "Speech is recognized on this computer (Whisper); audio is never saved or sent. \
                 Only the recognized text goes to Claude, like a typed prompt. Approvals still \
                 need a click; “Hey Mouse, stop” stops a task.",
            )
            .small(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_keeps_first_sentences_without_markdown() {
        assert_eq!(
            spoken_summary("**Done!** I opened TextEdit."),
            "Done! I opened TextEdit."
        );
        let long = "This is a sentence that goes on. ".repeat(20);
        assert!(spoken_summary(&long).chars().count() <= MAX_SPOKEN + 1);
        assert_eq!(spoken_summary("কাজ শেষ। আর কিছু?"), "কাজ শেষ। আর কিছু?");
        assert_eq!(spoken_summary(""), "");
    }

    #[test]
    fn fallback_replies_follow_the_language() {
        assert_eq!(
            fallback_reply(&TaskOutcome::Completed(String::new()), false),
            "Done."
        );
        assert_eq!(
            fallback_reply(&TaskOutcome::Failed(String::new()), true),
            "দুঃখিত, কাজটা হয়নি।"
        );
    }
}
