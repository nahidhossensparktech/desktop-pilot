//! The Siri-style look of the window: a floating dark-glass panel with a flowing
//! multicolor border, a large animated orb that reacts to the voice, glass cards and
//! shimmering text. Shares its palette with the screen overlay (`overlay.rs`).
//!
//! The window has no title bar, so this module also provides dragging (on the
//! header) and resizing (bottom-right grip).

use std::f32::consts::TAU;

use eframe::egui::{
    self, Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};

use super::overlay::{AI_COLORS, palette_at};

/// Corner radius of the floating panel.
pub(super) const PANEL_RADIUS: f32 = 26.0;
/// Panel fill: deep, nearly opaque dark glass. (Siri blurs what is behind it; a
/// window here cannot, so a see-through panel made text hard to read.)
const GLASS: Color32 = Color32::from_rgb(17, 17, 25);
/// Lighter glass for cards, pills and the input field.
pub(super) const CARD: Color32 = Color32::from_rgb(33, 33, 44);
pub(super) const CARD_STROKE: Color32 = Color32::from_rgb(52, 52, 66);

/// How lively the panel looks: 0 = idle, 1 = listening / working.
pub(super) fn energy(active: bool) -> f32 {
    if active { 1.0 } else { 0.0 }
}

/// Points around a rounded rectangle, clockwise from the top-left corner's end.
fn rounded_outline(rect: Rect, r: f32, per_corner: usize, per_side: usize) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -TAU / 4.0),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), TAU / 4.0),
        (pos2(rect.left() + r, rect.top() + r), TAU / 2.0),
    ];
    let mut points = Vec::with_capacity(4 * (per_corner + per_side));
    let mut prev_end: Option<Pos2> = Some(pos2(rect.left() + r, rect.top()));
    for (c, start) in corners {
        let first = c + vec2(start.cos(), start.sin()) * r;
        if let Some(from) = prev_end {
            for k in 0..per_side {
                points.push(from.lerp(first, k as f32 / per_side as f32));
            }
        }
        for k in 0..=per_corner {
            let a = start + TAU / 4.0 * k as f32 / per_corner as f32;
            points.push(c + vec2(a.cos(), a.sin()) * r);
        }
        prev_end = points.last().copied();
    }
    if let Some(from) = prev_end {
        let to = points[0];
        for k in 1..per_side {
            points.push(from.lerp(to, k as f32 / per_side as f32));
        }
    }
    points
}

/// A border whose colors flow around `rect`; brighter and wider with `energy`.
pub(super) fn flowing_border(painter: &Painter, rect: Rect, radius: f32, t: f32, energy: f32) {
    let points = rounded_outline(rect, radius, 10, 18);
    let n = points.len();
    let speed = 0.08 + 0.17 * energy;
    let alpha = 0.32 + 0.6 * energy;
    for k in 0..n {
        let u = k as f32 / n as f32;
        let color = palette_at(&AI_COLORS, u * 2.0 - t * speed);
        let seg = [points[k], points[(k + 1) % n]];
        if energy > 0.0 {
            // Soft outer glow while active.
            painter.line_segment(seg, Stroke::new(5.0, color.gamma_multiply(0.12 * energy)));
        }
        painter.line_segment(seg, Stroke::new(1.4, color.gamma_multiply(alpha)));
    }
}

/// The floating panel: dark glass with the flowing border.
pub(super) fn glass_panel(painter: &Painter, rect: Rect, t: f32, energy: f32) {
    painter.rect_filled(rect, CornerRadius::same(PANEL_RADIUS as u8), GLASS);
    flowing_border(painter, rect.shrink(0.7), PANEL_RADIUS, t, energy);
}

/// A glass card (Claude's answers, the input field).
pub(super) fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(14, 11))
}

/// What the big orb is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OrbState {
    Idle,
    Listening,
    Thinking,
}

/// A Siri-style orb: glowing layered color blobs that swirl, faster while thinking
/// and swelling with the voice while listening.
pub(super) fn big_orb(painter: &Painter, c: Pos2, r: f32, state: OrbState, level: f32, t: f32) {
    let (speed, swell) = match state {
        OrbState::Idle => (0.6, 0.0),
        OrbState::Listening => (1.4, level),
        OrbState::Thinking => (2.4, 0.25 + 0.15 * (t * 5.0).sin().abs()),
    };
    // Outer halo.
    for (k, a) in [(1.55, 0.05), (1.3, 0.08), (1.12, 0.12)] {
        painter.circle_filled(
            c,
            r * (k + 0.25 * swell),
            palette_at(&AI_COLORS, t * 0.05).gamma_multiply(a),
        );
    }
    painter.circle_filled(c, r, Color32::from_rgb(12, 10, 28));
    // Swirling blobs, clipped to the orb by keeping them inside the radius.
    for k in 0..5 {
        let phase = t * speed * (0.7 + 0.15 * k as f32) + k as f32 * TAU / 5.0;
        let wobble = r * (0.22 + 0.18 * swell);
        let center = c + vec2(phase.cos(), (phase * 1.3).sin()) * wobble;
        let blob = r * (0.5 + 0.12 * (t * 1.7 + k as f32).sin() + 0.25 * swell);
        let blob = blob.min(r - center.distance(c));
        if blob > 0.0 {
            painter.circle_filled(
                center,
                blob,
                palette_at(&AI_COLORS, k as f32 / 5.0 + t * 0.07).gamma_multiply(0.42),
            );
        }
    }
    // Bright core and a glassy rim.
    painter.circle_filled(c, r * (0.22 + 0.1 * swell), Color32::from_white_alpha(170));
    painter.circle_stroke(c, r, Stroke::new(1.2, Color32::from_white_alpha(40)));
    if state == OrbState::Thinking {
        super::overlay::spinner(painter, c, r + 5.0, &AI_COLORS, t);
    }
}

/// Makes `rect` drag the window (the panel has no title bar).
pub(super) fn drag_area(ui: &mut Ui, rect: Rect) -> Response {
    let response = ui.interact(
        rect,
        ui.make_persistent_id("dp-drag-window"),
        Sense::click_and_drag(),
    );
    if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    response
}

/// A small grip in the bottom-right corner that resizes the window.
pub(super) fn resize_grip(ui: &mut Ui, panel: Rect) {
    let size = 18.0;
    let rect = Rect::from_min_size(
        panel.right_bottom() - vec2(size + 6.0, size + 6.0),
        vec2(size, size),
    );
    let response = ui.interact(
        rect,
        ui.make_persistent_id("dp-resize-window"),
        Sense::drag(),
    );
    let color = if response.hovered() {
        Color32::from_white_alpha(120)
    } else {
        Color32::from_white_alpha(45)
    };
    for k in 1..=2 {
        let d = k as f32 * 5.0;
        ui.painter().line_segment(
            [
                rect.right_bottom() - vec2(d, 0.0),
                rect.right_bottom() - vec2(0.0, d),
            ],
            Stroke::new(1.2, color),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouthEast);
    }
    if response.drag_started() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::BeginResize(
                egui::viewport::ResizeDirection::SouthEast,
            ));
    }
}

/// A small round "×" button.
pub(super) fn close_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let c = rect.center();
    let bg = if response.hovered() {
        Color32::from_white_alpha(30)
    } else {
        Color32::from_white_alpha(12)
    };
    ui.painter().circle_filled(c, 11.0, bg);
    let s = Stroke::new(1.5, Color32::from_white_alpha(190));
    let d = 3.8;
    ui.painter()
        .line_segment([c + vec2(-d, -d), c + vec2(d, d)], s);
    ui.painter()
        .line_segment([c + vec2(-d, d), c + vec2(d, -d)], s);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A glass suggestion pill with a small sparkle.
pub(super) fn suggestion(ui: &mut Ui, text: &str, t: f32) -> Response {
    let font = egui::FontId::proportional(14.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, super::style::TEXT);
    let pad = vec2(16.0, 11.0);
    let size = vec2(
        galley.size().x + pad.x * 2.0 + 18.0,
        galley.size().y + pad.y * 2.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() {
        Color32::from_rgb(44, 44, 58)
    } else {
        CARD
    };
    painter.rect_filled(rect, CornerRadius::same(20), fill);
    if response.hovered() {
        flowing_border(painter, rect, 20.0, t, 0.6);
    } else {
        painter.rect_stroke(
            rect,
            CornerRadius::same(20),
            Stroke::new(1.0, CARD_STROKE),
            egui::StrokeKind::Inside,
        );
    }
    super::overlay::sparkle(
        painter,
        pos2(rect.left() + pad.x + 4.0, rect.center().y),
        5.5,
        &AI_COLORS,
        t * 0.2,
    );
    painter.galley(
        pos2(
            rect.left() + pad.x + 18.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        super::style::TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_goes_around_the_rect() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0));
        let pts = rounded_outline(rect, 20.0, 6, 8);
        assert!(pts.len() > 40);
        for p in &pts {
            assert!(rect.expand(0.01).contains(*p), "{p:?}");
        }
        // Touches all four sides.
        assert!(pts.iter().any(|p| p.y < 0.5));
        assert!(pts.iter().any(|p| p.y > 99.5));
        assert!(pts.iter().any(|p| p.x < 0.5));
        assert!(pts.iter().any(|p| p.x > 199.5));
    }
}
