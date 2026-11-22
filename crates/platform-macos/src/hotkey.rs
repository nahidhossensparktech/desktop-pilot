//! Global hotkeys with `global-hotkey` (Carbon `RegisterEventHotKey`; needs no
//! Accessibility permission). Events arrive on the main run loop, which eframe runs.
//!
//! The manager is not thread-safe, so it lives in a main-thread thread-local and
//! registering from another thread is refused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use dp_platform::{Capability, HotkeyCombo, HotkeyId, HotkeyService, PlatformError};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2::MainThreadMarker;

type Callback = Arc<dyn Fn() + Send + Sync>;

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

#[derive(Default)]
pub(crate) struct MacHotkeys {
    /// Hotkey id → (hotkey, callback).
    registered: Arc<Mutex<HashMap<u32, (HotKey, Callback)>>>,
}

impl std::fmt::Debug for MacHotkeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacHotkeys").finish_non_exhaustive()
    }
}

impl MacHotkeys {
    pub(crate) fn new() -> Self {
        let this = Self::default();
        let registered = this.registered.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state() != HotKeyState::Pressed {
                return;
            }
            let callback = registered
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&e.id())
                .map(|(_, cb)| cb.clone());
            if let Some(cb) = callback {
                cb();
            }
        }));
        this
    }

    fn with_manager<T>(
        f: impl FnOnce(&GlobalHotKeyManager) -> Result<T, PlatformError>,
    ) -> Result<T, PlatformError> {
        if MainThreadMarker::new().is_none() {
            return Err(PlatformError::Unsupported(
                "hotkeys can only be changed on the main thread on macOS".to_owned(),
            ));
        }
        MANAGER.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = Some(
                    GlobalHotKeyManager::new()
                        .map_err(|e| PlatformError::Os(format!("hotkey manager: {e}")))?,
                );
            }
            f(slot.as_ref().expect("set above"))
        })
    }
}

/// Parses "ctrl+alt+Escape" / "alt+Space" (case-insensitive; `win` means Command).
fn parse(combo: &str) -> Result<HotKey, PlatformError> {
    let normalized: Vec<String> = combo
        .split('+')
        .map(|p| match p.trim().to_ascii_lowercase().as_str() {
            "win" | "windows" => "super".to_owned(),
            other => other.to_owned(),
        })
        .collect();
    normalized
        .join("+")
        .parse::<HotKey>()
        .map_err(|e| PlatformError::NotFound(format!("hotkey `{combo}`: {e}")))
}

impl HotkeyService for MacHotkeys {
    fn register(
        &self,
        combo: &HotkeyCombo,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> Result<HotkeyId, PlatformError> {
        let hotkey = parse(&combo.0)?;
        Self::with_manager(|m| {
            m.register(hotkey)
                .map_err(|e| PlatformError::Os(format!("{} is taken by another app: {e}", combo.0)))
        })?;
        self.registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(hotkey.id(), (hotkey, Arc::from(callback)));
        Ok(HotkeyId(hotkey.id()))
    }

    fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError> {
        let entry = self
            .registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.0);
        if let Some((hotkey, _)) = entry {
            Self::with_manager(|m| {
                m.unregister(hotkey)
                    .map_err(|e| PlatformError::Os(format!("unregister hotkey: {e}")))
            })?;
        }
        Ok(())
    }

    fn capability(&self) -> Capability {
        Capability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_combos() {
        assert!(parse("ctrl+alt+Escape").is_ok());
        assert!(parse("alt+Space").is_ok());
        assert!(parse("win+shift+A").is_ok());
        assert_ne!(
            parse("alt+Space").unwrap(),
            parse("ctrl+alt+Escape").unwrap()
        );
        assert!(parse("alt+NoSuchKey").is_err());
    }
}
