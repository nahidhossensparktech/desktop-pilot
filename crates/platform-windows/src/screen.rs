//! Monitor enumeration with per-monitor DPI. Screen capture arrives in M2.

use std::mem::size_of;

use async_trait::async_trait;
use dp_platform::{
    Capability, ExclusionSupport, Frame, MonitorId, MonitorInfo, NativeWindowHandle, PlatformError,
    ScreenCapture,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

/// `MONITORINFOF_PRIMARY` from the Windows SDK.
const MONITORINFOF_PRIMARY: u32 = 1;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WindowsScreen;

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the address of the `Vec<HMONITOR>` that `list_monitors` passes
    // to EnumDisplayMonitors; it outlives the synchronous enumeration.
    let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    BOOL::from(true)
}

fn list_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only writes into `handles`, which lives until the call returns.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    if !ok.as_bool() {
        return Err(PlatformError::Os("EnumDisplayMonitors failed".to_owned()));
    }

    let mut monitors = Vec::with_capacity(handles.len());
    for handle in handles {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).unwrap_or(0);
        // SAFETY: `info` is a properly sized MONITORINFOEXW with cbSize set, which
        // GetMonitorInfoW accepts through a MONITORINFO pointer.
        let ok = unsafe {
            GetMonitorInfoW(
                handle,
                (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
            )
        };
        if !ok.as_bool() {
            continue;
        }
        let (mut dpi_x, mut dpi_y) = (96_u32, 96_u32);
        // SAFETY: valid monitor handle and writable u32 out-parameters.
        if unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
            dpi_x = 96;
        }
        let scale = f64::from(dpi_x) / 96.0;
        let rc = info.monitorInfo.rcMonitor;
        let name_len = info
            .szDevice
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..name_len]);
        monitors.push(MonitorInfo {
            id: MonitorId(name.clone()),
            name,
            origin_physical: (rc.left, rc.top),
            size_physical: (
                u32::try_from(rc.right - rc.left).unwrap_or(0),
                u32::try_from(rc.bottom - rc.top).unwrap_or(0),
            ),
            origin_logical: (f64::from(rc.left) / scale, f64::from(rc.top) / scale),
            scale_factor: scale,
            is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    // Primary first, then left-to-right, so "monitor 1" is predictable.
    monitors.sort_by_key(|m| (!m.is_primary, m.origin_physical.0, m.origin_physical.1));
    Ok(monitors)
}

#[async_trait]
impl ScreenCapture for WindowsScreen {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        list_monitors()
    }

    async fn capture(&self, monitor: &MonitorId) -> Result<Frame, PlatformError> {
        let monitors = list_monitors()?;
        let info = monitors
            .iter()
            .find(|m| &m.id == monitor)
            .ok_or_else(|| PlatformError::NotFound(format!("monitor `{}`", monitor.0)))?;
        crate::capture::capture_monitor(info)
    }

    fn exclude_windows(&self, _handles: &[NativeWindowHandle]) -> ExclusionSupport {
        // Our control bar stays visible on purpose so Claude can see and avoid it.
        ExclusionSupport::MustHide
    }

    fn capability(&self) -> Capability {
        Capability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_at_least_one_monitor_with_a_primary() {
        // Hosted Windows CI runners have a virtual display; a machine without any
        // display would fail here, which is acceptable for a desktop app.
        let monitors = list_monitors().unwrap();
        assert!(!monitors.is_empty());
        assert!(monitors[0].is_primary);
        assert!(monitors.iter().all(|m| m.scale_factor >= 1.0));
    }

    #[test]
    fn captures_the_primary_monitor() {
        let monitors = list_monitors().unwrap();
        let frame = crate::capture::capture_monitor(&monitors[0]).unwrap();
        assert_eq!(frame.width, monitors[0].size_physical.0);
        assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
        // A real desktop is never a single flat color.
        let pixels = frame.rgba.as_chunks::<4>().0;
        assert!(pixels.iter().any(|px| px != &pixels[0]));
    }
}
