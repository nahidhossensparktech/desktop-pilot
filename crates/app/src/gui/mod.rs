//! The Desktop Pilot window (DESIGN §3).
//!
//! - **Normal layout** (idle): prompt, model, Run / follow-up, STOP, last screenshot,
//!   settings, mouse test panel, activity log, status bar.
//! - **Compact layout** (task running): a small always-on-top bar at the top-right with
//!   state, current action, Pause and STOP; it grows to show the approval dialog.
//!
//! Threading (ADR-005): the UI runs on the main thread and never blocks. The agent and
//! mouse actions run on tokio with a `CancellationToken`. The global hotkey and the
//! corner fail-safe cancel that token directly, without waiting for a UI frame.

mod app_icon;
mod chat;
mod fonts;
mod mouse;
mod overlay;
mod siri;
mod style;
mod voice;
mod widgets;

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dp_config::{ConfirmMode, Engine, ModelEntry, Settings};
use dp_core::agent::{self, AgentEvent, Decision, Session, TaskConfig, TaskContext, TaskOutcome};
use dp_core::state::AgentState;
use dp_llm::{ClaudeClient, Credentials, Usage};
use dp_platform::{DesktopPoint, HotkeyCombo, MonitorInfo, Platform, SecretKey, WindowId};
use eframe::egui::{self, Color32, RichText};
use secrecy::SecretString;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use mouse::MouseTest;
use widgets::{AMBER, GOOD_GREEN, NOTE_GRAY, PreviewImage, STOP_RED};

const MAX_LOG_LINES: usize = 400;
const POLL: Duration = Duration::from_millis(50);
const PICK_DELAY: Duration = Duration::from_secs(3);
const COMPACT_W: f32 = 460.0;
const COMPACT_H: f32 = 190.0;
const COMPACT_CONFIRM_H: f32 = 520.0;
/// Fail-safe: cursor this close to the primary monitor's top-left corner…
const CORNER_PX: i32 = 2;
/// …for this many consecutive polls (≈150 ms) stops the task.
const CORNER_POLLS: u32 = 3;

/// Starts the GUI. Blocks until the window is closed.
pub(crate) fn run(platform: Platform) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("dp-worker")
        .enable_all()
        .build()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Desktop Pilot")
            .with_icon(app_icon::icon())
            .with_inner_size([420.0, 640.0])
            .with_min_inner_size([340.0, 160.0])
            // Siri-style floating panel: no title bar; the panel draws its own glass,
            // header (drag to move) and close button.
            .with_decorations(false)
            // Lets the "Monitoring" overlay window be see-through (the GL config is
            // chosen once, from the root window). Both layouts paint opaque panels.
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "Desktop Pilot",
        options,
        Box::new(move |cc| Ok(Box::new(PilotApp::new(&cc.egui_ctx, platform, runtime)))),
    )
    .map_err(|e| anyhow::anyhow!("the window could not be opened: {e}"))
}

#[derive(Debug)]
enum UiEvent {
    Monitors(Result<Vec<MonitorInfo>, String>),
    MouseDone {
        id: u64,
        label: String,
        outcome: mouse::Outcome,
    },
    Picked((i32, i32)),
    Agent(AgentEvent),
    TaskDone {
        id: u64,
        outcome: TaskOutcome,
    },
    Auth(Result<String, String>),
    ExternalStop(&'static str),
    /// Claude Code's conversation id, for follow-up prompts.
    ClaudeSession(String),
    Connection(Conn),
    SignInDone(Result<(), String>),
    /// Live model list (id, name) for Gemini / Antigravity.
    EngineModels(Vec<(String, String)>),
    /// Antigravity conversation id, for follow-ups.
    AgySession(String),
    HotkeyReady(String),
    /// Technical detail (hidden unless "Show technical details" is on).
    Info(String),
    /// A short visible note, e.g. turns and tokens used.
    Stat(String),
    Warn(String),
    /// From the voice assistant thread.
    Voice(dp_voice::VoiceEvent),
    /// Speech model download: bytes done, total.
    VoiceProgress(u64, u64),
    /// Speech model download finished (path) or failed.
    VoiceModel(Result<std::path::PathBuf, String>),
    /// The global voice shortcut was pressed.
    VoiceShortcut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    Info,
    You,
    Claude,
    Note,
    Action,
    Good,
    Bad,
}

/// Whether Claude is reachable. The chat only shows when `Ready`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Conn {
    Checking,
    Ready(String),
    Failed {
        title: String,
        detail: String,
        fix: Option<Fix>,
    },
}

/// A one-click fix offered on the connection screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fix {
    SignInClaude,
    ConnectAntigravity,
}

/// Which screen the main window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Chat,
    Settings,
    MouseTest,
}

#[derive(Debug)]
struct LogLine {
    time: String,
    kind: LineKind,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunKind {
    Mouse,
    Agent,
}

#[derive(Debug)]
struct Running {
    id: u64,
    kind: RunKind,
    token: CancellationToken,
    pause: Option<watch::Sender<bool>>,
}

#[derive(Debug)]
struct PendingConfirm {
    description: String,
    reply: Option<oneshot::Sender<Decision>>,
    note: String,
}

struct PilotApp {
    platform: Platform,
    runtime: tokio::runtime::Runtime,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    ctx: egui::Context,
    started: Instant,

    settings: Settings,
    credentials: Arc<Credentials>,
    client: Option<Arc<ClaudeClient>>,
    session: Arc<tokio::sync::Mutex<Session>>,
    session_turns: u32,

    state: AgentState,
    running: Option<Running>,
    next_id: u64,
    log: VecDeque<LogLine>,
    /// Index (from the back) bookkeeping for streaming text: the line being appended to.
    streaming: Option<(LineKind, usize)>,

    prompt: String,
    models: Vec<ModelEntry>,
    model_idx: usize,
    keep_on_top: bool,

    cursor: Arc<Mutex<Option<(i32, i32)>>>,
    monitors: Vec<MonitorInfo>,
    mouse: MouseTest,

    /// The running task's token, shared with the hotkey and fail-safe threads.
    stop_slot: Arc<Mutex<Option<CancellationToken>>>,
    /// The last focused window that is not Desktop Pilot (to hand focus back on Run).
    last_external: Arc<Mutex<Option<WindowId>>>,
    hotkey_status: String,
    /// Training-data record of the running task (Settings ▸ Training data).
    recorder: Option<crate::training::Recorder>,
    /// "Hey Mouse" voice assistant, tray icon and background mode.
    voice: voice::Voice,
    /// Mouse and keyboard control was allowed in this session (Claude Code and
    /// Antigravity engines; the API engines keep it in their `Session`).
    session_approved: Arc<std::sync::atomic::AtomicBool>,

    pending: Option<PendingConfirm>,
    preview: Option<PreviewImage>,
    confirm_preview: Option<PreviewImage>,
    usage: Usage,
    step: u32,
    current_action: String,
    paused: bool,

    compact: bool,
    saved_rect: Option<egui::Rect>,

    api_key_input: String,
    auth_status: String,
    /// In-app MCP server for the Claude Code engine (started on first use).
    mcp_http: Arc<tokio::sync::Mutex<Option<Arc<crate::mcp_http::HttpMcp>>>>,
    /// Claude Code conversation to resume for follow-ups.
    cc_session: Option<String>,
    /// Antigravity conversation to resume for follow-ups.
    agy_conversation: Option<String>,
    gemini: Arc<dp_llm::gemini::GeminiClient>,
    gemini_session: Arc<tokio::sync::Mutex<agent::GeminiSession>>,
    /// Live model list for Gemini / Antigravity.
    engine_models: Vec<ModelEntry>,
    gemini_key_input: String,
    view: View,
    /// Actions taken in the current/last run (0 = a plain chat answer).
    run_actions: u32,
    conn: Conn,
    signing_in: bool,
    plus_open: bool,
    show_details: bool,
    show_preview: bool,
    /// First name for the greeting.
    user_name: String,
    /// A plain-language explanation of the last failure, shown as a banner.
    notice: Option<Notice>,
}

/// A banner that explains a problem and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    title: &'static str,
    body: &'static str,
    link: Option<(&'static str, &'static str)>,
}

/// Turns known API errors into something a non-developer can act on.
fn explain_failure(reason: &str) -> Option<Notice> {
    let r = reason.to_ascii_lowercase();
    if r.contains("credit balance is too low") {
        Some(Notice {
            title: "Your Anthropic account has no API credit",
            body: "Your login works, but the Claude API refuses every request until the \
                   Console account has credit. Add credit, then press Run again. \
                   (A Claude.ai Pro/Max subscription does not cover API use.)",
            link: Some((
                "Open Anthropic billing",
                "https://console.anthropic.com/settings/billing",
            )),
        })
    } else if r.contains("authentication")
        || r.contains("401")
        || r.contains("no claude credentials")
    {
        Some(Notice {
            title: "Claude login problem",
            body: "Run `ant auth login` again in a terminal, or save an API key in Settings.",
            link: None,
        })
    } else if r.contains("rate limit") || r.contains("overloaded") {
        Some(Notice {
            title: "Claude is busy right now",
            body: "The API is rate-limited or overloaded. Wait a minute and try again.",
            link: None,
        })
    } else {
        None
    }
}

impl std::fmt::Debug for PilotApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PilotApp")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PilotApp {
    fn new(ctx: &egui::Context, platform: Platform, runtime: tokio::runtime::Runtime) -> Self {
        let (tx, rx) = channel();
        let (settings, settings_warning) = Settings::load();
        let models = dp_config::default_models();
        let model_idx = models
            .iter()
            .position(|m| m.id == settings.default_model)
            .or_else(|| models.iter().position(|m| m.default))
            .unwrap_or(0);
        let saved_key = platform
            .secrets
            .get(SecretKey::AnthropicApiKey)
            .ok()
            .flatten();
        let credentials = Arc::new(Credentials::new(saved_key));
        let gemini_key = platform.secrets.get(SecretKey::GeminiApiKey).ok().flatten();
        let gemini = Arc::new(
            dp_llm::gemini::GeminiClient::new(gemini_key)
                .unwrap_or_else(|_| dp_llm::gemini::GeminiClient::new(None).expect("HTTP client")),
        );
        // The chat-style look is designed for dark mode.
        ctx.set_theme(egui::ThemePreference::Dark);
        let bangla_font = fonts::add_bangla(ctx);

        let mut app = Self {
            platform,
            runtime,
            tx,
            rx,
            ctx: ctx.clone(),
            started: Instant::now(),
            settings,
            credentials,
            client: None,
            session: Arc::new(tokio::sync::Mutex::new(Session::new())),
            session_turns: 0,
            state: AgentState::Idle,
            running: None,
            next_id: 1,
            log: VecDeque::new(),
            streaming: None,
            prompt: String::new(),
            models,
            model_idx,
            keep_on_top: false,
            cursor: Arc::new(Mutex::new(None)),
            monitors: Vec::new(),
            mouse: MouseTest::default(),
            stop_slot: Arc::new(Mutex::new(None)),
            last_external: Arc::new(Mutex::new(None)),
            hotkey_status: String::new(),
            recorder: None,
            voice: voice::Voice::default(),
            session_approved: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending: None,
            preview: None,
            confirm_preview: None,
            usage: Usage::default(),
            step: 0,
            current_action: String::new(),
            paused: false,
            compact: false,
            saved_rect: None,
            api_key_input: String::new(),
            auth_status: String::new(),
            notice: None,
            mcp_http: Arc::new(tokio::sync::Mutex::new(None)),
            cc_session: None,
            agy_conversation: None,
            gemini,
            gemini_session: Arc::new(tokio::sync::Mutex::new(agent::GeminiSession::new())),
            engine_models: Vec::new(),
            gemini_key_input: String::new(),
            view: View::Chat,
            run_actions: 0,
            conn: Conn::Checking,
            signing_in: false,
            plus_open: false,
            show_details: false,
            show_preview: false,
            user_name: greeting_name(),
        };
        app.push(
            LineKind::Info,
            format!(
                "Desktop Pilot {} on {}",
                env!("CARGO_PKG_VERSION"),
                app.platform.info
            ),
        );
        if let Some(w) = settings_warning {
            app.push(LineKind::Bad, w);
        }
        match bangla_font {
            Some(path) => app.push(LineKind::Info, format!("Bangla font: {path}")),
            None => app.push(
                LineKind::Bad,
                "No Bangla font found on this computer: Bangla text may show as boxes.                  Install Noto Sans Bengali."
                    .to_owned(),
            ),
        }
        match app.credentials.preferred_source() {
            Some(s) => app.push(LineKind::Info, format!("Claude credentials: {s}")),
            None => app.push(
                LineKind::Bad,
                "No Claude credentials yet: open Settings to add an API key, or install the ant CLI and run `ant auth login`.".to_owned(),
            ),
        }
        app.register_hotkey();
        app.start_polling();
        app.load_monitors();
        app.check_connection();
        app.voice_init();
        app
    }

    // ---------- connection ----------

    /// Checks (in the background) that the selected engine can reach Claude.
    fn check_connection(&mut self) {
        self.conn = Conn::Checking;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        match self.settings.engine {
            Engine::ClaudeCode => {
                self.runtime.spawn(async move {
                    let conn = match crate::claude_code::auth_status().await {
                        Ok(s) if s.logged_in => {
                            let how = match (s.plan.as_deref(), s.method.as_str()) {
                                (Some(plan), _) => format!("your Claude {} plan", capitalize(plan)),
                                (None, "claude.ai") => "your Claude plan".to_owned(),
                                (None, m) if !m.is_empty() => format!("{m} sign-in"),
                                _ => "your Claude account".to_owned(),
                            };
                            Conn::Ready(format!("Claude Code · {how}"))
                        }
                        Ok(_) => Conn::Failed {
                            title: "Claude Code is not signed in".to_owned(),
                            detail: "Sign in with your Claude account (a Pro plan works). A browser window opens; come back here when you're done.".to_owned(),
                            fix: Some(Fix::SignInClaude),
                        },
                        Err(e) if crate::claude_code::find_claude().is_none() => Conn::Failed {
                            title: "Claude Code is not installed".to_owned(),
                            detail: format!("Desktop Pilot uses Claude Code to talk to Claude with your plan. Install it from claude.com/claude-code, or switch the engine to Claude API in Settings. ({e})"),
                            fix: None,
                        },
                        Err(e) => Conn::Failed {
                            title: "Could not check Claude Code".to_owned(),
                            detail: e,
                            fix: None,
                        },
                    };
                    let _ = tx.send(UiEvent::Connection(conn));
                    ctx.request_repaint();
                });
            }
            Engine::ClaudeApi => match self.client() {
                Ok(client) => {
                    self.runtime.spawn(async move {
                        let conn = match client.test_connection().await {
                            Ok(desc) => Conn::Ready(format!("Claude API · {}", desc.trim_start_matches("Connected using "))),
                            Err(e) => Conn::Failed {
                                title: "Could not connect to the Claude API".to_owned(),
                                detail: format!("{e}. Save an API key in Settings, or run `ant auth login`, or switch the engine to Claude Code."),
                                fix: None,
                            },
                        };
                        let _ = tx.send(UiEvent::Connection(conn));
                        ctx.request_repaint();
                    });
                }
                Err(e) => {
                    self.conn = Conn::Failed {
                        title: "Could not create the Claude client".to_owned(),
                        detail: e,
                        fix: None,
                    };
                }
            },
            Engine::GeminiApi => {
                if self.gemini.key_source().is_none() {
                    self.conn = Conn::Failed {
                        title: "Add your Gemini API key".to_owned(),
                        detail: "Get a free key at aistudio.google.com/apikey, then paste it in Settings ▸ Gemini API key. Desktop Pilot keeps it in Windows Credential Manager. (Billed by Google separately from a Google One / Gemini subscription; there is a free tier.)".to_owned(),
                        fix: None,
                    };
                    return;
                }
                let gemini = self.gemini.clone();
                self.runtime.spawn(async move {
                    let conn = match gemini.list_models().await {
                        Ok(models) if !models.is_empty() => {
                            let list = models.into_iter().map(|m| (m.id, m.display_name)).collect();
                            let _ = tx.send(UiEvent::EngineModels(list));
                            let source = gemini
                                .key_source()
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            Conn::Ready(format!("Gemini API · {source}"))
                        }
                        Ok(_) => Conn::Failed {
                            title: "No Gemini models available for this key".to_owned(),
                            detail: "The key works, but Google lists no chat models for it."
                                .to_owned(),
                            fix: None,
                        },
                        Err(e) => Conn::Failed {
                            title: "Could not connect to the Gemini API".to_owned(),
                            detail: format!("{e}. Check the key in Settings."),
                            fix: None,
                        },
                    };
                    let _ = tx.send(UiEvent::Connection(conn));
                    ctx.request_repaint();
                });
            }
            Engine::Antigravity => {
                self.runtime.spawn(async move {
                    let conn = match crate::antigravity::models().await {
                        Err(e) if crate::antigravity::find_agy().is_none() => Conn::Failed {
                            title: "Antigravity is not installed".to_owned(),
                            detail: format!("Install Google Antigravity (it includes the `agy` command), sign in, then press Retry. ({e})"),
                            fix: None,
                        },
                        Err(e) => Conn::Failed {
                            title: "Antigravity is not ready".to_owned(),
                            detail: format!("Open Antigravity once and sign in with your Google account, then press Retry. ({e})"),
                            fix: None,
                        },
                        Ok(models) => match crate::antigravity::is_registered().await {
                            Ok(true) => {
                                let _ = tx.send(UiEvent::EngineModels(models));
                                Conn::Ready("Antigravity · your Google account".to_owned())
                            }
                            Ok(false) => Conn::Failed {
                                title: "Connect Desktop Pilot to Antigravity".to_owned(),
                                detail: "One-time step: this adds Desktop Pilot's screen, mouse and keyboard tools to Antigravity's tool list (as `desktop-pilot`). Those tools only act for tasks you start in this window. Note: Antigravity also has its own shell, file and browser tools that Desktop Pilot cannot switch off; runs use Antigravity's sandbox and are told to use only Desktop Pilot's tools.".to_owned(),
                                fix: Some(Fix::ConnectAntigravity),
                            },
                            Err(e) => Conn::Failed {
                                title: "Could not read Antigravity's tool list".to_owned(),
                                detail: e,
                                fix: None,
                            },
                        },
                    };
                    let _ = tx.send(UiEvent::Connection(conn));
                    ctx.request_repaint();
                });
            }
        }
    }

    /// One-time registration of Desktop Pilot's bridge in Antigravity (user's click).
    fn connect_antigravity(&mut self) {
        self.signing_in = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let r = crate::antigravity::register().await;
            let _ = tx.send(UiEvent::SignInDone(r));
            ctx.request_repaint();
        });
    }

    fn sign_in(&mut self) {
        self.signing_in = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let r = crate::claude_code::login().await;
            let _ = tx.send(UiEvent::SignInDone(r));
            ctx.request_repaint();
        });
    }

    fn is_connected(&self) -> bool {
        matches!(self.conn, Conn::Ready(_))
    }

    /// The first real action of a task: shrink to the control bar and hand keyboard
    /// focus back to the app the user was using.
    fn begin_acting(&mut self, refocus: bool) {
        if self.running.is_none() || self.compact {
            return;
        }
        self.enter_compact();
        if refocus && let Some(w) = lock(&self.last_external).clone() {
            let windows = self.platform.windows.clone();
            self.runtime.spawn(async move {
                let _ = windows.focus(&w).await;
            });
        }
    }

    // ---------- log ----------

    fn push(&mut self, kind: LineKind, text: String) {
        if kind != LineKind::Claude && kind != LineKind::Note {
            tracing::info!("{text}");
        }
        let time = format!("{:>6.1}s", self.started.elapsed().as_secs_f32());
        self.log.push_back(LogLine { time, kind, text });
        while self.log.len() > MAX_LOG_LINES {
            self.log.pop_front();
        }
        self.streaming = None;
    }

    /// Appends streamed text to the current Claude/Note line, starting one if needed.
    fn stream(&mut self, kind: LineKind, text: &str) {
        let same = matches!(self.streaming, Some((k, len)) if k == kind && len == self.log.len());
        if same && let Some(last) = self.log.back_mut() {
            last.text.push_str(text);
            return;
        }
        self.push(kind, text.to_owned());
        self.streaming = Some((kind, self.log.len()));
    }

    // ---------- background services ----------

    /// Registers the global FORCE STOP hotkey (works while other apps are focused).
    /// Registers the global FORCE STOP hotkey. If another program holds it (e.g. a
    /// Desktop Pilot started by Claude Code in MCP mode), keeps retrying in the
    /// background and takes it over once it is free.
    fn register_hotkey(&mut self) {
        let combo = HotkeyCombo(self.settings.stop_hotkey.clone());
        let hotkeys = self.platform.hotkeys.clone();
        let slot = self.stop_slot.clone();
        let input = self.platform.input.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let make_callback = move || -> Box<dyn Fn() + Send + Sync> {
            let (slot, input, tx, ctx) = (slot.clone(), input.clone(), tx.clone(), ctx.clone());
            Box::new(move || {
                if let Some(token) = lock(&slot).as_ref() {
                    token.cancel();
                }
                let _ = input.release_all();
                let _ = tx.send(UiEvent::ExternalStop("hotkey"));
                ctx.request_repaint();
            })
        };
        match hotkeys.register(&combo, make_callback()) {
            Ok(_) => {
                self.hotkey_status = format!("Global stop hotkey: {}", combo.0);
                self.push(LineKind::Info, self.hotkey_status.clone());
            }
            Err(e) => {
                self.hotkey_status = format!(
                    "{} is in use by another program (maybe Desktop Pilot running for Claude Code). \
                     Esc, the stop button and the top-left corner still stop tasks; the hotkey is \
                     taken over automatically when it's free.",
                    combo.0
                );
                self.push(
                    LineKind::Info,
                    format!("Global stop hotkey not available yet: {e}"),
                );
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                self.runtime.spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        if hotkeys.register(&combo, make_callback()).is_ok() {
                            let _ = tx.send(UiEvent::Info(format!(
                                "Global stop hotkey active: {}",
                                combo.0
                            )));
                            let _ = tx.send(UiEvent::HotkeyReady(combo.0.clone()));
                            ctx.request_repaint();
                            break;
                        }
                    }
                });
            }
        }
    }

    /// One background loop for: live cursor position, the corner fail-safe, and
    /// tracking the last focused non-Desktop-Pilot window.
    fn start_polling(&self) {
        let platform = self.platform.clone();
        let cursor = self.cursor.clone();
        let slot = self.stop_slot.clone();
        let last_external = self.last_external.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let our_pid = std::process::id();
        self.runtime.spawn(async move {
            let mut corner_hits = 0_u32;
            let mut tick = 0_u64;
            loop {
                tick += 1;
                let pos = match platform.input.cursor_position().await {
                    Ok(Some(DesktopPoint::Physical { x, y })) => Some((x, y)),
                    _ => None,
                };
                let changed = {
                    let mut current = lock(&cursor);
                    let changed = *current != pos;
                    *current = pos;
                    changed
                };
                if changed {
                    ctx.request_repaint();
                }

                // Corner fail-safe (SECURITY §4.3): only while a task runs.
                let active = lock(&slot).as_ref().is_some_and(|t| !t.is_cancelled());
                if active && pos.is_some_and(|(x, y)| x <= CORNER_PX && y <= CORNER_PX) {
                    corner_hits += 1;
                    if corner_hits >= CORNER_POLLS {
                        if let Some(t) = lock(&slot).as_ref() {
                            t.cancel();
                        }
                        let _ = platform.input.release_all();
                        let _ =
                            tx.send(UiEvent::ExternalStop("mouse moved to the top-left corner"));
                        ctx.request_repaint();
                        corner_hits = 0;
                    }
                } else {
                    corner_hits = 0;
                }

                if tick.is_multiple_of(10)
                    && let Ok(Some(w)) = platform.windows.focused().await
                    && w.pid != Some(our_pid)
                {
                    *lock(&last_external) = Some(w.id);
                }
                tokio::time::sleep(POLL).await;
            }
        });
    }

    fn load_monitors(&self) {
        let screen = self.platform.screen.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = screen.monitors().await.map_err(|e| e.to_string());
            let _ = tx.send(UiEvent::Monitors(result));
            ctx.request_repaint();
        });
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        *lock(&self.cursor)
    }

    fn client(&mut self) -> Result<Arc<ClaudeClient>, String> {
        if let Some(c) = &self.client {
            return Ok(c.clone());
        }
        let c = Arc::new(ClaudeClient::new(self.credentials.clone()).map_err(|e| e.to_string())?);
        self.client = Some(c.clone());
        Ok(c)
    }

    // ---------- running things ----------

    fn begin(
        &mut self,
        kind: RunKind,
        pause: Option<watch::Sender<bool>>,
    ) -> (u64, CancellationToken) {
        self.run_actions = 0;
        let id = self.next_id;
        self.next_id += 1;
        let token = CancellationToken::new();
        *lock(&self.stop_slot) = Some(token.clone());
        self.running = Some(Running {
            id,
            kind,
            token: token.clone(),
            pause,
        });
        (id, token)
    }

    fn finish(&mut self, id: u64) {
        if self.running.as_ref().is_some_and(|r| r.id == id) {
            self.running = None;
            *lock(&self.stop_slot) = None;
        }
    }

    fn start_mouse(&mut self, label: String, action: mouse::MouseAction) {
        if self.running.is_some() {
            return;
        }
        let (id, token) = self.begin(RunKind::Mouse, None);
        self.state = AgentState::Acting;
        self.push(LineKind::Action, format!("Start: {label}"));
        let input = self.platform.input.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = mouse::perform(&*input, action, &token).await;
            let _ = input.release_all();
            let _ = tx.send(UiEvent::MouseDone {
                id,
                label,
                outcome: result.into(),
            });
            ctx.request_repaint();
        });
    }

    fn pick_position(&mut self) {
        self.mouse.picking = true;
        self.push(
            LineKind::Info,
            "Point at a spot on screen; position is captured in 3 s".to_owned(),
        );
        let input = self.platform.input.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(PICK_DELAY).await;
            if let Ok(Some(DesktopPoint::Physical { x, y })) = input.cursor_position().await {
                let _ = tx.send(UiEvent::Picked((x, y)));
            }
            ctx.request_repaint();
        });
    }

    fn can_run(&self) -> Result<(), &'static str> {
        if self.running.is_some() {
            return Err("A task is already running");
        }
        if self.prompt.trim().is_empty() {
            return Err("Type what you want done first");
        }
        match self.settings.engine {
            Engine::ClaudeCode => {
                if crate::claude_code::find_claude().is_none() {
                    return Err(
                        "Claude Code (`claude`) was not found. Install it, or switch the engine to Claude API in Settings",
                    );
                }
            }
            Engine::ClaudeApi => {
                if self.credentials.preferred_source().is_none() {
                    return Err(
                        "No Claude credentials: add an API key in Settings, or run `ant auth login`",
                    );
                }
            }
            Engine::GeminiApi => {
                if self.gemini.key_source().is_none() {
                    return Err("No Gemini API key: add one in Settings");
                }
            }
            Engine::Antigravity => {
                if crate::antigravity::find_agy().is_none() {
                    return Err("Antigravity (`agy`) was not found");
                }
            }
        }
        if self.selected_model().is_none() {
            return Err("No model available yet");
        }
        Ok(())
    }

    /// Models for the current engine: the Claude list, or the live list from
    /// Gemini / Antigravity.
    fn current_models(&self) -> &[ModelEntry] {
        if self.settings.engine.uses_claude_models() {
            &self.models
        } else {
            &self.engine_models
        }
    }

    fn selected_model(&self) -> Option<ModelEntry> {
        let models = self.current_models();
        models.get(self.model_idx).or(models.first()).cloned()
    }

    /// Runs the prompt through a command-line agent on this PC: Claude Code (the user's
    /// Claude plan) or Antigravity (`agy`, the user's Google account). Desktop Pilot's
    /// tools are served from inside this window, with its approvals and safety rules.
    fn start_cli(&mut self, agy: bool) {
        let Some(model) = self.selected_model() else {
            self.push(
                LineKind::Bad,
                "No model available yet; wait for the connection check".to_owned(),
            );
            return;
        };
        let engine_name = if agy { "Antigravity" } else { "Claude Code" };
        let prompt = std::mem::take(&mut self.prompt).trim().to_owned();
        let (pause_tx, pause_rx) = watch::channel(false);
        let (id, token) = self.begin(RunKind::Agent, Some(pause_tx));
        self.notice = None;
        self.paused = false;
        self.step = 0;
        self.current_action.clear();
        self.state = AgentState::Thinking;
        self.session_turns += 1;
        self.push(
            LineKind::You,
            format!(
                "{} ({engine_name} · {}): {prompt}",
                self.you_label(),
                model.display_name
            ),
        );
        self.start_recording(&prompt, &model.id);
        let prompt = self.voice_prompt(&prompt);

        let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            while let Some(e) = erx.recv().await {
                if tx.send(UiEvent::Agent(e)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        let platform = self.platform.clone();
        let session_approved = self.session_approved.clone();
        let slot = self.mcp_http.clone();
        let resume = if agy {
            self.agy_conversation.clone()
        } else {
            self.cc_session.clone()
        };
        let confirm = self.settings.confirm_mode;
        let options = crate::mcp::computer_options(&self.settings);
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let send = |e: UiEvent| {
                let _ = tx.send(e);
                ctx.request_repaint();
            };
            // Start (once) the in-app MCP server that Claude Code will call.
            let http = {
                let mut guard = slot.lock().await;
                if guard.is_none() {
                    match crate::mcp_http::start(platform.clone(), options).await {
                        Ok(h) => *guard = Some(Arc::new(h)),
                        Err(e) => {
                            send(UiEvent::TaskDone {
                                id,
                                outcome: TaskOutcome::Failed(format!(
                                    "could not start the local tool server: {e}"
                                )),
                            });
                            return;
                        }
                    }
                }
                guard.clone().expect("set above")
            };
            http.stop.reset();
            {
                let mut server = http.server.lock().await;
                server.set_options(options);
                server.set_hooks(Some(crate::mcp::GuiHooks {
                    events: etx.clone(),
                    confirm,
                    pause: pause_rx,
                    platform: platform.clone(),
                    task_approved: false,
                    session_approved: session_approved.clone(),
                    started: false,
                }));
            }
            // STOP / hotkey / corner cancel the run token: also stop the current action.
            let watcher = {
                let token = token.clone();
                let stop = http.stop.clone();
                tokio::spawn(async move {
                    token.cancelled().await;
                    stop.trigger("stopped");
                })
            };
            let session_event = move |s: String| {
                if agy {
                    UiEvent::AgySession(s)
                } else {
                    UiEvent::ClaudeSession(s)
                }
            };
            let mut final_text = String::new();
            let mut ok = true;
            let mut on_event = |e: crate::claude_code::CcEvent| match e {
                crate::claude_code::CcEvent::Started {
                    session_id,
                    model,
                    mcp_ok,
                } => {
                    let _ = tx.send(session_event(session_id));
                    let status = if mcp_ok {
                        "tools connected"
                    } else {
                        "WARNING: Desktop Pilot tools not connected"
                    };
                    let wrap = if mcp_ok { UiEvent::Info } else { UiEvent::Warn };
                    let _ = tx.send(wrap(format!("{engine_name} started ({model}, {status})")));
                    ctx.request_repaint();
                }
                crate::claude_code::CcEvent::Text(t) => {
                    let _ = etx.send(AgentEvent::TurnStart { step: 0 });
                    let _ = etx.send(AgentEvent::Text(t));
                }
                crate::claude_code::CcEvent::MessageStart => {
                    let _ = etx.send(AgentEvent::TurnStart { step: 0 });
                }
                crate::claude_code::CcEvent::TextDelta(t) => {
                    let _ = etx.send(AgentEvent::Text(t));
                }
                crate::claude_code::CcEvent::Finished {
                    ok: success,
                    text,
                    session_id,
                    turns,
                    tokens,
                } => {
                    if let Some(s) = session_id {
                        let _ = tx.send(session_event(s));
                    }
                    let turns = turns.map(|n| format!("{n} turn(s)")).unwrap_or_default();
                    let tokens = tokens.map(|t| format!(", tokens: {t}")).unwrap_or_default();
                    let _ = tx.send(UiEvent::Stat(format!(
                        "{engine_name} finished: {turns}{tokens}"
                    )));
                    ok = success;
                    final_text = text;
                    ctx.request_repaint();
                }
            };
            let result = if agy {
                let run = crate::antigravity::AgyRun {
                    prompt,
                    model: Some(model.id.clone()),
                    conversation: resume,
                };
                crate::antigravity::run(&run, &token, &mut on_event).await
            } else {
                let run = crate::claude_code::ClaudeRun {
                    prompt,
                    model: Some(model.id.clone()),
                    resume,
                    mcp_url: http.url.clone(),
                    mcp_secret: http.secret.clone(),
                };
                crate::claude_code::run(&run, &token, &mut on_event).await
            };
            watcher.abort();
            http.server.lock().await.set_hooks(None);
            let _ = platform.input.release_all();
            let outcome = match result {
                Err(e) if token.is_cancelled() => TaskOutcome::Stopped(e),
                Err(e) => TaskOutcome::Failed(e),
                Ok(()) if ok => TaskOutcome::Completed(final_text),
                Ok(()) => TaskOutcome::Failed(if final_text.is_empty() {
                    format!("{engine_name} reported an error")
                } else {
                    final_text
                }),
            };
            drop(etx);
            send(UiEvent::TaskDone { id, outcome });
        });
    }

    fn start_agent(&mut self) {
        if self.can_run().is_err() {
            return;
        }
        match self.settings.engine {
            Engine::ClaudeCode => return self.start_cli(false),
            Engine::Antigravity => return self.start_cli(true),
            Engine::ClaudeApi | Engine::GeminiApi => {}
        }
        let gemini = self.settings.engine == Engine::GeminiApi;
        let client = if gemini {
            None
        } else {
            match self.client() {
                Ok(c) => Some(c),
                Err(e) => {
                    self.push(
                        LineKind::Bad,
                        format!("Could not create the Claude client: {e}"),
                    );
                    return;
                }
            }
        };
        let Some(model) = self.selected_model() else {
            self.push(
                LineKind::Bad,
                "No model available yet; wait for the connection check".to_owned(),
            );
            return;
        };
        self.notice = None;
        let prompt = std::mem::take(&mut self.prompt).trim().to_owned();
        let cfg = TaskConfig {
            model: model.clone(),
            confirm: self.settings.confirm_mode,
            max_steps: self.settings.max_steps,
            action_delay: Duration::from_millis(self.settings.action_delay_ms),
            screenshot_long_edge: self.settings.screenshot_long_edge,
            mouse_motion: Duration::from_millis(self.settings.mouse_motion_ms),
            screenshot_after: self.settings.screenshot_after_actions,
        };
        let (pause_tx, mut pause_rx) = watch::channel(false);
        let (id, token) = self.begin(RunKind::Agent, Some(pause_tx));
        self.paused = false;
        self.step = 0;
        self.current_action.clear();
        self.state = AgentState::Thinking;
        self.session_turns += 1;
        self.push(
            LineKind::You,
            format!("{} ({}): {prompt}", self.you_label(), model.display_name),
        );
        self.start_recording(&prompt, &model.id);
        let prompt = self.voice_prompt(&prompt);

        // Forward agent events into the UI channel, waking the UI each time.
        let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            while let Some(e) = erx.recv().await {
                if tx.send(UiEvent::Agent(e)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        let platform = self.platform.clone();
        let session = self.session.clone();
        let gemini_session = self.gemini_session.clone();
        let gemini_client = self.gemini.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let outcome = match client {
                Some(client) => {
                    let mut session = session.lock().await;
                    agent::run_task(
                        TaskContext {
                            platform: &platform,
                            client: &client,
                            events: &etx,
                            cancel: &token,
                        },
                        &mut session,
                        &prompt,
                        &cfg,
                        &mut pause_rx,
                    )
                    .await
                }
                None => {
                    let mut session = gemini_session.lock().await;
                    agent::run_gemini_task(
                        agent::GeminiContext {
                            platform: &platform,
                            client: &gemini_client,
                            events: &etx,
                            cancel: &token,
                        },
                        &mut session,
                        &prompt,
                        &cfg,
                        &mut pause_rx,
                    )
                    .await
                }
            };
            let _ = platform.input.release_all();
            drop(etx);
            let _ = tx.send(UiEvent::TaskDone { id, outcome });
            ctx.request_repaint();
        });
    }

    fn stop(&mut self, source: &str) {
        if let Some(running) = self.running.take() {
            running.token.cancel();
            *lock(&self.stop_slot) = None;
            let _ = self.platform.input.release_all();
            self.pending = None;
            self.confirm_preview = None;
            self.state = AgentState::Stopped;
            self.push(LineKind::Bad, format!("Stopped by user ({source})"));
            if running.kind == RunKind::Agent {
                self.exit_compact();
            }
        }
    }

    fn toggle_pause(&mut self) {
        if let Some(Running { pause: Some(p), .. }) = &self.running {
            self.paused = !self.paused;
            let _ = p.send(self.paused);
            let msg = if self.paused {
                "Paused (finishes the current action first)"
            } else {
                "Resumed"
            };
            self.push(LineKind::Info, msg.to_owned());
        }
    }

    fn answer_confirm(&mut self, approve: bool) {
        if self.pending.is_some()
            && let Some(r) = &mut self.recorder
        {
            r.on_decision(approve);
        }
        if let Some(mut p) = self.pending.take() {
            let decision = if approve {
                Decision::Approve
            } else {
                Decision::Reject(std::mem::take(&mut p.note))
            };
            let verb = if approve { "Approved" } else { "Rejected" };
            self.push(
                if approve {
                    LineKind::Good
                } else {
                    LineKind::Bad
                },
                format!("{verb}: {}", p.description),
            );
            if let Some(reply) = p.reply.take() {
                let _ = reply.send(decision);
            }
            self.confirm_preview = None;
            self.resize_compact();
        }
    }

    fn new_session(&mut self) {
        if self.running.is_some() {
            return;
        }
        self.session = Arc::new(tokio::sync::Mutex::new(Session::new()));
        self.session_approved
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.session_turns = 0;
        self.usage = Usage::default();
        self.preview = None;
        self.state = AgentState::Idle;
        self.cc_session = None;
        self.agy_conversation = None;
        self.gemini_session = Arc::new(tokio::sync::Mutex::new(agent::GeminiSession::new()));
        self.push(
            LineKind::Info,
            "New session: Claude starts with no memory of earlier prompts".to_owned(),
        );
    }

    // ---------- compact mode ----------

    fn primary_rect_points(&self) -> Option<egui::Rect> {
        let m = self
            .monitors
            .iter()
            .find(|m| m.is_primary)
            .or(self.monitors.first())?;
        let s = m.scale_factor.max(1.0) as f32;
        let (x, y) = (
            m.origin_physical.0 as f32 / s,
            m.origin_physical.1 as f32 / s,
        );
        let (w, h) = (m.size_physical.0 as f32 / s, m.size_physical.1 as f32 / s);
        Some(egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(w, h),
        ))
    }

    fn enter_compact(&mut self) {
        if self.compact {
            return;
        }
        // A voice task started while in the background: show the control bar (with
        // STOP), then go back to the background when it ends.
        if self.voice.background {
            self.unmake_tiny();
            self.voice.compact_from_background = true;
        }
        self.saved_rect = self.ctx.input(|i| i.viewport().outer_rect);
        self.compact = true;
        let size = egui::vec2(COMPACT_W, COMPACT_H);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        if let Some(screen) = self.primary_rect_points() {
            let pos = egui::pos2(screen.max.x - COMPACT_W - 16.0, screen.min.y + 16.0);
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
    }

    fn resize_compact(&self) {
        if self.compact {
            let h = if self.pending.is_some() {
                COMPACT_CONFIRM_H
            } else {
                COMPACT_H
            };
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(COMPACT_W, h)));
        }
    }

    fn exit_compact(&mut self) {
        if !self.compact {
            return;
        }
        self.compact = false;
        if std::mem::take(&mut self.voice.compact_from_background) {
            self.saved_rect = None;
            self.make_tiny();
            return;
        }
        if let Some(rect) = self.saved_rect.take() {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::OuterPosition(rect.min));
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    rect.width().max(380.0),
                    (rect.height() - 32.0).max(520.0),
                )));
        }
        let level = if self.keep_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
    }

    // ---------- events ----------

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                UiEvent::Monitors(Ok(monitors)) => {
                    self.monitors = monitors;
                    let summary: Vec<String> = self
                        .monitors
                        .iter()
                        .enumerate()
                        .map(|(i, m)| mouse::describe_monitor(i, m))
                        .collect();
                    self.push(LineKind::Info, format!("Monitors: {}", summary.join("; ")));
                    if let Some(m) = self.monitors.iter().find(|m| m.is_primary) {
                        self.mouse.target = mouse::monitor_center(m);
                    }
                }
                UiEvent::Monitors(Err(e)) => {
                    self.push(LineKind::Bad, format!("Monitor list unavailable: {e}"))
                }
                UiEvent::Picked(p) => {
                    self.mouse.picking = false;
                    self.mouse.target = p;
                    self.push(
                        LineKind::Info,
                        format!("Picked position ({}, {})", p.0, p.1),
                    );
                }
                UiEvent::MouseDone { id, label, outcome } => {
                    self.finish(id);
                    match outcome {
                        mouse::Outcome::Cancelled => {}
                        mouse::Outcome::Done => {
                            self.state = AgentState::Completed;
                            self.push(LineKind::Good, format!("Done: {label}"));
                        }
                        mouse::Outcome::TookOver(msg) => {
                            self.state = AgentState::Stopped;
                            self.push(LineKind::Bad, format!("Stopped: {msg}"));
                        }
                        mouse::Outcome::Failed(msg) => {
                            self.state = AgentState::Error;
                            self.push(LineKind::Bad, format!("Failed: {label}: {msg}"));
                        }
                    }
                }
                UiEvent::Agent(e) => self.on_agent_event(e),
                UiEvent::TaskDone { id, outcome } => {
                    let was_current = self.running.as_ref().is_some_and(|r| r.id == id);
                    if was_current {
                        self.save_recording(&outcome);
                        self.voice_on_task_done(&outcome);
                    }
                    self.finish(id);
                    self.pending = None;
                    self.confirm_preview = None;
                    if was_current {
                        match outcome {
                            TaskOutcome::Completed(_) => {
                                self.state = AgentState::Completed;
                                // A plain chat answer needs no "Done" line.
                                if self.run_actions > 0 {
                                    self.push(
                                        LineKind::Good,
                                        format!("Done ({} action(s)).", self.run_actions),
                                    );
                                }
                            }
                            TaskOutcome::Stopped(reason) => {
                                self.state = AgentState::Stopped;
                                self.push(LineKind::Bad, format!("Stopped: {reason}"));
                            }
                            TaskOutcome::Failed(reason) => {
                                self.state = AgentState::Error;
                                self.notice = explain_failure(&reason);
                                self.push(LineKind::Bad, format!("Error: {reason}"));
                            }
                        }
                        self.exit_compact();
                    }
                }
                UiEvent::Auth(r) => {
                    self.auth_status = match r {
                        Ok(s) => s,
                        Err(e) => format!("Failed: {e}"),
                    };
                    let line = self.auth_status.clone();
                    self.push(LineKind::Info, line);
                }
                UiEvent::ExternalStop(source) => self.stop(source),
                UiEvent::Voice(e) => self.on_voice_event(e),
                UiEvent::VoiceProgress(done, total) => self.on_voice_progress(done, total),
                UiEvent::VoiceModel(result) => self.on_voice_model(result),
                UiEvent::VoiceShortcut => self.voice_push_to_talk(),
                UiEvent::ClaudeSession(id) => {
                    if !id.is_empty() {
                        self.cc_session = Some(id);
                    }
                }
                UiEvent::Info(text) => self.push(LineKind::Info, text),
                UiEvent::Stat(text) => self.push(LineKind::Note, text),
                UiEvent::Connection(c) => {
                    if let Conn::Ready(desc) = &c {
                        self.push(LineKind::Info, format!("Connected: {desc}"));
                    }
                    self.conn = c;
                }
                UiEvent::EngineModels(list) => {
                    let previous = self.selected_model().map(|m| m.id);
                    self.engine_models = list
                        .iter()
                        .map(|(id, name)| ModelEntry::simple(id, name))
                        .collect();
                    self.model_idx = previous
                        .and_then(|p| self.engine_models.iter().position(|m| m.id == p))
                        .unwrap_or(0);
                }
                UiEvent::AgySession(id) => {
                    if !id.is_empty() {
                        self.agy_conversation = Some(id);
                    }
                }
                UiEvent::HotkeyReady(combo) => {
                    self.hotkey_status = format!("Global stop hotkey: {combo}");
                }
                UiEvent::SignInDone(r) => {
                    self.signing_in = false;
                    if let Err(e) = r {
                        self.push(LineKind::Bad, format!("Sign-in did not finish: {e}"));
                    }
                    self.check_connection();
                }
                UiEvent::Warn(text) => self.push(LineKind::Bad, text),
            }
        }
    }

    fn start_recording(&mut self, prompt: &str, model: &str) {
        self.recorder = self.settings.save_training_data.then(|| {
            crate::training::Recorder::start(
                prompt,
                self.settings.engine.label(),
                model,
                &self.platform.info.os.to_string(),
            )
        });
        if self.voice.next_prompt_lang.is_some()
            && let Some(r) = &mut self.recorder
        {
            r.mark_voice();
        }
    }

    /// Appends the finished task to the training-data file (if recording).
    fn save_recording(&mut self, outcome: &TaskOutcome) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let result = dp_config::training_dir()
            .map_err(|e| e.to_string())
            .and_then(|dir| {
                recorder
                    .save(&dir, outcome, &self.usage)
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(path) => tracing::info!("training data saved to {}", path.display()),
            Err(e) => self.push(LineKind::Bad, format!("Could not save training data: {e}")),
        }
    }

    fn on_agent_event(&mut self, e: AgentEvent) {
        if let Some(r) = &mut self.recorder {
            r.on_event(&e);
        }
        match e {
            AgentEvent::State(s) => {
                if self.running.is_some() {
                    self.state = s;
                }
            }
            AgentEvent::TurnStart { step } => {
                // The Claude Code engine sends step 0 (it counts actions instead).
                if step > 0 {
                    self.step = step;
                }
                self.streaming = None;
                self.voice_on_turn();
            }
            AgentEvent::Text(t) => {
                self.voice_on_text(&t);
                self.stream(LineKind::Claude, &t);
            }
            AgentEvent::Thinking(t) => self.stream(LineKind::Note, &t),
            AgentEvent::Action(d) => {
                self.begin_acting(true);
                self.run_actions += 1;
                if self.settings.engine == Engine::ClaudeCode {
                    self.step += 1;
                }
                self.current_action = d.clone();
                self.push(LineKind::Action, d);
            }
            AgentEvent::ActionFailed(d) => self.push(LineKind::Bad, d),
            AgentEvent::Screenshot(p) => {
                self.preview = Some(PreviewImage::load(&self.ctx, "preview", &p));
            }
            AgentEvent::Usage(u) => self.usage = u,
            AgentEvent::Retrying(r) => self.push(LineKind::Info, format!("API busy: {r}")),
            AgentEvent::Confirm(req) => {
                self.begin_acting(false);
                self.confirm_preview = req
                    .preview
                    .as_ref()
                    .map(|p| PreviewImage::load(&self.ctx, "confirm", p));
                self.current_action = req.description.clone();
                self.pending = Some(PendingConfirm {
                    description: req.description,
                    reply: Some(req.reply),
                    note: String::new(),
                });
                self.resize_compact();
                // Bring the approval bar forward without the user hunting for it.
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    // ---------- layouts ----------

    fn status_text(&self) -> String {
        let mut parts = vec![self.state.to_string()];
        if self.step > 0 {
            parts.push(if self.settings.engine == Engine::ClaudeCode {
                format!("{} action(s)", self.step)
            } else {
                format!("step {}/{}", self.step, self.settings.max_steps)
            });
        }
        if self.usage.total() > 0 {
            parts.push(format!("{:.1}k tokens", self.usage.total() as f64 / 1000.0));
        }
        parts.join(" · ")
    }

    fn compact_ui(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let t = ui.input(|i| i.time) as f32;
        siri::glass_panel(&ui.painter_at(full), full.shrink(1.0), t, 1.0);
        siri::drag_area(
            ui,
            egui::Rect::from_min_size(full.min, egui::vec2(full.width(), 30.0)),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 12)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::status_dot(ui, self.state);
                    ui.strong(self.status_text());
                });
                let action = if self.current_action.is_empty() {
                    "Starting…".to_owned()
                } else {
                    self.current_action.clone()
                };
                ui.label(RichText::new(action).small());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = if self.paused { "Resume" } else { "Pause" };
                    if ui
                        .add_sized([90.0, 40.0], egui::Button::new(label))
                        .clicked()
                    {
                        self.toggle_pause();
                    }
                    if widgets::stop_button(ui, self.running.is_some(), ui.available_width(), 40.0)
                    {
                        self.stop("STOP button");
                    }
                });

                if self.pending.is_some() {
                    ui.separator();
                    ui.colored_label(AMBER, RichText::new("Approve this action?").strong());
                    if let Some(p) = &self.confirm_preview {
                        p.show(ui, ui.available_width());
                    }
                    if let Some(p) = &mut self.pending {
                        ui.label(RichText::new(&p.description).strong());
                        ui.add(
                            egui::TextEdit::singleline(&mut p.note)
                                .hint_text("Optional note for Claude if you reject")
                                .desired_width(f32::INFINITY),
                        );
                    }
                    let mut decision = None;
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([140.0, 32.0], egui::Button::new("Reject (Esc)"))
                            .clicked()
                        {
                            decision = Some(false);
                        }
                        let approve = egui::Button::new(
                            RichText::new("Approve (Ctrl+Enter)")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(GOOD_GREEN);
                        if ui.add_sized([180.0, 32.0], approve).clicked() {
                            decision = Some(true);
                        }
                    });
                    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                        decision = Some(true);
                    }
                    if let Some(d) = decision {
                        self.answer_confirm(d);
                    }
                } else {
                    for line in self
                        .log
                        .iter()
                        .rev()
                        .take(3)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                    {
                        let text: String = line.text.chars().take(160).collect();
                        ui.label(RichText::new(text).small().color(line_color(line.kind, ui)));
                    }
                }
            });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Engine");
        let engine_before = self.settings.engine;
        ui.radio_value(
            &mut self.settings.engine,
            Engine::ClaudeCode,
            "Claude Code: uses your Claude plan (e.g. Pro), no API key",
        );
        ui.radio_value(
            &mut self.settings.engine,
            Engine::ClaudeApi,
            "Claude API: API key or ant login, needs Console credit",
        );
        ui.radio_value(
            &mut self.settings.engine,
            Engine::GeminiApi,
            "Gemini API: free key from aistudio.google.com, safest Gemini option",
        );
        ui.radio_value(
            &mut self.settings.engine,
            Engine::Antigravity,
            "Antigravity (Gemini): your Google account, no key",
        );
        if self.settings.engine == Engine::Antigravity {
            ui.colored_label(
                widgets::AMBER,
                RichText::new(
                    "Antigravity has its own shell, file and browser tools that Desktop Pilot \
                     cannot switch off. Runs use Antigravity's sandbox and are told to use only \
                     Desktop Pilot's tools, but those rules are not enforced by Desktop Pilot.",
                )
                .small(),
            );
        }
        if self.settings.engine != engine_before {
            // Each engine has its own models and conversation.
            self.model_idx = 0;
            self.engine_models.clear();
            self.check_connection();
        }
        if self.settings.engine == Engine::GeminiApi {
            ui.add_space(4.0);
            ui.strong("Gemini API key");
            match self.gemini.key_source() {
                Some(s) => ui.label(RichText::new(format!("Using: {s}")).small()),
                None => ui.colored_label(STOP_RED, "No Gemini key yet"),
            };
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.gemini_key_input)
                        .password(true)
                        .hint_text("Gemini API key (AIza…)")
                        .desired_width(200.0),
                );
                if ui
                    .add_enabled(
                        !self.gemini_key_input.trim().is_empty(),
                        egui::Button::new("Save key"),
                    )
                    .clicked()
                {
                    let key = SecretString::from(
                        std::mem::take(&mut self.gemini_key_input).trim().to_owned(),
                    );
                    match self.platform.secrets.set(SecretKey::GeminiApiKey, &key) {
                        Ok(()) => {
                            self.gemini.set_saved_key(Some(key));
                            self.push(
                                LineKind::Good,
                                "Gemini API key saved in Windows Credential Manager".to_owned(),
                            );
                            self.check_connection();
                        }
                        Err(e) => self.push(LineKind::Bad, format!("Could not save the key: {e}")),
                    }
                }
                if ui.button("Remove key").clicked() {
                    let _ = self.platform.secrets.delete(SecretKey::GeminiApiKey);
                    self.gemini.set_saved_key(None);
                    self.check_connection();
                }
            });
            ui.hyperlink_to(
                RichText::new("Get a free Gemini API key").small(),
                "https://aistudio.google.com/apikey",
            );
        }
        ui.horizontal(|ui| {
            let status = match &self.conn {
                Conn::Checking => "Checking connection…".to_owned(),
                Conn::Ready(d) => format!("Connected: {d}"),
                Conn::Failed { title, .. } => format!("Not connected: {title}"),
            };
            ui.label(RichText::new(status).small());
            if ui.small_button("Check again").clicked() {
                self.check_connection();
            }
        });
        if self.settings.engine == Engine::ClaudeCode {
            match crate::claude_code::find_claude() {
                Some(p) => {
                    ui.weak(RichText::new(format!("Claude Code found: {}", p.display())).small())
                }
                None => ui.colored_label(STOP_RED, "Claude Code (`claude`) not found on this PC"),
            };
        }
        ui.separator();

        ui.strong("Claude API connection");
        match self.credentials.preferred_source() {
            Some(s) => ui.label(format!("Using: {s}")),
            None => ui.colored_label(STOP_RED, "No credentials found"),
        };
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.api_key_input)
                    .password(true)
                    .hint_text("API key (sk-ant-…), optional")
                    .desired_width(200.0),
            );
            if ui
                .add_enabled(
                    !self.api_key_input.trim().is_empty(),
                    egui::Button::new("Save key"),
                )
                .clicked()
            {
                let key =
                    SecretString::from(std::mem::take(&mut self.api_key_input).trim().to_owned());
                match self.platform.secrets.set(SecretKey::AnthropicApiKey, &key) {
                    Ok(()) => {
                        self.credentials.set_saved_key(Some(key));
                        self.push(
                            LineKind::Good,
                            "API key saved in Windows Credential Manager".to_owned(),
                        );
                    }
                    Err(e) => self.push(LineKind::Bad, format!("Could not save the key: {e}")),
                }
            }
            if ui.button("Remove key").clicked() {
                match self.platform.secrets.delete(SecretKey::AnthropicApiKey) {
                    Ok(()) => {
                        self.credentials.set_saved_key(None);
                        self.push(LineKind::Info, "Saved API key removed".to_owned());
                    }
                    Err(e) => self.push(LineKind::Bad, format!("Could not remove the key: {e}")),
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Test connection").clicked() {
                match self.client() {
                    Ok(client) => {
                        self.auth_status = "Testing…".to_owned();
                        let tx = self.tx.clone();
                        let ctx = self.ctx.clone();
                        self.runtime.spawn(async move {
                            let r = client.test_connection().await.map_err(|e| e.to_string());
                            let _ = tx.send(UiEvent::Auth(r));
                            ctx.request_repaint();
                        });
                    }
                    Err(e) => self.auth_status = e,
                }
            }
            ui.label(RichText::new(&self.auth_status).small());
        });
        ui.weak(
            RichText::new(
                "Order: ANTHROPIC_API_KEY env > ANTHROPIC_AUTH_TOKEN env > saved key > ant CLI login. \
                 With the ant CLI installed, run `ant auth login` once and leave the key empty.",
            )
            .small(),
        );
        ui.separator();

        ui.strong("Safety");
        ui.radio_value(
            &mut self.settings.confirm_mode,
            ConfirmMode::RiskyOnly,
            "Ask once per session, then only for security steps (installers, permissions, settings, terminals, deleting, passwords, sending)",
        );
        ui.radio_value(
            &mut self.settings.confirm_mode,
            ConfirmMode::EveryAction,
            "Ask before every action",
        );
        ui.radio_value(
            &mut self.settings.confirm_mode,
            ConfirmMode::Automatic,
            "Never ask; I'm watching and can STOP",
        );
        ui.horizontal(|ui| {
            ui.label("Mouse glide");
            ui.add(egui::Slider::new(&mut self.settings.mouse_motion_ms, 0..=1000).suffix(" ms"))
                .on_hover_text("0 = jump. A visible glide lets you follow along; grabbing the mouse stops the task");
        });
        ui.separator();
        ui.strong("Display");
        ui.checkbox(&mut self.show_details, "Show technical details in the chat");
        ui.separator();
        self.voice_settings_ui(ui);
        ui.separator();
        ui.strong("Training data");
        ui.checkbox(
            &mut self.settings.save_training_data,
            "Save each task as training data (prompt, actions, Claude's replies; no screenshots)",
        );
        ui.weak(
            RichText::new(
                "Saved only on this computer, one line per task in a .jsonl file per day. \
                 Passwords, keys and card numbers are masked.",
            )
            .small(),
        );
        if let Ok(dir) = dp_config::training_dir() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(dir.display().to_string()).small().monospace());
                if ui.small_button("Open folder").clicked()
                    && let Err(e) = crate::platform_init::open_folder(&dir)
                {
                    self.push(LineKind::Bad, format!("Could not open the folder: {e}"));
                }
            });
        }
        ui.separator();
        ui.strong("Token saving");
        ui.checkbox(
            &mut self.settings.screenshot_after_actions,
            "Return a screenshot with each action (fewer turns, fewer tokens)",
        );
        ui.weak(
            RichText::new("Smaller screenshots use fewer tokens: 1024 px is a good balance.")
                .small(),
        );
        ui.horizontal(|ui| {
            ui.label("Max steps per task");
            ui.add(egui::DragValue::new(&mut self.settings.max_steps).range(1..=200));
        });
        ui.horizontal(|ui| {
            ui.label("Pause after each action");
            ui.add(egui::Slider::new(&mut self.settings.action_delay_ms, 0..=1000).suffix(" ms"));
        });
        ui.horizontal(|ui| {
            ui.label("Screenshot size");
            egui::ComboBox::from_id_salt("shot")
                .selected_text(format!("{} px", self.settings.screenshot_long_edge))
                .show_ui(ui, |ui| {
                    for px in [1024, 1280, 1366, 1600, 1920] {
                        ui.selectable_value(
                            &mut self.settings.screenshot_long_edge,
                            px,
                            format!("{px} px"),
                        );
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Stop hotkey");
            ui.add(egui::TextEdit::singleline(&mut self.settings.stop_hotkey).desired_width(140.0));
        });
        ui.weak(RichText::new(&self.hotkey_status).small());
        if ui.button("Save settings").clicked() {
            // Only Claude models are remembered as the default (Gemini lists are live).
            if self.settings.engine.uses_claude_models()
                && let Some(m) = self.selected_model()
            {
                self.settings.default_model = m.id;
            }
            self.settings = self.settings.clone().sanitized();
            match self.settings.save() {
                Ok(path) => self.push(
                    LineKind::Good,
                    format!(
                        "Settings saved to {} (hotkey changes apply after restart)",
                        path.display()
                    ),
                ),
                Err(e) => self.push(LineKind::Bad, format!("Could not save settings: {e}")),
            }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// The user's first name for the greeting (Windows/Unix user name, capitalized).
fn greeting_name() -> String {
    let raw = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let first = raw.split(['.', '_', ' ', '-']).next().unwrap_or_default();
    let mut chars = first.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => "there".to_owned(),
    }
}

fn line_color(kind: LineKind, ui: &egui::Ui) -> Color32 {
    match kind {
        LineKind::Info => ui.visuals().weak_text_color(),
        LineKind::You | LineKind::Claude => ui.visuals().strong_text_color(),
        LineKind::Note => NOTE_GRAY,
        LineKind::Action => Color32::from_rgb(110, 150, 235),
        LineKind::Good => GOOD_GREEN,
        LineKind::Bad => STOP_RED,
    }
}

impl eframe::App for PilotApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_events();
        self.voice_tick();
        if ui.input(|i| i.viewport().close_requested()) {
            self.on_close_requested();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.pending.is_some() {
                self.answer_confirm(false);
            } else if self.running.is_some() {
                self.stop("Esc");
            } else if self.voice_listening() {
                self.voice_end_conversation();
            }
        }
        // In the background the window is an invisible 1×1 pixel: draw nothing.
        if self.voice.background {
        } else if self.compact {
            self.compact_ui(ui);
        } else {
            self.chat_ui(ui);
        }
        let view = if self.compact && self.running.is_some() {
            Some(overlay::View {
                mode: if self.paused {
                    overlay::Mode::Paused
                } else {
                    overlay::Mode::Working
                },
                detail: self.current_action.clone(),
                level: 0.0,
            })
        } else {
            self.voice_overlay()
        };
        if let Some(view) = view
            && let Some(screen) = self.primary_rect_points()
        {
            overlay::show(&self.ctx, screen, &view, &self.platform.screen);
        }
    }

    /// Transparent, so the overlay window is see-through. The main window's panels
    /// paint their own opaque background.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

impl Drop for PilotApp {
    /// On window close: stop everything and release held keys and buttons.
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.token.cancel();
        }
        let _ = self.platform.input.release_all();
        crate::mcp_http::remove_endpoint();
    }
}

#[cfg(test)]
mod tests {
    use super::explain_failure;

    #[test]
    fn known_failures_get_a_plain_explanation() {
        let credit = explain_failure(
            "API error 400 invalid_request_error: Your credit balance is too low to access the Anthropic API.",
        )
        .unwrap();
        assert!(credit.title.contains("no API credit"));
        assert!(credit.link.is_some());
        assert!(explain_failure("rate limited; retry after Some(5) s").is_some());
        assert!(explain_failure("could not take a screenshot: x").is_none());
    }
}
