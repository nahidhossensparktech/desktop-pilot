//! Colors, the logo and drawn icons (the Siri-style panel itself is in `siri.rs`).
//! Icons are painted (not font glyphs) because the default fonts lack most symbols.

use eframe::egui::{
    self, Color32, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, vec2,
};

pub(super) const SURFACE: Color32 = Color32::from_rgb(32, 33, 37);
pub(super) const SURFACE_HOVER: Color32 = Color32::from_rgb(45, 47, 53);
pub(super) const TEXT: Color32 = Color32::from_rgb(227, 227, 227);
pub(super) const TEXT_DIM: Color32 = Color32::from_rgb(150, 154, 162);
pub(super) const ACCENT: Color32 = Color32::from_rgb(91, 141, 239);
pub(super) const ACCENT_2: Color32 = Color32::from_rgb(160, 120, 245);
pub(super) const RED: Color32 = Color32::from_rgb(214, 60, 60);

/// Fills a convex polygon with a vertical two-color gradient.
fn gradient_polygon(painter: &Painter, points: &[Pos2], top: Color32, bottom: Color32) {
    let (min_y, max_y) = points
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let mut mesh = Mesh::default();
    for p in points {
        let t = if max_y > min_y {
            (p.y - min_y) / (max_y - min_y)
        } else {
            0.0
        };
        mesh.colored_vertex(*p, lerp_color(top, bottom, t));
    }
    for i in 1..points.len() as u32 - 1 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(Shape::mesh(mesh));
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Desktop Pilot's mark: a mouse pointer with a small spark.
pub(super) fn paint_logo(painter: &Painter, center: Pos2, size: f32) {
    let s = size / 26.0;
    let o = center - vec2(8.0 * s, 13.0 * s);
    let p = |x: f32, y: f32| o + vec2(x * s, y * s);
    // Arrow head and tail as two convex shapes.
    gradient_polygon(
        painter,
        &[p(0.0, 0.0), p(17.0, 15.5), p(9.5, 15.5), p(0.0, 22.0)],
        ACCENT,
        ACCENT_2,
    );
    gradient_polygon(
        painter,
        &[p(6.5, 13.0), p(10.5, 11.5), p(15.0, 22.5), p(11.0, 24.0)],
        ACCENT_2,
        ACCENT_2,
    );
    // Spark.
    let spark = p(20.0, 3.0);
    let r = 3.2 * s;
    painter.add(Shape::convex_polygon(
        vec![
            spark + vec2(0.0, -r),
            spark + vec2(r * 0.35, -r * 0.35),
            spark + vec2(r, 0.0),
            spark + vec2(r * 0.35, r * 0.35),
            spark + vec2(0.0, r),
            spark + vec2(-r * 0.35, r * 0.35),
            spark + vec2(-r, 0.0),
            spark + vec2(-r * 0.35, -r * 0.35),
        ],
        Color32::from_rgb(120, 200, 255),
        Stroke::NONE,
    ));
}

/// Icons drawn into a square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Plus,
    Send,
    Stop,
    Gear,
    Pin,
    Back,
    Mic,
}

fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width() * 0.3;
    let stroke = Stroke::new(1.8, color);
    match icon {
        Icon::Plus => {
            painter.line_segment([c - vec2(r, 0.0), c + vec2(r, 0.0)], stroke);
            painter.line_segment([c - vec2(0.0, r), c + vec2(0.0, r)], stroke);
        }
        Icon::Send => {
            let s = Stroke::new(2.2, color);
            painter.line_segment([c + vec2(0.0, r), c - vec2(0.0, r)], s);
            painter.line_segment([c - vec2(0.0, r), c + vec2(-r * 0.75, -r * 0.2)], s);
            painter.line_segment([c - vec2(0.0, r), c + vec2(r * 0.75, -r * 0.2)], s);
        }
        Icon::Mic => {
            // Capsule body, a U-shaped holder and a short stand.
            let body = Rect::from_center_size(c - vec2(0.0, r * 0.25), vec2(r * 0.8, r * 1.3));
            painter.rect_filled(body, r * 0.4, color);
            let holder: Vec<Pos2> = (0..=12)
                .map(|k| {
                    let a = std::f32::consts::PI * k as f32 / 12.0;
                    c + vec2(-a.cos() * r * 0.75, a.sin() * r * 0.75 - r * 0.05)
                })
                .collect();
            painter.add(Shape::line(holder, Stroke::new(1.8, color)));
            painter.line_segment([c + vec2(0.0, r * 0.7), c + vec2(0.0, r * 1.05)], stroke);
        }
        Icon::Stop => {
            let h = r * 0.75;
            painter.rect_filled(
                Rect::from_center_size(c, vec2(h * 2.0, h * 2.0)),
                2.0,
                color,
            );
        }
        Icon::Gear => {
            painter.circle_stroke(c, r * 0.62, stroke);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment(
                    [c + d * r * 0.62, c + d * r * 1.05],
                    Stroke::new(2.4, color),
                );
            }
            painter.circle_filled(c, r * 0.22, color);
        }
        Icon::Pin => {
            // A window with an "on top" bar.
            let b = Rect::from_center_size(c, vec2(r * 2.0, r * 1.7));
            painter.rect_stroke(b, 2.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment(
                [
                    b.left_top() + vec2(0.0, r * 0.5),
                    b.right_top() + vec2(0.0, r * 0.5),
                ],
                stroke,
            );
        }
        Icon::Back => {
            let s = Stroke::new(2.0, color);
            painter.line_segment([c + vec2(r * 0.4, -r), c + vec2(-r * 0.5, 0.0)], s);
            painter.line_segment([c + vec2(-r * 0.5, 0.0), c + vec2(r * 0.4, r)], s);
        }
    }
}

/// A round icon button. `fill`: background circle color (None = only on hover).
pub(super) fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    size: f32,
    fill: Option<Color32>,
    color: Color32,
    enabled: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    let painter = ui.painter_at(rect.expand(1.0));
    let bg = match fill {
        Some(f) if enabled => Some(if response.hovered() {
            lerp_color(f, Color32::WHITE, 0.12)
        } else {
            f
        }),
        Some(f) => Some(f.gamma_multiply(0.4)),
        None if enabled && response.hovered() => Some(SURFACE_HOVER),
        None => None,
    };
    if let Some(bg) = bg {
        painter.circle_filled(rect.center(), size / 2.0, bg);
    }
    paint_icon(
        &painter,
        rect,
        icon,
        if enabled {
            color
        } else {
            color.gamma_multiply(0.45)
        },
    );
    response
}
