//! Screenshot scaling, encoding and coordinate mapping (TRD §5, ADR-007).
//!
//! Claude sees a downscaled screenshot ("model space"). Its coordinates are mapped
//! back to the captured frame ("capture space") and then to the input backend's
//! desktop coordinates.

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use dp_platform::{CoordinateSpace, DesktopPoint, Frame, MonitorInfo};
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};

/// API limits for current models (TRD §3.1): long edge ≤ 2576 px and
/// ≤ 4784 visual tokens of 28×28 px.
const MAX_LONG_EDGE: u32 = 2576;
const MAX_VISUAL_TOKENS: u64 = 4784;

fn visual_tokens(w: u32, h: u32) -> u64 {
    u64::from(w.div_ceil(28)) * u64::from(h.div_ceil(28))
}

/// Model-space size for a frame, never upscaling.
pub fn model_size(frame_w: u32, frame_h: u32, target_long_edge: u32) -> (u32, u32) {
    let long = frame_w.max(frame_h).max(1);
    let limit = target_long_edge.min(MAX_LONG_EDGE).min(long);
    let mut s = f64::from(limit) / f64::from(long);
    loop {
        let w = ((f64::from(frame_w) * s).floor() as u32).max(1);
        let h = ((f64::from(frame_h) * s).floor() as u32).max(1);
        if visual_tokens(w, h) <= MAX_VISUAL_TOKENS {
            return (w, h);
        }
        s *= 0.98;
    }
}

/// Maps between model space and the desktop for one captured monitor.
#[derive(Clone, Debug)]
pub struct ScreenMap {
    pub monitor: MonitorInfo,
    pub frame_w: u32,
    pub frame_h: u32,
    pub model_w: u32,
    pub model_h: u32,
}

impl ScreenMap {
    pub fn new(monitor: MonitorInfo, frame_w: u32, frame_h: u32, target_long_edge: u32) -> Self {
        let (model_w, model_h) = model_size(frame_w, frame_h, target_long_edge);
        Self {
            monitor,
            frame_w,
            frame_h,
            model_w,
            model_h,
        }
    }

    fn sx(&self) -> f64 {
        f64::from(self.model_w) / f64::from(self.frame_w)
    }

    fn sy(&self) -> f64 {
        f64::from(self.model_h) / f64::from(self.frame_h)
    }

    /// Model coordinate → capture-frame pixel (pixel-center mapping, no drift).
    pub fn to_capture(&self, x: i64, y: i64) -> Result<(u32, u32), String> {
        if x < 0 || y < 0 || x >= i64::from(self.model_w) || y >= i64::from(self.model_h) {
            return Err(format!(
                "coordinate ({x}, {y}) is outside the {}x{} screenshot",
                self.model_w, self.model_h
            ));
        }
        let map = |v: i64, s: f64, max: u32| -> u32 {
            let c = ((v as f64 + 0.5) / s - 0.5).round();
            (c.max(0.0) as u32).min(max - 1)
        };
        Ok((
            map(x, self.sx(), self.frame_w),
            map(y, self.sy(), self.frame_h),
        ))
    }

    /// Model coordinate → point for the input backend.
    pub fn to_desktop(
        &self,
        x: i64,
        y: i64,
        space: &CoordinateSpace,
    ) -> Result<DesktopPoint, String> {
        let (cx, cy) = self.to_capture(x, y)?;
        let (ox, oy) = self.monitor.origin_physical;
        let (px, py) = (ox + cx as i32, oy + cy as i32);
        match space {
            CoordinateSpace::DesktopPhysical => Ok(DesktopPoint::Physical { x: px, y: py }),
            CoordinateSpace::DesktopLogical => {
                let s = self.monitor.scale_factor.max(1.0);
                Ok(DesktopPoint::Logical {
                    x: f64::from(px) / s,
                    y: f64::from(py) / s,
                })
            }
            CoordinateSpace::StreamLogical { .. } => {
                Err("stream coordinates are not supported yet".to_owned())
            }
        }
    }

    /// Desktop physical point → model coordinate (for `cursor_position`).
    pub fn to_model(&self, px: i32, py: i32) -> (i64, i64) {
        let (ox, oy) = self.monitor.origin_physical;
        let cx = f64::from(px - ox);
        let cy = f64::from(py - oy);
        (
            ((cx + 0.5) * self.sx() - 0.5).round() as i64,
            ((cy + 0.5) * self.sy() - 0.5).round() as i64,
        )
    }
}

/// An encoded screenshot plus an RGBA copy for the UI preview.
#[derive(Clone, Debug)]
pub struct Shot {
    pub png: Vec<u8>,
    pub rgba: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
}

impl Shot {
    /// `{"type": "image", ...}` content block for the API.
    pub fn image_block(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": base64::engine::general_purpose::STANDARD.encode(&self.png),
            }
        })
    }
}

fn frame_image(frame: &Frame) -> Result<RgbaImage, String> {
    RgbaImage::from_raw(frame.width, frame.height, frame.rgba.to_vec())
        .ok_or_else(|| "frame buffer has the wrong size".to_owned())
}

fn encode(img: &RgbaImage) -> Result<Shot, String> {
    let (w, h) = img.dimensions();
    // RGB is smaller than RGBA; screenshots are opaque anyway.
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut png = Vec::new();
    PngEncoder::new_with_quality(
        Cursor::new(&mut png),
        CompressionType::Fast,
        PngFilter::Adaptive,
    )
    .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
    .map_err(|e| format!("PNG encoding failed: {e}"))?;
    Ok(Shot {
        png,
        rgba: Arc::from(img.as_raw().clone().into_boxed_slice()),
        width: w,
        height: h,
    })
}

/// Downscales a full frame to model size and encodes it.
pub fn screenshot(frame: &Frame, map: &ScreenMap) -> Result<Shot, String> {
    let img = frame_image(frame)?;
    let small = if (map.model_w, map.model_h) == (frame.width, frame.height) {
        img
    } else {
        image::imageops::resize(&img, map.model_w, map.model_h, FilterType::Triangle)
    };
    encode(&small)
}

/// Crops a model-space region from the full-resolution frame and scales it to fit the
/// screenshot size (aspect ratio kept). Coordinates stay in full-screenshot space.
pub fn zoom(frame: &Frame, map: &ScreenMap, region: [i64; 4]) -> Result<Shot, String> {
    let [x0, y0, x1, y1] = region;
    if x1 <= x0 || y1 <= y0 {
        return Err("zoom region must have x1 > x0 and y1 > y0".to_owned());
    }
    let (cx0, cy0) = map.to_capture(x0, y0)?;
    let (cx1, cy1) = map.to_capture((x1 - 1).max(x0), (y1 - 1).max(y0))?;
    let (w, h) = ((cx1 - cx0 + 1).max(1), (cy1 - cy0 + 1).max(1));
    let img = frame_image(frame)?;
    let crop = image::imageops::crop_imm(&img, cx0, cy0, w, h).to_image();
    let fit = (f64::from(map.model_w) / f64::from(w)).min(f64::from(map.model_h) / f64::from(h));
    let (tw, th) = (
        ((f64::from(w) * fit).round() as u32).max(1),
        ((f64::from(h) * fit).round() as u32).max(1),
    );
    let scaled = image::imageops::resize(&crop, tw, th, FilterType::Lanczos3);
    encode(&scaled)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use dp_platform::MonitorId;

    use super::*;

    fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            id: MonitorId("m".into()),
            name: "m".into(),
            origin_physical: (x, y),
            size_physical: (w, h),
            origin_logical: (f64::from(x) / scale, f64::from(y) / scale),
            scale_factor: scale,
            is_primary: true,
        }
    }

    #[test]
    fn model_sizes_respect_limits() {
        assert_eq!(model_size(1920, 1080, 1280), (1280, 720));
        assert_eq!(model_size(1280, 720, 1920), (1280, 720)); // never upscales
        let (w, h) = model_size(3840, 2160, 2576);
        assert!(w <= 2576 && visual_tokens(w, h) <= MAX_VISUAL_TOKENS);
        let (w, h) = model_size(5120, 1440, 1920); // ultra-wide
        assert!(w <= 1920 && h >= 1);
    }

    #[test]
    fn mapping_round_trips_within_one_pixel() {
        for (fw, fh, target) in [
            (1920, 1080, 1280),
            (2560, 1440, 1366),
            (3840, 2160, 1280),
            (1366, 768, 1280),
        ] {
            let map = ScreenMap::new(monitor(-1920, 0, fw, fh, 1.5), fw, fh, target);
            for x in (0..map.model_w).step_by(37) {
                for y in (0..map.model_h).step_by(29) {
                    let p = map
                        .to_desktop(
                            i64::from(x),
                            i64::from(y),
                            &CoordinateSpace::DesktopPhysical,
                        )
                        .unwrap();
                    let DesktopPoint::Physical { x: px, y: py } = p else {
                        unreachable!()
                    };
                    assert!(px >= -1920 && px < -1920 + fw as i32 && py >= 0 && py < fh as i32);
                    let (mx, my) = map.to_model(px, py);
                    assert!((mx - i64::from(x)).abs() <= 1 && (my - i64::from(y)).abs() <= 1);
                }
            }
        }
    }

    #[test]
    fn corners_map_to_monitor_corners() {
        let map = ScreenMap::new(monitor(0, 0, 1920, 1080, 1.0), 1920, 1080, 1280);
        let tl = map
            .to_desktop(0, 0, &CoordinateSpace::DesktopPhysical)
            .unwrap();
        let br = map
            .to_desktop(1279, 719, &CoordinateSpace::DesktopPhysical)
            .unwrap();
        assert_eq!(tl, DesktopPoint::Physical { x: 0, y: 0 });
        assert_eq!(br, DesktopPoint::Physical { x: 1919, y: 1079 });
        assert!(
            map.to_desktop(1280, 0, &CoordinateSpace::DesktopPhysical)
                .is_err()
        );
        assert!(
            map.to_desktop(-1, 0, &CoordinateSpace::DesktopPhysical)
                .is_err()
        );
    }

    #[test]
    fn screenshot_and_zoom_encode_png() {
        let (w, h) = (400_u32, 300_u32);
        let mut rgba = vec![0_u8; (w * h * 4) as usize];
        for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *px = [(i % 251) as u8, 80, 160, 255];
        }
        let frame = Frame {
            monitor: MonitorId("m".into()),
            width: w,
            height: h,
            rgba: Arc::from(rgba.into_boxed_slice()),
            captured_at: Instant::now(),
        };
        let map = ScreenMap::new(monitor(0, 0, w, h, 1.0), w, h, 200);
        let shot = screenshot(&frame, &map).unwrap();
        assert_eq!((shot.width, shot.height), (200, 150));
        assert_eq!(&shot.png[1..4], b"PNG");
        let z = zoom(&frame, &map, [0, 0, 50, 50]).unwrap();
        assert_eq!((z.width, z.height), (150, 150));
        assert!(zoom(&frame, &map, [10, 10, 5, 20]).is_err());
        let block = shot.image_block();
        assert_eq!(block["source"]["media_type"], "image/png");
    }
}
