//! Platform Abstraction Layer (PAL).
//!
//! This crate declares the traits every OS backend implements (`platform-windows`,
//! `platform-macos`, `platform-linux`) and the types they share. It must never contain
//! OS-specific code or `cfg(target_os)`; `cargo xtask check-deps` enforces this.
//!
//! Signatures are the M0 skeleton from `docs/ARCHITECTURE.md` §3 and will be refined in M2.

#![forbid(unsafe_code)]

mod capability;
mod error;
mod info;
pub mod speech;
pub mod stub;
mod traits;
mod types;

use std::sync::Arc;

pub use capability::{Capability, CapabilityReport};
pub use error::PlatformError;
pub use info::{OsKind, PlatformInfo, SessionKind};
pub use traits::{
    AccessibilityTree, AppCatalog, Clipboard, HotkeyService, InputController, NativeWindowHandle,
    PermissionManager, ScreenCapture, SecretStore, TextToSpeech, WindowManager,
};
pub use types::*;

/// Title of the "Monitoring" overlay the GUI shows while a task runs. Screen capture
/// backends leave windows with this title out of screenshots, so Claude sees the real
/// screen and not the red tint.
pub const OVERLAY_WINDOW_TITLE: &str = "Desktop Pilot Monitoring";

/// The set of backends for the current machine, built once at startup by the `app` crate.
#[derive(Clone, Debug)]
pub struct Platform {
    pub info: PlatformInfo,
    pub screen: Arc<dyn ScreenCapture>,
    pub input: Arc<dyn InputController>,
    pub apps: Arc<dyn AppCatalog>,
    pub windows: Arc<dyn WindowManager>,
    pub a11y: Arc<dyn AccessibilityTree>,
    pub hotkeys: Arc<dyn HotkeyService>,
    pub permissions: Arc<dyn PermissionManager>,
    pub secrets: Arc<dyn SecretStore>,
    pub clipboard: Arc<dyn Clipboard>,
    pub speech: Arc<dyn TextToSpeech>,
}

impl Platform {
    /// Builds a platform where every capability reports "not implemented yet".
    /// Used by OS crates until their real backends land (M1/M2).
    pub fn stub(info: PlatformInfo) -> Self {
        let s = Arc::new(stub::NotYetImplemented);
        Self {
            info,
            screen: s.clone(),
            input: s.clone(),
            apps: s.clone(),
            windows: s.clone(),
            a11y: s.clone(),
            hotkeys: s.clone(),
            permissions: s.clone(),
            secrets: s.clone(),
            clipboard: s.clone(),
            speech: s,
        }
    }

    /// Collects every backend's capability for the About ▸ Capabilities view.
    pub fn capability_report(&self) -> CapabilityReport {
        CapabilityReport {
            entries: vec![
                ("Screen capture", self.screen.capability()),
                ("Mouse & keyboard input", self.input.capability()),
                ("Window management", self.windows.capability()),
                ("Accessibility tree", self.a11y.capability()),
                ("Global hotkey", self.hotkeys.capability()),
                ("Text to speech", self.speech.capability()),
            ],
        }
    }
}
