//! The overlay shown while a task controls the computer, styled so it reads as "the AI
//! is working": a faint tint over the primary screen, a multicolor glow that flows
//! around the screen edge, and a dark glass pill at the top center with a sparkle in
//! a spinning ring, shimmering "AI is working" text and the current action.
//!
//! It is a separate borderless, transparent window that lets every click through. The
//! platform raises it over menu bars and docks (`ScreenCapture::prepare_overlay`), and
//! screen capture leaves it out by its title ([`dp_platform::OVERLAY_WINDOW_TITLE`]),
//! so Claude sees the real screen.

use std::f32::consts::TAU;
use std::sync::Arc;

use dp_platform::ScreenCapture;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shadow, Shape, Stroke, Ui,
    Vec2, ViewportBuilder, ViewportId, pos2, text::LayoutJob, vec2,
};

/// Whole-screen tint (premultiplied): a faint rose wash, ~4 % strong.
const TINT: Color32 = Color32::from_rgba_premultiplied(10, 2, 5, 10);
/// Width of the glow along the screen edge (it breathes between 60 % and 100 %).
const EDGE_GLOW: f32 = 26.0;
/// Segments per screen side for the flowing edge gradient.
const EDGE_STEPS: usize = 48;
/// Gap above the pill: clear of the macOS menu bar and camera notch.
const BANNER_TOP: f32 = 44.0;
const BANNER_H: f32 = 38.0;
const GLASS: Color32 = Color32::from_rgba_premultiplied(12, 12, 18, 222);
const TEXT: Color32 = Color32::from_rgb(246, 246, 250);
const TEXT_DIM: Color32 = Color32::from_rgb(166, 168, 182);
/// Base color of the shimmering title, between sweeps.
const SHIMMER_BASE: Color32 = Color32::from_rgb(196, 184, 255);
/// Longest action text shown in the pill.
const MAX_ACTION_CHARS: usize = 56;

/// The "AI" palette, looped: rose, orange, violet, blue.
pub(super) const AI_COLORS: [Color32; 4] = [
    Color32::from_rgb(255, 77, 141),
    Color32::from_rgb(255, 154, 61),
    Color32::from_rgb(157, 92, 255),
    Color32::from_rgb(58, 141, 255),
];
/// Paused: warm, calm ambers.
const PAUSED_COLORS: [Color32; 4] = [
    Color32::from_rgb(245, 170, 50),
    Color32::from_rgb(240, 130, 50),
    Color32::from_rgb(245, 190, 80),
    Color32::from_rgb(235, 140, 60),
];

/// What the overlay shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// A task is controlling the computer.
    Working,
    Paused,
    /// The voice assistant heard "Hey Mouse" and listens for the command.
    Listening,
    /// The voice assistant is transcribing the command.
    Understanding,
}

impl Mode {
    /// Animated (flowing glow, spinner, shimmer) vs. calm.
    fn animated(self) -> bool {
        self != Self::Paused
    }
}

/// One frame of the overlay.
#[derive(Clone, Debug)]
pub(super) struct View {
    pub(super) mode: Mode,
    /// Text after the title; empty = a default for the mode.
    pub(super) detail: String,
    /// Microphone level 0–1 (Listening: drives the orb and the glow).
    pub(super) level: f32,
}

/// Shows the overlay over `screen` (in points) for this frame. Call every frame while
/// it should show; stop calling it and the window closes.
pub(super) fn show(
    ctx: &egui::Context,
    screen: Rect,
    view: &View,
    capture: &Arc<dyn ScreenCapture>,
) {
    let builder = ViewportBuilder::default()
        .with_title(dp_platform::OVERLAY_WINDOW_TITLE)
        .with_position(screen.min)
        .with_inner_size(screen.size())
        .with_decorations(false)
        .with_transparent(true)
        .with_mouse_passthrough(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_taskbar(false)
        .with_active(false)
        .with_resizable(false)
        .with_has_shadow(false)
        .with_clamp_size_to_monitor_size(false);
    ctx.show_viewport_immediate(
        ViewportId::from_hash_of("dp-monitoring-overlay"),
        builder,
        |ui: &mut Ui, _class| {
            let rect = ui.max_rect();
            let t = ui.input(|i| i.time) as f32;
            paint(ui.painter(), rect, view, t);
            capture.prepare_overlay();
            // ~30 fps keeps the flow smooth without much GPU work.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        },
    );
}

fn paint(painter: &Painter, rect: Rect, view: &View, t: f32) {
    let palette = match view.mode {
        Mode::Paused => &PAUSED_COLORS,
        _ => &AI_COLORS,
    };
    // Paused: the glow stands still and only breathes. Listening: it flows faster and
    // swells with your voice.
    let flow = match view.mode {
        Mode::Paused => 0.0,
        Mode::Listening => t * 1.8,
        _ => t,
    };
    let swell = if view.mode == Mode::Listening {
        1.0 + 1.2 * view.level
    } else {
        1.0
    };
    // No tint while only listening: nothing is being controlled yet.
    if !matches!(view.mode, Mode::Listening | Mode::Understanding) {
        painter.rect_filled(rect, 0.0, TINT);
    }
    paint_edge(painter, rect, palette, flow, t, swell);
    paint_pill(painter, rect, view, palette, t);
}

fn lerp(a: Color32, b: Color32, f: f32) -> Color32 {
    let f = f.clamp(0.0, 1.0);
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round() as u8;
    Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// The palette as a smooth loop: `u` in any range, period 1.
pub(super) fn palette_at(palette: &[Color32; 4], u: f32) -> Color32 {
    let x = u.rem_euclid(1.0) * 4.0;
    let i = x.floor() as usize % 4;
    lerp(palette[i], palette[(i + 1) % 4], x.fract())
}

/// A glow that flows around the screen edge: color moves along the perimeter, and the
/// band swells and thins in slow waves.
fn paint_edge(
    painter: &Painter,
    rect: Rect,
    palette: &[Color32; 4],
    flow: f32,
    t: f32,
    swell: f32,
) {
    let perimeter = 2.0 * (rect.width() + rect.height());
    // Corners clockwise, with the inward direction of each side.
    let sides: [(Pos2, Pos2, Vec2); 4] = [
        (rect.left_top(), rect.right_top(), vec2(0.0, 1.0)),
        (rect.right_top(), rect.right_bottom(), vec2(-1.0, 0.0)),
        (rect.right_bottom(), rect.left_bottom(), vec2(0.0, -1.0)),
        (rect.left_bottom(), rect.left_top(), vec2(1.0, 0.0)),
    ];
    let breathe = (0.85 + 0.15 * (t * 1.6).sin()) * swell;
    let mut glow = Mesh::default();
    let mut line = Mesh::default();
    let mut walked = 0.0;
    for (a, b, inward) in sides {
        let len = a.distance(b);
        for s in 0..EDGE_STEPS {
            let (f0, f1) = (
                s as f32 / EDGE_STEPS as f32,
                (s + 1) as f32 / EDGE_STEPS as f32,
            );
            let (p0, p1) = (a.lerp(b, f0), a.lerp(b, f1));
            let (u0, u1) = (
                (walked + len * f0) / perimeter,
                (walked + len * f1) / perimeter,
            );
            let width =
                |u: f32| EDGE_GLOW * breathe * (0.8 + 0.2 * (TAU * (u * 3.0 - flow * 0.15)).sin());
            let color = |u: f32, alpha: f32| {
                palette_at(palette, u * 2.0 - flow * 0.12).gamma_multiply(alpha)
            };
            // Soft glow band: colored at the edge, transparent inside.
            let i = glow.vertices.len() as u32;
            glow.colored_vertex(p0, color(u0, 0.42));
            glow.colored_vertex(p1, color(u1, 0.42));
            glow.colored_vertex(p1 + inward * width(u1), Color32::TRANSPARENT);
            glow.colored_vertex(p0 + inward * width(u0), Color32::TRANSPARENT);
            glow.add_triangle(i, i + 1, i + 2);
            glow.add_triangle(i, i + 2, i + 3);
            // Crisp 2.5 px line right at the edge.
            let i = line.vertices.len() as u32;
            line.colored_vertex(p0, color(u0, 0.85));
            line.colored_vertex(p1, color(u1, 0.85));
            line.colored_vertex(p1 + inward * 2.5, color(u1, 0.85));
            line.colored_vertex(p0 + inward * 2.5, color(u0, 0.85));
            line.add_triangle(i, i + 1, i + 2);
            line.add_triangle(i, i + 2, i + 3);
        }
        walked += len;
    }
    painter.add(Shape::mesh(glow));
    painter.add(Shape::mesh(line));
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_ACTION_CHARS {
        return text.to_owned();
    }
    let mut s: String = text.chars().take(MAX_ACTION_CHARS - 1).collect();
    s.push('…');
    s
}

/// Text with a bright highlight sweeping left to right every ~1.8 s.
pub(super) fn shimmer_text(text: &str, size: f32, t: f32) -> LayoutJob {
    let n = text.chars().count() as f32;
    let head = (t / 1.8).fract() * (n + 8.0) - 4.0;
    let mut job = LayoutJob::default();
    for (i, ch) in text.chars().enumerate() {
        let d = i as f32 - head;
        let glow = (-d * d / 6.0).exp();
        job.append(
            &ch.to_string(),
            0.0,
            egui::TextFormat {
                font_id: FontId::proportional(size),
                color: lerp(SHIMMER_BASE, Color32::WHITE, glow),
                ..Default::default()
            },
        );
    }
    job
}

/// A four-point sparkle (✦), filled with a gradient from the palette.
pub(super) fn sparkle(painter: &Painter, c: Pos2, r: f32, palette: &[Color32; 4], phase: f32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, Color32::WHITE);
    for k in 0..8 {
        let angle = k as f32 * TAU / 8.0 - TAU / 4.0;
        let radius = if k % 2 == 0 { r } else { r * 0.3 };
        let p = c + vec2(angle.cos(), angle.sin()) * radius;
        mesh.colored_vertex(p, palette_at(palette, phase + k as f32 / 8.0));
    }
    for k in 1..=8_u32 {
        mesh.add_triangle(0, k, if k == 8 { 1 } else { k + 1 });
    }
    painter.add(Shape::mesh(mesh));
}

/// A spinning comet-tail ring in palette colors.
pub(super) fn spinner(painter: &Painter, c: Pos2, r: f32, palette: &[Color32; 4], t: f32) {
    let segments = 28;
    let sweep = TAU * 0.75;
    let start = t * 4.2;
    for k in 0..segments {
        let f0 = k as f32 / segments as f32;
        let f1 = (k + 1) as f32 / segments as f32;
        let (a0, a1) = (start + f0 * sweep, start + f1 * sweep);
        let p0 = c + vec2(a0.cos(), a0.sin()) * r;
        let p1 = c + vec2(a1.cos(), a1.sin()) * r;
        let color = palette_at(palette, f0 + t * 0.2).gamma_multiply(0.15 + 0.85 * f1);
        painter.line_segment([p0, p1], Stroke::new(2.0, color));
    }
}

/// A Siri-style orb: three colored blobs circling the center, growing with the voice.
fn orb(painter: &Painter, c: Pos2, r: f32, palette: &[Color32; 4], level: f32, t: f32) {
    painter.circle_filled(c, r, Color32::from_rgb(20, 16, 40));
    for k in 0..3 {
        let phase = t * (1.6 + k as f32 * 0.5) + k as f32 * TAU / 3.0;
        let wobble = r * (0.18 + 0.25 * level);
        let center = c + vec2(phase.cos(), phase.sin()) * wobble;
        let blob = r * (0.55 + 0.35 * level + 0.08 * (t * 4.0 + k as f32).sin());
        painter.circle_filled(
            center,
            blob,
            palette_at(palette, k as f32 / 3.0 + t * 0.1).gamma_multiply(0.55),
        );
    }
    painter.circle_filled(c, r * 0.28, Color32::from_white_alpha(200));
}

fn paint_pill(painter: &Painter, screen: Rect, view: &View, palette: &[Color32; 4], t: f32) {
    let mode = view.mode;
    let title = match mode {
        Mode::Working => "AI is working",
        Mode::Paused => "Paused",
        Mode::Listening => "Listening",
        Mode::Understanding => "Understanding",
    };
    let detail = if view.detail.is_empty() {
        match mode {
            Mode::Paused => "Press Resume to continue",
            Mode::Working => "Looking at your screen",
            Mode::Listening => "Say your command in Bangla or English",
            Mode::Understanding => "Turning your voice into text",
        }
        .to_owned()
    } else {
        shorten(&view.detail)
    };
    let title_job = if mode.animated() {
        shimmer_text(title, 14.5, t)
    } else {
        LayoutJob::simple_singleline(title.to_owned(), FontId::proportional(14.5), TEXT)
    };
    let title_galley = painter.layout_job(title_job);
    let detail_galley = painter.layout_no_wrap(detail, FontId::proportional(13.0), TEXT_DIM);

    // [pad][icon][gap][title][dots][gap][divider][gap][detail][pad]
    let (pad, icon, gap, dots_w) = (12.0, 26.0, 10.0, 18.0);
    let width = pad
        + icon
        + gap
        + title_galley.size().x
        + dots_w
        + gap
        + 1.0
        + gap
        + detail_galley.size().x
        + pad
        + 4.0;
    let pill = Rect::from_center_size(
        pos2(
            screen.center().x,
            screen.top() + BANNER_TOP + BANNER_H / 2.0,
        ),
        vec2(width, BANNER_H),
    );
    let radius = CornerRadius::same((BANNER_H / 2.0) as u8);

    // Colored glow behind the pill that drifts through the palette, then the glass.
    let glow = Shadow {
        offset: [0, 4],
        blur: 30,
        spread: 2,
        color: palette_at(palette, t * 0.15).gamma_multiply(0.55),
    };
    painter.add(glow.as_shape(pill, radius));
    painter.rect_filled(pill, radius, GLASS);
    paint_pill_border(painter, pill, palette, t);

    // Sparkle in a spinning ring.
    let cy = pill.center().y;
    let center = pos2(pill.left() + pad + icon / 2.0, cy);
    let twinkle = 1.0 + 0.12 * (t * 3.0).sin();
    match mode {
        Mode::Listening => orb(painter, center, icon / 2.0, palette, view.level, t),
        Mode::Paused => {
            painter.circle_stroke(
                center,
                icon / 2.0,
                Stroke::new(2.0, palette[0].gamma_multiply(0.7)),
            );
            sparkle(painter, center, 7.5, palette, 0.0);
        }
        Mode::Working | Mode::Understanding => {
            spinner(painter, center, icon / 2.0, palette, t);
            sparkle(painter, center, 7.5 * twinkle, palette, t * 0.25);
        }
    }

    let mut x = pill.left() + pad + icon + gap;
    let title_w = title_galley.size().x;
    painter.galley(
        pos2(x, cy - title_galley.size().y / 2.0),
        title_galley,
        TEXT,
    );
    x += title_w;
    // Three typing dots that bounce in turn.
    if mode.animated() {
        for k in 0..3 {
            let phase = t * 6.0 - k as f32 * 0.9;
            let lift = phase.sin().max(0.0) * 2.5;
            let alpha = 0.45 + 0.55 * phase.sin().max(0.0);
            painter.circle_filled(
                pos2(x + 4.0 + k as f32 * 5.0, cy + 3.5 - lift),
                1.6,
                SHIMMER_BASE.gamma_multiply(alpha),
            );
        }
    }
    x += dots_w + gap;
    painter.line_segment(
        [pos2(x, cy - 9.0), pos2(x, cy + 9.0)],
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
    x += 1.0 + gap;
    painter.galley(
        pos2(x, cy - detail_galley.size().y / 2.0),
        detail_galley,
        TEXT_DIM,
    );
}

/// A thin border whose colors rotate around the pill.
fn paint_pill_border(painter: &Painter, pill: Rect, palette: &[Color32; 4], t: f32) {
    let r = pill.height() / 2.0;
    let (cl, cr) = (
        pos2(pill.left() + r, pill.center().y),
        pos2(pill.right() - r, pill.center().y),
    );
    // Points around the stadium shape, clockwise from the top-left.
    let mut points = Vec::with_capacity(96);
    let straight = 24;
    for k in 0..=straight {
        points.push(pos2(
            cl.x + (cr.x - cl.x) * k as f32 / straight as f32,
            pill.top(),
        ));
    }
    let arc = 20;
    for k in 1..arc {
        let a = -TAU / 4.0 + TAU / 2.0 * k as f32 / arc as f32;
        points.push(cr + vec2(a.cos(), a.sin()) * r);
    }
    for k in 0..=straight {
        points.push(pos2(
            cr.x - (cr.x - cl.x) * k as f32 / straight as f32,
            pill.bottom(),
        ));
    }
    for k in 1..arc {
        let a = TAU / 4.0 + TAU / 2.0 * k as f32 / arc as f32;
        points.push(cl + vec2(a.cos(), a.sin()) * r);
    }
    let n = points.len();
    for k in 0..n {
        let u = k as f32 / n as f32;
        let color = palette_at(palette, u - t * 0.25).gamma_multiply(0.8);
        let inset = |p: Pos2| p + (pill.center() - p).normalized() * 0.5;
        painter.line_segment(
            [inset(points[k]), inset(points[(k + 1) % n])],
            Stroke::new(1.2, color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_actions_are_shortened() {
        assert_eq!(shorten("Left-click at (1, 2)"), "Left-click at (1, 2)");
        let s = shorten(&"x".repeat(100));
        assert_eq!(s.chars().count(), MAX_ACTION_CHARS);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn palette_loops_smoothly() {
        assert_eq!(palette_at(&AI_COLORS, 0.0), AI_COLORS[0]);
        assert_eq!(palette_at(&AI_COLORS, 1.0), AI_COLORS[0]);
        assert_eq!(palette_at(&AI_COLORS, 0.25), AI_COLORS[1]);
        assert_eq!(palette_at(&AI_COLORS, -0.25), AI_COLORS[3]);
    }
}
