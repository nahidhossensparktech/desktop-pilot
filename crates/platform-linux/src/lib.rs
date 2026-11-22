//! Linux backend: X11 (XTest, EWMH, AT-SPI) and Wayland (xdg-desktop-portal + PipeWire).
//!
//! M0: session detection only; every capability is a stub. Real backends arrive in
//! M1/M2 (see ADR-002). The whole crate is empty on other operating systems.

#![cfg(target_os = "linux")]

mod session;
mod tts;

use std::sync::Arc;

use dp_platform::{OsKind, Platform, PlatformError, PlatformInfo};

pub use session::detect_session;

/// Detects the session type and builds the matching backends.
pub fn init() -> Result<Platform, PlatformError> {
    let mut platform = Platform::stub(detect_info());
    platform.speech = Arc::new(tts::speech());
    Ok(platform)
}

/// Detects distribution version, architecture, session type and desktop environment.
pub fn detect_info() -> PlatformInfo {
    let info = os_info::get();
    let env = |name: &str| std::env::var(name).ok();
    PlatformInfo {
        os: OsKind::Linux,
        os_version: format!("{} {}", info.os_type(), info.version()),
        arch: std::env::consts::ARCH,
        session: detect_session(env),
        desktop: env("XDG_CURRENT_DESKTOP").filter(|d| !d.is_empty()),
    }
}
