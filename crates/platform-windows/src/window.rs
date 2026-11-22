//! Foreground window query and focus (TRD TR-W7). Window listing arrives with the
//! `list_open_windows` tool (M4).

use async_trait::async_trait;
use dp_platform::{Capability, PlatformError, WindowId, WindowInfo, WindowManager};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    SetForegroundWindow,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WindowsWindows;

fn hwnd_from_id(id: &WindowId) -> Result<HWND, PlatformError> {
    let raw: usize =
        id.0.parse()
            .map_err(|_| PlatformError::NotFound(format!("bad window id `{}`", id.0)))?;
    Ok(HWND(raw as *mut core::ffi::c_void))
}

fn info(hwnd: HWND) -> WindowInfo {
    let mut buf = [0_u16; 512];
    // SAFETY: valid buffer; returns the number of characters written.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    let title = String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)]);
    let mut pid = 0_u32;
    // SAFETY: valid out-pointer for the process id.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    WindowInfo {
        id: WindowId((hwnd.0 as usize).to_string()),
        title,
        app: String::new(),
        pid: Some(pid),
        bounds: (0, 0, 0, 0),
        focused: true,
    }
}

#[async_trait]
impl WindowManager for WindowsWindows {
    async fn list(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        Err(PlatformError::Unsupported(
            "listing windows arrives with the window tools (M4)".to_owned(),
        ))
    }

    async fn focus(&self, id: &WindowId) -> Result<(), PlatformError> {
        let target = hwnd_from_id(id)?;
        // SAFETY: plain Win32 calls on window handles; IsWindow validates the target.
        unsafe {
            if !IsWindow(Some(target)).as_bool() {
                return Err(PlatformError::NotFound(
                    "window no longer exists".to_owned(),
                ));
            }
            // Windows only lets the foreground thread change the foreground window;
            // attaching our input queue to it lifts that restriction for this call.
            let foreground = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(foreground, None);
            let our_thread = GetCurrentThreadId();
            let attached = fg_thread != 0
                && fg_thread != our_thread
                && AttachThreadInput(our_thread, fg_thread, true).as_bool();
            let ok = SetForegroundWindow(target).as_bool();
            let _ = BringWindowToTop(target);
            if attached {
                let _ = AttachThreadInput(our_thread, fg_thread, false);
            }
            if ok {
                Ok(())
            } else {
                Err(PlatformError::Os(
                    "Windows refused to focus the window".to_owned(),
                ))
            }
        }
    }

    async fn focused(&self) -> Result<Option<WindowInfo>, PlatformError> {
        // SAFETY: no preconditions.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return Ok(None);
        }
        Ok(Some(info(hwnd)))
    }

    fn capability(&self) -> Capability {
        Capability::Degraded {
            reason: "focus only; window listing arrives in M4".to_owned(),
            workaround: None,
        }
    }
}
