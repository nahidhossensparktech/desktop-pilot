//! Screen capture of one monitor with GDI `BitBlt` (physical pixels, DPI-aware process).
//!
//! GDI is simple and fast enough for agent screenshots (~20–40 ms at 1080p). DXGI
//! Desktop Duplication (TRD TR-W2) can replace it later if needed.

use std::mem::size_of;
use std::sync::Arc;
use std::time::Instant;

use dp_platform::{Frame, MonitorInfo, PlatformError};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
};
use windows::core::{HSTRING, PCWSTR};

/// Keeps the GUI's "Monitoring" overlay out of captures (Windows 10 2004+), so Claude
/// sees the real screen and not the red tint. The control bar stays visible on purpose.
fn exclude_overlay() {
    let title = HSTRING::from(dp_platform::OVERLAY_WINDOW_TITLE);
    // SAFETY: FindWindowW only reads the given title; SetWindowDisplayAffinity only
    // changes a flag on that window, which belongs to this process.
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), &title) {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
}

/// Captures the given monitor as RGBA8.
pub(crate) fn capture_monitor(monitor: &MonitorInfo) -> Result<Frame, PlatformError> {
    exclude_overlay();
    let (x, y) = monitor.origin_physical;
    let width = i32::try_from(monitor.size_physical.0)
        .map_err(|_| PlatformError::Os("monitor too wide".to_owned()))?;
    let height = i32::try_from(monitor.size_physical.1)
        .map_err(|_| PlatformError::Os("monitor too tall".to_owned()))?;
    if width <= 0 || height <= 0 {
        return Err(PlatformError::Os("monitor has no size".to_owned()));
    }

    let pixel_count = (width as usize) * (height as usize);
    let mut bgra = vec![0_u8; pixel_count * 4];

    // SAFETY: every GDI object created here is released below, on all paths; the
    // destination buffer is exactly width*height*4 bytes for a 32-bit top-down DIB.
    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(PlatformError::Os("GetDC failed".to_owned()));
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let previous = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        let blit = BitBlt(
            mem_dc,
            0,
            0,
            width,
            height,
            Some(screen_dc),
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                biWidth: width,
                biHeight: -height, // negative = top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = if blit.is_ok() {
            GetDIBits(
                mem_dc,
                bitmap,
                0,
                height.unsigned_abs(),
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem_dc, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if let Err(e) = blit {
            return Err(PlatformError::Os(format!(
                "screen capture failed ({e}); the secure desktop may be active"
            )));
        }
        if lines != height {
            return Err(PlatformError::Os(
                "GetDIBits returned too few lines".to_owned(),
            ));
        }
    }

    // BGRA → RGBA, opaque.
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 255;
    }
    Ok(Frame {
        monitor: monitor.id.clone(),
        width: monitor.size_physical.0,
        height: monitor.size_physical.1,
        rgba: Arc::from(bgra.into_boxed_slice()),
        captured_at: Instant::now(),
    })
}
