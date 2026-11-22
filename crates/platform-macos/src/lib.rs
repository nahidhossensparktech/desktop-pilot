//! macOS backend (macOS 13+, Apple Silicon and Intel).
//!
//! Implemented: monitor enumeration (CoreGraphics), screen capture (ScreenCaptureKit,
//! macOS 14+), mouse + keyboard input (CGEvent), talk-back (`say`), global hotkeys. Still stubs: windows, AX tree,
//! Keychain, clipboard.
//! The whole crate is empty on other operating systems.

#![cfg(target_os = "macos")]

mod hotkey;
mod input;
mod keys;
mod overlay;
mod screen;
mod tts;

use std::sync::Arc;

use dp_platform::{OsKind, Platform, PlatformError, PlatformInfo, SessionKind};

/// Detects the OS and builds the macOS platform backends.
pub fn init() -> Result<Platform, PlatformError> {
    let mut platform = Platform::stub(detect_info());
    platform.screen = Arc::new(screen::MacScreen::default());
    platform.input = Arc::new(input::MacInput::default());
    platform.speech = Arc::new(tts::speech());
    platform.hotkeys = Arc::new(hotkey::MacHotkeys::new());
    Ok(platform)
}

/// Detects macOS version and architecture.
pub fn detect_info() -> PlatformInfo {
    let info = os_info::get();
    PlatformInfo {
        os: OsKind::MacOs,
        os_version: info.version().to_string(),
        arch: std::env::consts::ARCH,
        session: SessionKind::MacOs,
        desktop: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_macos() {
        let info = super::detect_info();
        assert_eq!(info.os, dp_platform::OsKind::MacOs);
    }
}
