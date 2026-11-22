//! The application icon (Dock, app switcher, taskbar): the Desktop Pilot pointer with
//! its spark, in the logo's blue → violet gradient, on a dark rounded square, drawn
//! in code like the rest of the app's artwork (no image files to ship).
//!
//! Shapes match `style::paint_logo` (the logo in the window) and the menu-bar icon.

use std::sync::Arc;

use eframe::egui::IconData;

/// Icon size in pixels (the Dock shows up to 256 px on Retina; 512 leaves headroom).
const SIZE: usize = 512;
/// Samples per pixel along each axis (anti-aliasing).
const SS: usize = 3;
/// Rounded-square margin: Apple's icon grid keeps the shape at about 80 % of the
/// canvas.
const MARGIN: f32 = 50.0;

type Rgb = [f32; 3];

const BG_TOP: Rgb = [38.0, 30.0, 86.0];
const BG_BOTTOM: Rgb = [11.0, 11.0, 26.0];
const GLOW: Rgb = [124.0, 77.0, 255.0];
const POINTER_TOP: Rgb = [96.0, 150.0, 255.0];
const POINTER_BOTTOM: Rgb = [170.0, 118.0, 255.0];
const SPARK: Rgb = [130.0, 210.0, 255.0];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((xi, yi), (xj, yj)) = (poly[i], poly[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            c = !c;
        }
        j = i;
    }
    c
}

/// Inside the rounded square (a superellipse, like macOS icon corners).
fn in_squircle(x: f32, y: f32) -> bool {
    let half = (SIZE as f32 - 2.0 * MARGIN) / 2.0;
    let c = SIZE as f32 / 2.0;
    let (u, v) = (((x - c) / half).abs(), ((y - c) / half).abs());
    u.powi(5) + v.powi(5) <= 1.0
}

/// One sample: `None` outside the rounded square.
fn sample(x: f32, y: f32, shapes: &Shapes) -> Option<Rgb> {
    if !in_squircle(x, y) {
        return None;
    }
    let n = SIZE as f32;
    // Background: vertical gradient with a soft violet glow behind the pointer.
    let mut color = mix(BG_TOP, BG_BOTTOM, (y - MARGIN) / (n - 2.0 * MARGIN));
    let d = ((x - n * 0.47).powi(2) + (y - n * 0.45).powi(2)).sqrt() / (n * 0.42);
    color = mix(color, GLOW, 0.35 * (1.0 - d).max(0.0).powi(2));
    if inside(&shapes.head, x, y) || inside(&shapes.tail, x, y) {
        let t = (y - shapes.top) / (shapes.bottom - shapes.top);
        color = mix(POINTER_TOP, POINTER_BOTTOM, t);
    } else if inside(&shapes.spark, x, y) {
        color = SPARK;
    }
    Some(color)
}

struct Shapes {
    head: Vec<(f32, f32)>,
    tail: Vec<(f32, f32)>,
    spark: Vec<(f32, f32)>,
    top: f32,
    bottom: f32,
}

fn shapes() -> Shapes {
    // Logo units as in `style::paint_logo` (pointer 17 × 24, spark at (20, 3)),
    // scaled and centered on the canvas.
    let scale = 11.5;
    // Centered by visual weight (the pointer), not by the bounding box: the small
    // spark would otherwise pull the pointer too far left.
    let (ox, oy) = (
        SIZE as f32 / 2.0 - 11.0 * scale,
        SIZE as f32 / 2.0 - 12.0 * scale,
    );
    let map = |pts: &[(f32, f32)]| -> Vec<(f32, f32)> {
        pts.iter()
            .map(|(x, y)| (ox + x * scale, oy + y * scale))
            .collect()
    };
    let (cx, cy, r) = (ox + 20.5 * scale, oy + 3.0 * scale, 4.2 * scale);
    let spark = (0..8)
        .map(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
            let rr = if k % 2 == 0 { r } else { r * 0.3 };
            (cx + a.cos() * rr, cy + a.sin() * rr)
        })
        .collect();
    Shapes {
        head: map(&[(0.0, 0.0), (17.0, 15.5), (9.5, 15.5), (0.0, 22.0)]),
        tail: map(&[(6.5, 13.0), (10.5, 11.5), (15.0, 22.5), (11.0, 24.0)]),
        spark,
        top: oy,
        bottom: oy + 24.0 * scale,
    }
}

/// Renders the icon as straight-alpha RGBA, `SIZE` × `SIZE`.
pub(super) fn rgba() -> Vec<u8> {
    rgba_sized(SIZE)
}

/// Renders the icon at `size` × `size` (the artwork is designed at `SIZE` and scaled),
/// e.g. for the macOS `.icns` sizes 16–1024.
pub(super) fn rgba_sized(size: usize) -> Vec<u8> {
    let shapes = shapes();
    let k = SIZE as f32 / size as f32;
    let mut out = vec![0_u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let (mut sum, mut hits) = ([0.0_f32; 3], 0_usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * k;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * k;
                    if let Some(c) = sample(x, y, &shapes) {
                        sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let i = (py * size + px) * 4;
                let h = hits as f32;
                out[i] = (sum[0] / h).round() as u8;
                out[i + 1] = (sum[1] / h).round() as u8;
                out[i + 2] = (sum[2] / h).round() as u8;
                out[i + 3] = (hits * 255 / (SS * SS)) as u8;
            }
        }
    }
    out
}

/// The icon for the window / Dock.
pub(super) fn icon() -> Arc<IconData> {
    Arc::new(IconData {
        rgba: rgba(),
        width: SIZE as u32,
        height: SIZE as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_logo() {
        let px = rgba();
        assert_eq!(px.len(), SIZE * SIZE * 4);
        let at = |x: usize, y: usize| &px[(y * SIZE + x) * 4..(y * SIZE + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "corner is transparent");
        assert_eq!(
            at(SIZE / 2, SIZE - 70)[3],
            255,
            "inside the square is opaque"
        );
        // A point inside the pointer head is the bright blue/violet, not background.
        let s = shapes();
        let (hx, hy) = (s.head[0].0 + 30.0, s.head[0].1 + 60.0);
        let p = at(hx as usize, hy as usize);
        assert!(p[2] > 200 && p[3] == 255, "{p:?}");
        // Optional: write a PNG to look at (DP_ICON_PNG=/path/icon.png).
        if let Ok(path) = std::env::var("DP_ICON_PNG") {
            image::save_buffer(
                path,
                &px,
                SIZE as u32,
                SIZE as u32,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        }
    }

    /// Writes the macOS iconset PNGs (16–1024 px) when `DP_ICONSET_DIR` is set; used by
    /// `packaging/macos/build-dmg.sh` to make `AppIcon.icns` from the same artwork.
    #[test]
    fn write_iconset_when_asked() {
        let Ok(dir) = std::env::var("DP_ICONSET_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for base in [16_usize, 32, 128, 256, 512] {
            for (scale, suffix) in [(1, ""), (2, "@2x")] {
                let size = base * scale;
                let path = format!("{dir}/icon_{base}x{base}{suffix}.png");
                image::save_buffer(
                    path,
                    &rgba_sized(size),
                    size as u32,
                    size as u32,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn small_sizes_render() {
        let px = rgba_sized(16);
        assert_eq!(px.len(), 16 * 16 * 4);
        assert!(px.chunks(4).any(|p| p[3] == 255));
    }
}
