//! Small drawing helpers shared by the layouts.

use dp_config::ModelEntry;
use dp_core::state::AgentState;
use eframe::egui::{self, Color32, RichText, Stroke, TextureHandle};

pub(super) const STOP_RED: Color32 = Color32::from_rgb(198, 40, 40);
pub(super) const GOOD_GREEN: Color32 = Color32::from_rgb(46, 125, 50);
pub(super) const AMBER: Color32 = Color32::from_rgb(230, 145, 20);
pub(super) const NOTE_GRAY: Color32 = Color32::from_rgb(140, 150, 165);

pub(super) fn state_color(state: AgentState) -> Color32 {
    match state {
        AgentState::Idle | AgentState::Paused => Color32::GRAY,
        AgentState::Thinking => Color32::from_rgb(2, 119, 189),
        AgentState::Acting => Color32::from_rgb(47, 91, 211),
        AgentState::AwaitingConfirmation => AMBER,
        AgentState::Completed => GOOD_GREEN,
        AgentState::Stopped | AgentState::Error => STOP_RED,
    }
}

/// A small filled circle in the state's color (drawn, so it needs no font glyph).
pub(super) fn status_dot(ui: &mut egui::Ui, state: AgentState) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 4.5, state_color(state));
}

pub(super) fn model_label(m: &ModelEntry) -> String {
    if m.cost_warning {
        format!("{} (premium cost)", m.display_name)
    } else {
        m.display_name.clone()
    }
}

/// The big red STOP button. Returns true when clicked.
pub(super) fn stop_button(ui: &mut egui::Ui, active: bool, width: f32, height: f32) -> bool {
    let button = egui::Button::new(
        RichText::new("STOP   (Esc)")
            .strong()
            .size(17.0)
            .color(Color32::WHITE),
    )
    .fill(if active {
        STOP_RED
    } else {
        STOP_RED.gamma_multiply(0.45)
    })
    .min_size(egui::vec2(width, height));
    ui.add_enabled(active, button)
        .on_hover_text("Stop immediately (also Ctrl+Alt+Esc anywhere, or push the mouse into the top-left screen corner)")
        .clicked()
}

/// A screenshot shown in the UI, with an optional click marker.
pub(super) struct PreviewImage {
    pub texture: TextureHandle,
    pub size: [usize; 2],
    pub marker: Option<(i64, i64)>,
}

impl std::fmt::Debug for PreviewImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewImage")
            .field("size", &self.size)
            .field("marker", &self.marker)
            .finish()
    }
}

impl PreviewImage {
    pub(super) fn load(ctx: &egui::Context, name: &str, p: &dp_core::agent::Preview) -> Self {
        let size = [p.width as usize, p.height as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &p.rgba);
        Self {
            texture: ctx.load_texture(name, image, egui::TextureOptions::LINEAR),
            size,
            marker: p.marker,
        }
    }

    /// Draws the image scaled to `max_width`, with the marker as an amber ring.
    pub(super) fn show(&self, ui: &mut egui::Ui, max_width: f32) {
        let (w, h) = (self.size[0] as f32, self.size[1] as f32);
        let scale = (max_width / w).min(1.0);
        let size = egui::vec2(w * scale, h * scale);
        let response = ui.add(egui::Image::new((self.texture.id(), size)));
        if let Some((mx, my)) = self.marker {
            let center = response.rect.min + egui::vec2(mx as f32 * scale, my as f32 * scale);
            let painter = ui.painter();
            painter.circle_stroke(center, 11.0, Stroke::new(2.5, AMBER));
            painter.circle_filled(center, 3.0, AMBER);
        }
    }
}
