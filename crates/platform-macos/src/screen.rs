//! Monitor enumeration (CoreGraphics) and screen capture (ScreenCaptureKit, TRD TR-M3).
//!
//! Captures use `SCScreenshotManager` (macOS 14+) at the display's native pixel size.
//! Needs the Screen Recording permission; without it we trigger the system prompt once
//! and return `PermissionDenied`. The control bar stays visible on purpose, as on
//! Windows, so Claude can see and avoid it; only the "Monitoring" overlay is left out.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use block2::RcBlock;
use dp_platform::{
    Capability, ExclusionSupport, Frame, MonitorId, MonitorInfo, NativeWindowHandle,
    PermissionKind, PlatformError, ScreenCapture,
};
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGDirectDisplayID, CGDisplayBounds,
    CGDisplayCopyDisplayMode, CGDisplayMode, CGError, CGGetActiveDisplayList, CGImage,
    CGImageAlphaInfo, CGMainDisplayID, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};

/// How long we wait for ScreenCaptureKit before giving up.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DISPLAYS: usize = 16;

#[derive(Debug, Default)]
pub(crate) struct MacScreen {
    /// The system prompt is shown once per run; later calls just report the error.
    prompted: AtomicBool,
}

/// macOS has no physical-pixel desktop, so we build one: every display's origin (in
/// points) is scaled by the largest backing scale. Each display then spans
/// `origin_physical + size_physical` without overlapping its neighbours, even when a
/// Retina panel sits next to a 1× monitor.
fn assign_physical_origins(monitors: &mut [MonitorInfo]) {
    let max_scale = monitors
        .iter()
        .map(|m| m.scale_factor)
        .fold(1.0_f64, f64::max);
    for m in monitors {
        m.origin_physical = (
            (m.origin_logical.0 * max_scale).round() as i32,
            (m.origin_logical.1 * max_scale).round() as i32,
        );
    }
}

pub(crate) fn list_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
    let main = CGMainDisplayID();
    let mut ids: [CGDirectDisplayID; MAX_DISPLAYS] = [0; MAX_DISPLAYS];
    let mut count = 0_u32;
    // SAFETY: `ids` holds MAX_DISPLAYS entries and `count` is a valid out-pointer.
    let err = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count) };
    if err != CGError::Success {
        return Err(PlatformError::Os(format!(
            "CGGetActiveDisplayList failed ({err:?})"
        )));
    }

    let mut monitors = Vec::with_capacity(count as usize);
    for &id in ids.iter().take(count as usize) {
        let bounds = CGDisplayBounds(id); // global points, origin top-left of main display
        let mode = CGDisplayCopyDisplayMode(id);
        let pixel_w = CGDisplayMode::pixel_width(mode.as_deref());
        let pixel_h = CGDisplayMode::pixel_height(mode.as_deref());
        if pixel_w == 0 || pixel_h == 0 || bounds.size.width <= 0.0 {
            continue;
        }
        let scale = pixel_w as f64 / bounds.size.width;
        monitors.push(MonitorInfo {
            id: MonitorId(id.to_string()),
            name: if id == main {
                format!("Main display ({id})")
            } else {
                format!("Display {id}")
            },
            origin_physical: (0, 0), // set below, once every scale is known
            size_physical: (
                u32::try_from(pixel_w).unwrap_or(u32::MAX),
                u32::try_from(pixel_h).unwrap_or(u32::MAX),
            ),
            origin_logical: (bounds.origin.x, bounds.origin.y),
            scale_factor: scale,
            is_primary: id == main,
        });
    }
    assign_physical_origins(&mut monitors);
    // Primary first, then left-to-right, so "monitor 1" is predictable.
    monitors.sort_by(|a, b| {
        (!a.is_primary, a.origin_logical.0, a.origin_logical.1)
            .partial_cmp(&(!b.is_primary, b.origin_logical.0, b.origin_logical.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(monitors)
}

fn ns_error_text(error: *mut NSError) -> String {
    // SAFETY: ScreenCaptureKit passes either null or a valid NSError for the duration of
    // the completion handler, which is where we read it.
    match unsafe { error.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => "unknown error".to_owned(),
    }
}

/// Our own "Monitoring" overlay windows, which screenshots leave out (the control bar
/// stays visible on purpose).
fn overlay_windows(content: &SCShareableContent) -> Retained<NSArray<SCWindow>> {
    let pid = std::process::id() as i32;
    // SAFETY: plain property reads on live ScreenCaptureKit objects.
    let ours: Vec<Retained<SCWindow>> = unsafe {
        content
            .windows()
            .iter()
            .filter(|w| {
                w.owningApplication().is_some_and(|a| a.processID() == pid)
                    && w.title()
                        .is_some_and(|t| t.to_string() == dp_platform::OVERLAY_WINDOW_TITLE)
            })
            .collect()
    };
    NSArray::from_retained_slice(&ours)
}

/// Draws `image` into an RGBA8 buffer of `width`×`height` (opaque, so premultiplied
/// alpha equals straight alpha).
fn image_to_rgba(image: &CGImage, width: usize, height: usize) -> Result<Vec<u8>, String> {
    let bytes_per_row = width * 4;
    let mut rgba = vec![0_u8; bytes_per_row * height];
    let space = CGColorSpace::new_device_rgb().ok_or("CGColorSpaceCreateDeviceRGB failed")?;
    // SAFETY: `rgba` is exactly bytes_per_row*height bytes and outlives the context,
    // which is dropped at the end of this function.
    let ctx = unsafe {
        CGBitmapContextCreate(
            rgba.as_mut_ptr().cast::<c_void>(),
            width,
            height,
            8,
            bytes_per_row,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .ok_or("CGBitmapContextCreate failed")?;
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&ctx), rect, Some(image));
    drop(ctx);
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    Ok(rgba)
}

/// Captures one display with ScreenCaptureKit. Blocks until the image arrives.
fn capture_display(monitor: &MonitorInfo) -> Result<Frame, PlatformError> {
    if !objc2::available!(macos = 14.0) {
        return Err(PlatformError::Unsupported(
            "screen capture needs macOS 14 or later".to_owned(),
        ));
    }
    let display_id: CGDirectDisplayID = monitor
        .id
        .0
        .parse()
        .map_err(|_| PlatformError::NotFound(format!("monitor `{}`", monitor.id.0)))?;
    let (width, height) = (
        monitor.size_physical.0 as usize,
        monitor.size_physical.1 as usize,
    );

    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();

    let on_content = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: valid (or null) for the duration of this handler.
            let Some(content) = (unsafe { content.as_ref() }) else {
                let _ = tx.send(Err(ns_error_text(error)));
                return;
            };
            // SAFETY: plain property reads on a live SCShareableContent.
            let displays = unsafe { content.displays() };
            let Some(display) = displays.iter().find(|d| {
                // SAFETY: property read on a live SCDisplay.
                unsafe { d.displayID() == display_id }
            }) else {
                let _ = tx.send(Err(format!("display {display_id} is not shareable")));
                return;
            };
            let excluded = overlay_windows(content);
            // SAFETY: standard ScreenCaptureKit object construction and setters.
            let (filter, config) = unsafe {
                let filter = SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &excluded,
                );
                let config = SCStreamConfiguration::new();
                config.setWidth(width);
                config.setHeight(height);
                config.setShowsCursor(false);
                (filter, config)
            };
            let tx = tx.clone();
            let on_image = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
                // SAFETY: valid (or null) for the duration of this handler; we finish
                // using it before returning.
                let result = match unsafe { image.as_ref() } {
                    Some(img) => image_to_rgba(img, width, height),
                    None => Err(ns_error_text(error)),
                };
                let _ = tx.send(result);
            });
            // SAFETY: filter/config are live; the block is copied by the callee.
            unsafe {
                SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                    &filter,
                    &config,
                    Some(&on_image),
                );
            }
        },
    );
    // SAFETY: the block is copied by the callee and called once on a system queue.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&on_content) };

    let rgba = rx
        .recv_timeout(CAPTURE_TIMEOUT)
        .map_err(|_| PlatformError::Os("screen capture timed out".to_owned()))?
        .map_err(|e| PlatformError::Os(format!("screen capture failed: {e}")))?;
    Ok(Frame {
        monitor: monitor.id.clone(),
        width: monitor.size_physical.0,
        height: monitor.size_physical.1,
        rgba: Arc::from(rgba.into_boxed_slice()),
        captured_at: Instant::now(),
    })
}

impl MacScreen {
    fn ensure_permission(&self) -> Result<(), PlatformError> {
        if CGPreflightScreenCaptureAccess() {
            return Ok(());
        }
        if !self.prompted.swap(true, Ordering::Relaxed) {
            // Shows the system prompt and adds us to the Screen Recording list.
            let _ = CGRequestScreenCaptureAccess();
        }
        Err(PlatformError::PermissionDenied(
            PermissionKind::ScreenRecording,
        ))
    }
}

#[async_trait]
impl ScreenCapture for MacScreen {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        list_monitors()
    }

    async fn capture(&self, monitor: &MonitorId) -> Result<Frame, PlatformError> {
        self.ensure_permission()?;
        let monitors = list_monitors()?;
        let info = monitors
            .iter()
            .find(|m| &m.id == monitor)
            .ok_or_else(|| PlatformError::NotFound(format!("monitor `{}`", monitor.0)))?;
        capture_display(info)
    }

    fn exclude_windows(&self, _handles: &[NativeWindowHandle]) -> ExclusionSupport {
        // Our control bar stays visible on purpose so Claude can see and avoid it.
        ExclusionSupport::MustHide
    }

    fn prepare_overlay(&self) {
        crate::overlay::cover_primary_display();
    }

    fn capability(&self) -> Capability {
        if !objc2::available!(macos = 14.0) {
            return Capability::unavailable("needs macOS 14 or later");
        }
        if CGPreflightScreenCaptureAccess() {
            Capability::Available
        } else {
            Capability::Unavailable {
                reason: "Screen Recording permission not granted".to_owned(),
                workaround: Some(
                    "System Settings ▸ Privacy & Security ▸ Screen & System Audio Recording"
                        .to_owned(),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_at_least_one_monitor_with_a_primary() {
        let monitors = list_monitors().unwrap();
        assert!(!monitors.is_empty());
        assert!(monitors[0].is_primary);
        assert!(monitors.iter().all(|m| m.scale_factor >= 1.0));
    }

    #[test]
    #[ignore = "needs the Screen Recording permission for the test runner"]
    fn captures_the_primary_monitor() {
        let monitors = list_monitors().unwrap();
        let frame = capture_display(&monitors[0]).unwrap();
        assert_eq!(frame.width, monitors[0].size_physical.0);
        assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
        let pixels = frame.rgba.as_chunks::<4>().0;
        assert!(pixels.iter().any(|px| px != &pixels[0]));
    }
}
