//! The main window, styled like Siri: a floating dark-glass panel with a flowing
//! colored border; an animated orb, a greeting and suggestions when empty; your
//! words in large type and Claude's answers in glass cards once a task has run; a
//! rounded "type or talk" field at the bottom.

use eframe::egui::{self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Ui, vec2};

use super::siri::{self, OrbState};
use super::style::{self, Icon};
use super::{LineKind, PilotApp, View};
use crate::gui::mouse::MouseRequest;

/// Text sizes used across the chat window.
const TEXT_BODY: f32 = 14.0;
const TEXT_SMALL: f32 = 12.5;
const TEXT_TINY: f32 = 11.5;

/// Longest comfortable reading width; wider windows center the content.
const MAX_COLUMN: f32 = 760.0;

/// Lays `add` out in a column at most [`MAX_COLUMN`] wide, centered horizontally.
fn centered_column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let avail = ui.available_rect_before_wrap();
    let width = avail.width().min(MAX_COLUMN);
    let left = avail.center().x - width / 2.0;
    let rect =
        egui::Rect::from_min_size(egui::pos2(left, avail.top()), vec2(width, avail.height()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add);
}

/// Renders the Markdown that chat answers commonly use (headings, bullets, `**bold**`,
/// `` `code` ``) into a text layout. Anything else stays as plain text.
fn markdown_job(text: &str, size: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{FontFamily, FontId};

    let mut job = LayoutJob::default();
    let plain = |s: f32, c: Color32| TextFormat {
        font_id: FontId::new(s, FontFamily::Proportional),
        color: c,
        ..Default::default()
    };
    for (i, raw_line) in text.lines().enumerate() {
        if i > 0 {
            job.append("\n", 0.0, plain(size, style::TEXT));
        }
        let mut line = raw_line;
        let mut line_size = size;
        let mut base = style::TEXT;
        // Headings: "# ", "## ", "### " → bigger, brighter.
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=3).contains(&hashes) && line[hashes..].starts_with(' ') {
            line = line[hashes + 1..].trim_start();
            line_size = size + (4 - hashes) as f32 * 1.5;
            base = Color32::WHITE;
        }
        // Bullets: "- " or "* " → "•".
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            job.append(
                &format!("{}• ", " ".repeat(indent)),
                0.0,
                plain(line_size, style::ACCENT),
            );
            line = &trimmed[2..];
        }
        // Inline **bold** and `code`.
        let mut rest = line;
        while !rest.is_empty() {
            let bold = rest.find("**");
            let code = rest.find('`');
            match (bold, code) {
                (Some(b), c) if c.is_none_or(|c| b < c) => {
                    job.append(&rest[..b], 0.0, plain(line_size, base));
                    let after = &rest[b + 2..];
                    match after.find("**") {
                        Some(end) => {
                            job.append(&after[..end], 0.0, plain(line_size, Color32::WHITE));
                            rest = &after[end + 2..];
                        }
                        None => {
                            job.append(&rest[b..], 0.0, plain(line_size, base));
                            rest = "";
                        }
                    }
                }
                (_, Some(c)) => {
                    job.append(&rest[..c], 0.0, plain(line_size, base));
                    let after = &rest[c + 1..];
                    match after.find('`') {
                        Some(end) => {
                            job.append(
                                &after[..end],
                                0.0,
                                TextFormat {
                                    font_id: FontId::new(line_size - 1.0, FontFamily::Monospace),
                                    color: Color32::from_rgb(230, 200, 140),
                                    background: Color32::from_rgb(40, 42, 48),
                                    ..Default::default()
                                },
                            );
                            rest = &after[end + 1..];
                        }
                        None => {
                            job.append(&rest[c..], 0.0, plain(line_size, base));
                            rest = "";
                        }
                    }
                }
                _ => {
                    job.append(rest, 0.0, plain(line_size, base));
                    rest = "";
                }
            }
        }
    }
    job
}

/// Suggestions shown on the empty screen.
/// Suggestions on the home screen (the text editor depends on the OS).
fn suggestions(os: dp_platform::OsKind) -> [&'static str; 3] {
    let editor = match os {
        dp_platform::OsKind::MacOs => "Open TextEdit and type hello",
        _ => "Open Notepad and type hello",
    };
    [
        editor,
        "Open my Downloads folder",
        "Tell me what's on my screen",
    ]
}

impl PilotApp {
    /// The orb's state: listening to the voice, thinking/working, or idle.
    fn orb_state(&self) -> OrbState {
        use super::voice::VoiceStatus;
        match self.voice.status {
            VoiceStatus::Listening => OrbState::Listening,
            VoiceStatus::Understanding => OrbState::Thinking,
            _ if self.running.is_some() => OrbState::Thinking,
            _ => OrbState::Idle,
        }
    }

    pub(super) fn chat_ui(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let t = ui.input(|i| i.time) as f32;
        let active = self.orb_state() != OrbState::Idle;
        siri::glass_panel(
            &ui.painter_at(full),
            full.shrink(1.0),
            t,
            siri::energy(active),
        );
        // The border and orb are always gently moving.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(if active {
                33
            } else {
                80
            }));

        egui::Panel::top("header")
            .show_separator_line(false)
            .frame(Frame::NONE.inner_margin(Margin {
                left: 16,
                right: 12,
                top: 10,
                bottom: 4,
            }))
            .show(ui, |ui| self.header(ui));

        if self.view == View::Chat && self.is_connected() {
            egui::Panel::bottom("composer")
                .show_separator_line(false)
                .frame(Frame::NONE.inner_margin(Margin {
                    left: 14,
                    right: 14,
                    top: 4,
                    bottom: 14,
                }))
                .show(ui, |ui| centered_column(ui, |ui| self.composer_area(ui)));
        }

        egui::CentralPanel::default()
            .frame(Frame::NONE.inner_margin(Margin::symmetric(18, 4)))
            .show(ui, |ui| match self.view {
                View::Chat => centered_column(ui, |ui| {
                    if !self.is_connected() {
                        self.connection_screen(ui);
                    } else if self.has_conversation() {
                        self.transcript(ui);
                    } else {
                        self.empty_state(ui);
                    }
                }),
                View::Settings => centered_column(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("settings")
                        .show(ui, |ui| self.settings_ui(ui));
                }),
                View::MouseTest => {
                    let busy = self.running.is_some();
                    let cursor = self.cursor();
                    if let Some(req) = self.mouse.ui(ui, busy, cursor, &self.monitors) {
                        match req {
                            MouseRequest::Run(label, action) => self.start_mouse(label, action),
                            MouseRequest::Pick => self.pick_position(),
                        }
                    }
                    ui.add_space(10.0);
                    self.transcript(ui);
                }
            });
        siri::resize_grip(ui, full);
    }

    fn has_conversation(&self) -> bool {
        self.log.iter().any(|l| l.kind == LineKind::You)
    }

    fn header(&mut self, ui: &mut Ui) {
        // The whole header strip moves the window (there is no title bar).
        let strip = ui.available_rect_before_wrap();
        let strip = egui::Rect::from_min_size(strip.min, vec2(strip.width(), 34.0));
        siri::drag_area(ui, strip);
        ui.horizontal(|ui| {
            if self.view != View::Chat {
                if style::icon_button(ui, Icon::Back, 30.0, None, style::TEXT, true)
                    .on_hover_text("Back")
                    .clicked()
                {
                    self.view = View::Chat;
                }
                let title = if self.view == View::Settings {
                    "Settings"
                } else {
                    "Mouse test"
                };
                ui.label(RichText::new(title).size(16.0).color(style::TEXT).strong());
            } else {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                let t = ui.input(|i| i.time) as f32;
                siri::big_orb(
                    ui.painter(),
                    r.center(),
                    8.0,
                    self.orb_state(),
                    self.voice_level(),
                    t,
                );
                ui.label(
                    RichText::new("Desktop Pilot")
                        .size(14.0)
                        .strong()
                        .color(style::TEXT),
                );
                if self.running.is_some() || self.usage.total() > 0 {
                    ui.label(
                        RichText::new(self.status_text())
                            .size(12.0)
                            .color(style::TEXT_DIM),
                    );
                } else if let super::Conn::Ready(desc) = &self.conn {
                    // Leave room for the three buttons on the right; cut with "…".
                    let room = (ui.available_width() - 112.0).max(0.0);
                    ui.allocate_ui_with_layout(
                        vec2(room, 18.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(desc).size(TEXT_TINY).color(style::TEXT_DIM),
                                )
                                .truncate(),
                            )
                            .on_hover_text(format!("Connected: {desc}"));
                        },
                    );
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if siri::close_button(ui)
                    .on_hover_text("Close (Desktop Pilot keeps running in the menu bar)")
                    .clicked()
                {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let on_top_color = if self.keep_on_top {
                    style::ACCENT
                } else {
                    style::TEXT_DIM
                };
                if style::icon_button(ui, Icon::Pin, 30.0, None, on_top_color, true)
                    .on_hover_text(if self.keep_on_top {
                        "Keep on top: on"
                    } else {
                        "Keep on top: off"
                    })
                    .clicked()
                {
                    self.keep_on_top = !self.keep_on_top;
                    let level = if self.keep_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    };
                    self.ctx
                        .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                }
                if style::icon_button(ui, Icon::Gear, 30.0, None, style::TEXT_DIM, true)
                    .on_hover_text("Settings")
                    .clicked()
                {
                    self.view = if self.view == View::Settings {
                        View::Chat
                    } else {
                        View::Settings
                    };
                }
            });
        });
    }

    fn empty_state(&mut self, ui: &mut Ui) {
        use super::voice::VoiceStatus;

        let t = ui.input(|i| i.time) as f32;
        let suggestions = suggestions(self.platform.info.os);
        let avail = ui.available_size();
        let chips_h = suggestions.len() as f32 * 48.0;
        let top_space = ((avail.y - chips_h - 190.0) * 0.4).max(8.0);
        ui.add_space(top_space);
        let mut talk = false;
        ui.vertical_centered(|ui| {
            // The orb: tap it to talk.
            let (rect, orb) = ui.allocate_exact_size(vec2(110.0, 110.0), egui::Sense::click());
            siri::big_orb(
                ui.painter(),
                rect.center(),
                if orb.hovered() { 40.0 } else { 38.0 },
                self.orb_state(),
                self.voice_level(),
                t,
            );
            if orb
                .on_hover_text("Tap to talk")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                talk = true;
            }
            ui.add_space(8.0);
            let title = match self.voice.status {
                VoiceStatus::Listening => "Listening…".to_owned(),
                VoiceStatus::Understanding => "Understanding…".to_owned(),
                _ => format!("Hi {}, what can I help with?", self.user_name),
            };
            ui.label(RichText::new(title).size(22.0).color(style::TEXT));
            ui.add_space(4.0);
            let hint = match self.voice.status {
                VoiceStatus::Ready if self.always_listening() => {
                    "Say “Hey Mouse”, tap the orb, or type below".to_owned()
                }
                VoiceStatus::Listening => "Speak in Bangla or English".to_owned(),
                VoiceStatus::Downloading { .. } => "Getting the voice assistant ready…".to_owned(),
                _ => format!(
                    "Tap the orb or press {} to talk, or type below",
                    self.shortcut_label()
                ),
            };
            ui.label(RichText::new(hint).size(13.0).color(style::TEXT_DIM));
        });
        if talk {
            self.voice_push_to_talk();
        }
        // Suggestions sit at the bottom, just above the input field.
        let mut picked = None;
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            ui.add_space(6.0);
            for s in suggestions.iter().rev() {
                if siri::suggestion(ui, s, t).clicked() {
                    picked = Some(*s);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(s) = picked
            && self.running.is_none()
        {
            self.prompt = s.to_owned();
            self.start_agent();
        }
    }

    fn transcript(&mut self, ui: &mut Ui) {
        use egui::Label;

        egui::ScrollArea::vertical()
            .id_salt("transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // Pin the content to the visible width so nothing can push lines
                // outside the window; every label below wraps explicitly.
                let width = ui.available_width();
                ui.set_width(width);
                ui.add_space(6.0);
                for line in &self.log {
                    match line.kind {
                        LineKind::Info if !self.show_details => continue,
                        LineKind::You => {
                            // Stored as "You (model): prompt"; show only the prompt.
                            let text = line
                                .text
                                .split_once("): ")
                                .map_or(line.text.as_str(), |(_, t)| t);
                            // Siri style: what you asked, in large type.
                            ui.add_space(6.0);
                            let by_voice = line.text.starts_with("You, by voice");
                            ui.label(
                                RichText::new(if by_voice { "You said" } else { "You asked" })
                                    .size(TEXT_TINY)
                                    .color(style::TEXT_DIM),
                            );
                            ui.add(
                                Label::new(
                                    RichText::new(format!("“{text}”"))
                                        .size(19.0)
                                        .color(style::TEXT),
                                )
                                .wrap(),
                            );
                            ui.add_space(10.0);
                        }
                        LineKind::Claude => {
                            // Answers in a glass card.
                            siri::card_frame().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(Label::new(markdown_job(&line.text, TEXT_BODY)).wrap());
                            });
                            ui.add_space(8.0);
                        }
                        LineKind::Note => {
                            ui.add(
                                Label::new(
                                    RichText::new(&line.text)
                                        .size(TEXT_SMALL)
                                        .italics()
                                        .color(style::TEXT_DIM),
                                )
                                .wrap(),
                            );
                            ui.add_space(4.0);
                        }
                        LineKind::Action => {
                            ui.horizontal_top(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(14.0, 16.0), egui::Sense::hover());
                                super::overlay::sparkle(
                                    ui.painter(),
                                    r.center(),
                                    4.5,
                                    &super::overlay::AI_COLORS,
                                    0.0,
                                );
                                ui.add(
                                    Label::new(
                                        RichText::new(&line.text)
                                            .size(TEXT_SMALL)
                                            .color(style::TEXT_DIM),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                        LineKind::Good => {
                            ui.add(
                                Label::new(
                                    RichText::new(&line.text)
                                        .size(TEXT_SMALL)
                                        .color(super::widgets::GOOD_GREEN),
                                )
                                .wrap(),
                            );
                            ui.add_space(6.0);
                        }
                        LineKind::Bad => {
                            ui.add(
                                Label::new(
                                    RichText::new(&line.text).size(TEXT_SMALL).color(style::RED),
                                )
                                .wrap(),
                            );
                            ui.add_space(6.0);
                        }
                        LineKind::Info => {
                            ui.add(
                                Label::new(
                                    RichText::new(format!("{}  {}", line.time.trim(), line.text))
                                        .size(TEXT_TINY)
                                        .monospace()
                                        .color(style::TEXT_DIM),
                                )
                                .wrap(),
                            );
                        }
                    }
                }
                // Waiting for Claude's reply (chat answers stay in this window).
                if self.running.is_some()
                    && !self.compact
                    && self.state == dp_core::state::AgentState::Thinking
                {
                    let t = ui.input(|i| i.time) as f32;
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::hover());
                        siri::big_orb(ui.painter(), r.center(), 11.0, OrbState::Thinking, 0.0, t);
                        ui.add_space(4.0);
                        ui.label(super::overlay::shimmer_text("Thinking…", 14.0, t));
                    });
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(33));
                }
                ui.add_space(6.0);
            });
    }

    /// Shown until Claude is reachable: progress, or what's wrong and how to fix it.
    fn connection_screen(&mut self, ui: &mut Ui) {
        let avail = ui.available_size();
        ui.add_space((avail.y * 0.22).max(10.0));
        let conn = self.conn.clone();
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), egui::Sense::hover());
            style::paint_logo(ui.painter(), rect.center(), 44.0);
            ui.add_space(16.0);
            match conn {
                super::Conn::Checking => {
                    ui.add(egui::Spinner::new().size(22.0).color(style::ACCENT));
                    ui.add_space(10.0);
                    let (title, how) = match self.settings.engine {
                        dp_config::Engine::ClaudeCode => {
                            ("Connecting to Claude…", "Checking your Claude Code sign-in")
                        }
                        dp_config::Engine::ClaudeApi => {
                            ("Connecting to Claude…", "Checking your Claude API access")
                        }
                        dp_config::Engine::GeminiApi => {
                            ("Connecting to Gemini…", "Checking your Gemini API key")
                        }
                        dp_config::Engine::Antigravity => (
                            "Connecting to Gemini…",
                            "Checking Antigravity and your Google sign-in",
                        ),
                    };
                    ui.label(RichText::new(title).size(20.0).color(style::TEXT));
                    ui.add_space(4.0);
                    ui.label(RichText::new(how).size(13.0).color(style::TEXT_DIM));
                }
                super::Conn::Failed { title, detail, fix } => {
                    ui.label(RichText::new(title).size(20.0).color(style::TEXT));
                    ui.add_space(8.0);
                    ui.set_max_width(avail.x * 0.9);
                    ui.label(RichText::new(detail).size(13.0).color(style::TEXT_DIM));
                    ui.add_space(18.0);
                    if let Some(fix) = fix {
                        let label = match (fix, self.signing_in) {
                            (_, true) => "Working… (check your browser if it opened)",
                            (super::Fix::SignInClaude, false) => "Sign in to Claude",
                            (super::Fix::ConnectAntigravity, false) => {
                                "Connect Desktop Pilot to Antigravity"
                            }
                        };
                        let button = egui::Button::new(
                            RichText::new(label).size(15.0).color(Color32::WHITE),
                        )
                        .fill(style::ACCENT)
                        .min_size(vec2(240.0, 38.0));
                        if ui.add_enabled(!self.signing_in, button).clicked() {
                            match fix {
                                super::Fix::SignInClaude => self.sign_in(),
                                super::Fix::ConnectAntigravity => self.connect_antigravity(),
                            }
                        }
                        ui.add_space(8.0);
                    }
                    ui.horizontal(|ui| {
                        let w = 200.0;
                        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                        if ui
                            .add_sized([95.0, 32.0], egui::Button::new("Retry"))
                            .clicked()
                        {
                            self.check_connection();
                        }
                        if ui
                            .add_sized([95.0, 32.0], egui::Button::new("Settings"))
                            .clicked()
                        {
                            self.view = View::Settings;
                        }
                    });
                }
                super::Conn::Ready(_) => {}
            }
        });
    }

    fn composer_area(&mut self, ui: &mut Ui) {
        self.notice_banner(ui);
        if self.plus_open {
            Frame::new()
                .fill(style::SURFACE)
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(self.running.is_none(), egui::Button::new("New session"))
                            .clicked()
                        {
                            self.new_session();
                            self.plus_open = false;
                        }
                        if ui.button("Settings").clicked() {
                            self.view = View::Settings;
                            self.plus_open = false;
                        }
                        if ui.button("Mouse test").clicked() {
                            self.view = View::MouseTest;
                            self.plus_open = false;
                        }
                        if ui.button("Last screenshot").clicked() {
                            self.show_preview = !self.show_preview;
                            self.plus_open = false;
                        }
                    });
                });
            ui.add_space(6.0);
        }
        if self.show_preview
            && let Some(p) = &self.preview
        {
            p.show(ui, ui.available_width());
            ui.add_space(6.0);
        }

        // Enter sends, Shift+Enter makes a new line (checked before the text box,
        // which would otherwise consume Enter).
        let prompt_id = ui.make_persistent_id("prompt");
        let focused = ui.memory(|m| m.has_focus(prompt_id));
        let enter = focused
            && ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
            });

        let running = self.running.is_some();
        Frame::new()
            .fill(siri::CARD)
            .stroke(egui::Stroke::new(1.0, siri::CARD_STROKE))
            .corner_radius(CornerRadius::same(26))
            .inner_margin(Margin {
                left: 6,
                right: 6,
                top: 6,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let plus_color = if self.plus_open {
                        style::ACCENT
                    } else {
                        style::TEXT
                    };
                    if style::icon_button(ui, Icon::Plus, 36.0, None, plus_color, true)
                        .on_hover_text("New session, settings, mouse test")
                        .clicked()
                    {
                        self.plus_open = !self.plus_open;
                    }
                    let model_w = 98.0;
                    let send_w = 40.0;
                    let mic_w = 40.0;
                    let text_w = (ui.available_width() - model_w - send_w - mic_w - 16.0).max(60.0);
                    let hint = if self.session_turns == 0 {
                        "Ask Desktop Pilot…"
                    } else {
                        "Ask a follow-up…"
                    };
                    ui.add_sized(
                        [text_w, 34.0],
                        egui::TextEdit::multiline(&mut self.prompt)
                            .id(prompt_id)
                            .frame(Frame::NONE)
                            .margin(Margin {
                                left: 4,
                                right: 4,
                                top: 8,
                                bottom: 4,
                            })
                            .desired_rows(1)
                            .font(egui::FontId::proportional(15.0))
                            .text_color(style::TEXT)
                            .hint_text(RichText::new(hint).color(style::TEXT_DIM)),
                    );
                    let short = |m: &dp_config::ModelEntry| {
                        m.display_name
                            .trim_start_matches("Claude ")
                            .trim_start_matches("Gemini ")
                            .to_owned()
                    };
                    let selected = self.selected_model().map(|m| short(&m)).unwrap_or_default();
                    let models = self.current_models().to_vec();
                    egui::ComboBox::from_id_salt("model")
                        .width(model_w - 8.0)
                        .selected_text(RichText::new(selected).color(style::TEXT).size(13.0))
                        .show_ui(ui, |ui| {
                            for (i, m) in models.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.model_idx,
                                    i,
                                    super::widgets::model_label(m),
                                );
                            }
                        });
                    // Push-to-talk: say a command without "Hey Mouse".
                    let (mic_fill, mic_tip) = match self.voice.status {
                        super::voice::VoiceStatus::Listening => {
                            (Some(style::RED), "Listening… speak now (click to stop)")
                        }
                        super::voice::VoiceStatus::Understanding => {
                            (Some(style::ACCENT_2), "Understanding…")
                        }
                        super::voice::VoiceStatus::Ready => (None, "Talk now"),
                        super::voice::VoiceStatus::Downloading { .. } => {
                            (None, "Downloading the speech model…")
                        }
                        _ => (None, "Turn on the voice assistant"),
                    };
                    if style::icon_button(ui, Icon::Mic, 36.0, mic_fill, style::TEXT, !running)
                        .on_hover_text(mic_tip)
                        .clicked()
                    {
                        self.voice_push_to_talk();
                    }
                    if running {
                        if style::icon_button(
                            ui,
                            Icon::Stop,
                            36.0,
                            Some(style::RED),
                            Color32::WHITE,
                            true,
                        )
                        .on_hover_text("Stop (Esc, Ctrl+Alt+Esc)")
                        .clicked()
                        {
                            self.stop("STOP button");
                        }
                    } else {
                        let can = self.can_run();
                        let send = style::icon_button(
                            ui,
                            Icon::Send,
                            36.0,
                            Some(style::ACCENT),
                            Color32::WHITE,
                            can.is_ok(),
                        );
                        let send = match can {
                            Err(why) => send.on_hover_text(why),
                            Ok(()) => send.on_hover_text("Run (Enter)"),
                        };
                        if send.clicked() || (enter && can.is_ok()) {
                            self.start_agent();
                        }
                    }
                });
            });
        let mode = match self.settings.confirm_mode {
            dp_config::ConfirmMode::RiskyOnly => {
                "Asks once per session, then only for security steps"
            }
            dp_config::ConfirmMode::EveryAction => "Asks before every action",
            dp_config::ConfirmMode::Automatic => "Never asks: you can always stop it",
        };
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(mode).size(11.0).color(style::TEXT_DIM));
        });
    }

    fn notice_banner(&mut self, ui: &mut Ui) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        let mut dismiss = false;
        Frame::new()
            .fill(style::RED.gamma_multiply(0.18))
            .stroke(egui::Stroke::new(1.0, style::RED))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(notice.title).strong().color(style::RED));
                ui.label(RichText::new(notice.body).size(12.0).color(style::TEXT));
                ui.horizontal(|ui| {
                    if let Some((text, url)) = notice.link {
                        ui.hyperlink_to(text, url);
                    }
                    if ui.small_button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
        if dismiss {
            self.notice = None;
        }
        ui.add_space(6.0);
    }
}

#[cfg(test)]
mod tests {
    use super::markdown_job;

    fn plain_text(md: &str) -> String {
        markdown_job(md, 14.0).text
    }

    #[test]
    fn markdown_markers_are_removed() {
        assert_eq!(
            plain_text("1. **Work in blocks.** Try it."),
            "1. Work in blocks. Try it."
        );
        assert_eq!(plain_text("- first\n- second"), "• first\n• second");
        assert_eq!(plain_text("## Title"), "Title");
        assert_eq!(plain_text("Run `cargo build` now"), "Run cargo build now");
        // Unclosed markers stay as typed.
        assert_eq!(plain_text("a ** b"), "a ** b");
        assert_eq!(plain_text("2 * 3 = 6"), "2 * 3 = 6");
    }
}
