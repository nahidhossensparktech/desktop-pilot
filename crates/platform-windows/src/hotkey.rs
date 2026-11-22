//! Global hotkeys with `RegisterHotKey` on a dedicated thread.
//!
//! The thread runs its own message loop, so the hotkey fires even while the UI thread
//! is busy or another app is focused (ADR-005: the stop path must not wait for a frame).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;

use dp_platform::{Capability, HotkeyCombo, HotkeyId, HotkeyService, PlatformError};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
};

use crate::keys;

static NEXT_ID: AtomicI32 = AtomicI32::new(1);

#[derive(Debug, Default)]
pub(crate) struct WindowsHotkeys {
    /// Hotkey id → message-loop thread id.
    threads: Mutex<HashMap<u32, u32>>,
}

/// Parses "ctrl+alt+Escape" into modifiers and a virtual-key code.
fn parse(combo: &str) -> Result<(HOT_KEY_MODIFIERS, u32), PlatformError> {
    let mut mods = MOD_NOREPEAT;
    let mut key = None;
    for part in combo.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" => mods |= MOD_WIN,
            _ => {
                let vk = keys::resolve(part).ok_or_else(|| {
                    PlatformError::NotFound(format!("unknown key `{part}` in hotkey `{combo}`"))
                })?;
                key = Some(u32::from(vk.code));
            }
        }
    }
    key.map(|k| (mods, k))
        .ok_or_else(|| PlatformError::NotFound(format!("hotkey `{combo}` has no key")))
}

impl HotkeyService for WindowsHotkeys {
    fn register(
        &self,
        combo: &HotkeyCombo,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> Result<HotkeyId, PlatformError> {
        let (mods, vk) = parse(&combo.0)?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let combo_text = combo.0.clone();

        std::thread::Builder::new()
            .name(format!("dp-hotkey-{id}"))
            .spawn(move || {
                // SAFETY: registers a thread-level hotkey (no window) on this thread.
                if let Err(e) = unsafe { RegisterHotKey(None, id, mods, vk) } {
                    let _ = ready_tx.send(Err(format!(
                        "`{combo_text}` could not be registered ({e}); another app may use it"
                    )));
                    return;
                }
                // SAFETY: trivial query of the current thread id.
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                let mut msg = MSG::default();
                // SAFETY: standard message loop on the thread that owns the hotkey.
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    if msg.message == WM_HOTKEY && msg.wParam.0 == id as usize {
                        callback();
                    }
                }
                // SAFETY: unregister on the same thread that registered.
                let _ = unsafe { UnregisterHotKey(None, id) };
            })
            .map_err(|e| PlatformError::Os(format!("hotkey thread: {e}")))?;

        let thread_id = ready_rx
            .recv()
            .map_err(|_| PlatformError::Os("hotkey thread exited".to_owned()))?
            .map_err(PlatformError::Os)?;
        let id = u32::try_from(id).unwrap_or(0);
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, thread_id);
        Ok(HotkeyId(id))
    }

    fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError> {
        let thread = self
            .threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id.0);
        if let Some(thread_id) = thread {
            // SAFETY: posts WM_QUIT to the hotkey thread's message queue.
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
                .map_err(|e| PlatformError::Os(format!("stop hotkey thread: {e}")))?;
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
    fn parses_stop_hotkey() {
        let (mods, vk) = parse("ctrl+alt+Escape").unwrap();
        assert_eq!(mods, MOD_NOREPEAT | MOD_CONTROL | MOD_ALT);
        assert_eq!(vk, 0x1B);
        assert!(parse("ctrl+alt").is_err());
        assert!(parse("ctrl+nokey").is_err());
    }
}
