//! Windows backend (Windows 10/11, x64 and ARM64).
//!
//! Implemented: mouse + keyboard input (`SendInput`), screen capture (GDI) and monitor
//! enumeration with per-monitor DPI, global hotkeys, foreground window focus, secrets in
//! Credential Manager. Still stubs: app catalog, window list, UI tree, clipboard (M4).
//! The whole crate is empty on other OSes.

#![cfg(target_os = "windows")]

mod capture;
mod dpi;
mod hotkey;
mod input;
mod keys;
mod screen;
mod secrets;
mod tts;
mod window;

use std::sync::Arc;

use dp_platform::{OsKind, Platform, PlatformError, PlatformInfo, SessionKind};

/// Detects the OS and builds the Windows platform backends.
///
/// Makes the process per-monitor DPI aware (v2) first, so every coordinate we read or
/// send is in physical pixels (TRD TR-W1).
pub fn init() -> Result<Platform, PlatformError> {
    dpi::make_process_dpi_aware();
    let mut platform = Platform::stub(detect_info());
    platform.input = Arc::new(input::WindowsInput::default());
    platform.screen = Arc::new(screen::WindowsScreen);
    platform.hotkeys = Arc::new(hotkey::WindowsHotkeys::default());
    platform.windows = Arc::new(window::WindowsWindows);
    platform.secrets = Arc::new(secrets::WindowsSecrets);
    platform.speech = Arc::new(tts::speech());
    Ok(platform)
}

/// Detects Windows version and architecture.
pub fn detect_info() -> PlatformInfo {
    let info = os_info::get();
    PlatformInfo {
        os: OsKind::Windows,
        os_version: info.version().to_string(),
        arch: std::env::consts::ARCH,
        session: SessionKind::Windows,
        desktop: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_windows() {
        let info = super::detect_info();
        assert_eq!(info.os, dp_platform::OsKind::Windows);
        assert!(!info.os_version.is_empty());
    }
}
