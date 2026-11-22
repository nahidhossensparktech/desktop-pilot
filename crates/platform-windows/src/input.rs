//! Mouse and keyboard input through `SendInput` (TRD TR-W3).

use std::mem::size_of;
use std::sync::Mutex;

use async_trait::async_trait;
use dp_platform::{
    Capability, CoordinateSpace, DesktopPoint, Direction, HeldInputs, InputController, Key,
    MouseButton, PlatformError,
};
use windows::Win32::Foundation::POINT;

use crate::keys::{self, Vk};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW,
    SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SetCursorPos,
};

/// One wheel "notch" on Windows.
const WHEEL_DELTA: i32 = 120;

/// Mouse and keyboard controller. Remembers which buttons and keys it pressed so
/// `release_all` can let go of them (FORCE STOP, SECURITY §4.3).
#[derive(Debug, Default)]
pub(crate) struct WindowsInput {
    held_buttons: Mutex<Vec<MouseButton>>,
    held_keys: Mutex<Vec<(Key, Vk)>>,
}

fn key_input(vk: Vk, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if vk.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    // SAFETY: MapVirtualKeyW only translates the code using the current layout.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.code), MAPVK_VK_TO_VSC) };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk.code),
                wScan: u16::try_from(scan).unwrap_or(0),
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Keyboard events that type `text`. Newlines and tabs are sent as real Enter/Tab key
/// presses (many apps ignore them as Unicode characters); everything else as Unicode.
fn text_inputs(text: &str) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let special = match c {
            '\r' => {
                // Treat "\r\n" as one Enter.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                Some(0x0D)
            }
            '\n' => Some(0x0D),
            '\t' => Some(0x09),
            _ => None,
        };
        if let Some(code) = special {
            let vk = Vk {
                code,
                extended: false,
            };
            inputs.push(key_input(vk, false));
            inputs.push(key_input(vk, true));
            continue;
        }
        let mut buf = [0_u16; 2];
        for unit in c.encode_utf16(&mut buf) {
            inputs.push(unicode_input(*unit, false));
            inputs.push(unicode_input(*unit, true));
        }
    }
    inputs
}

/// The bounding rectangle of all monitors, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VirtualDesktop {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn virtual_desktop() -> VirtualDesktop {
    // SAFETY: GetSystemMetrics has no preconditions and only reads system state.
    unsafe {
        VirtualDesktop {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

/// Converts a pixel coordinate to `SendInput`'s 0..=65535 absolute range over the
/// virtual desktop. Windows maps back with `floor(n * extent / 65536)`, so we round up
/// to land exactly on the requested pixel.
fn normalize(value: i32, origin: i32, extent: i32) -> i32 {
    let extent = i64::from(extent.max(1));
    let rel = (i64::from(value) - i64::from(origin)).clamp(0, extent - 1);
    let n = (rel * 65_536 + extent - 1) / extent;
    // In range by construction: rel < extent ⇒ n ≤ 65_535 for any extent ≤ 65_536.
    i32::try_from(n.min(65_535)).unwrap_or(65_535)
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                // Wheel deltas are signed but the field is a DWORD; keep the bit pattern.
                mouseData: data as _,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), PlatformError> {
    let size = i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX);
    // SAFETY: `inputs` is a valid slice of initialized INPUT structs and `size` is the
    // size of one INPUT, as SendInput requires.
    let sent = unsafe { SendInput(inputs, size) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        // SendInput returns 0 when blocked by UIPI (target runs elevated) or the
        // secure desktop is active.
        let err = windows::core::Error::from_thread();
        Err(PlatformError::Os(format!(
            "SendInput sent {sent} of {} events ({err}); the target window may run as \
             administrator, or the secure desktop is active",
            inputs.len()
        )))
    }
}

fn button_flags(button: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

fn read_cursor() -> Result<(i32, i32), PlatformError> {
    let mut p = POINT::default();
    // SAFETY: `p` is a valid, writable POINT for the duration of the call.
    unsafe { GetCursorPos(&mut p) }
        .map_err(|e| PlatformError::Os(format!("GetCursorPos failed: {e}")))?;
    Ok((p.x, p.y))
}

impl WindowsInput {
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<MouseButton>> {
        // A poisoned lock only means another thread panicked mid-update; the list is
        // still usable, and release_all must keep working in that situation.
        self.held_buttons
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn held_keys(&self) -> std::sync::MutexGuard<'_, Vec<(Key, Vk)>> {
        self.held_keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn move_to(&self, x: i32, y: i32) -> Result<(), PlatformError> {
        let vd = virtual_desktop();
        let input = mouse_input(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            normalize(x, vd.x, vd.width),
            normalize(y, vd.y, vd.height),
            0,
        );
        send(&[input])?;
        // Rounding inside Windows can be off by one pixel on unusual layouts; make the
        // final position exact.
        if read_cursor()? != (x, y) {
            // SAFETY: plain Win32 call with integer arguments.
            unsafe { SetCursorPos(x, y) }
                .map_err(|e| PlatformError::Os(format!("SetCursorPos failed: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl InputController for WindowsInput {
    fn coordinate_space(&self) -> CoordinateSpace {
        CoordinateSpace::DesktopPhysical
    }

    async fn mouse_move(&self, p: DesktopPoint) -> Result<(), PlatformError> {
        match p {
            DesktopPoint::Physical { x, y } => self.move_to(x, y),
            DesktopPoint::Logical { .. } => Err(PlatformError::Unsupported(
                "Windows input takes physical pixel coordinates".to_owned(),
            )),
        }
    }

    async fn button(&self, b: MouseButton, dir: Direction) -> Result<(), PlatformError> {
        let (down, up) = button_flags(b);
        match dir {
            Direction::Press => {
                send(&[mouse_input(down, 0, 0, 0)])?;
                let mut held = self.held();
                if !held.contains(&b) {
                    held.push(b);
                }
            }
            Direction::Release => {
                send(&[mouse_input(up, 0, 0, 0)])?;
                self.held().retain(|h| *h != b);
            }
            Direction::Click => send(&[mouse_input(down, 0, 0, 0), mouse_input(up, 0, 0, 0)])?,
        }
        Ok(())
    }

    async fn scroll(&self, dx: i32, dy: i32) -> Result<(), PlatformError> {
        let mut inputs = Vec::with_capacity(2);
        if dy != 0 {
            // Positive dy scrolls down; Windows' positive wheel delta scrolls up.
            inputs.push(mouse_input(
                MOUSEEVENTF_WHEEL,
                0,
                0,
                dy.saturating_mul(-WHEEL_DELTA),
            ));
        }
        if dx != 0 {
            inputs.push(mouse_input(
                MOUSEEVENTF_HWHEEL,
                0,
                0,
                dx.saturating_mul(WHEEL_DELTA),
            ));
        }
        if inputs.is_empty() {
            return Ok(());
        }
        send(&inputs)
    }

    async fn key(&self, key: &Key, dir: Direction) -> Result<(), PlatformError> {
        let vk = keys::resolve(&key.0)
            .ok_or_else(|| PlatformError::NotFound(format!("unknown key name `{}`", key.0)))?;
        match dir {
            Direction::Press => {
                send(&[key_input(vk, false)])?;
                let mut held = self.held_keys();
                if !held.iter().any(|(_, h)| h.code == vk.code) {
                    held.push((key.clone(), vk));
                }
            }
            Direction::Release => {
                send(&[key_input(vk, true)])?;
                self.held_keys().retain(|(_, h)| h.code != vk.code);
            }
            Direction::Click => send(&[key_input(vk, false), key_input(vk, true)])?,
        }
        Ok(())
    }

    async fn text(&self, chunk: &str) -> Result<(), PlatformError> {
        let inputs = text_inputs(chunk);
        if inputs.is_empty() {
            return Ok(());
        }
        send(&inputs)
    }

    async fn cursor_position(&self) -> Result<Option<DesktopPoint>, PlatformError> {
        let (x, y) = read_cursor()?;
        Ok(Some(DesktopPoint::Physical { x, y }))
    }

    fn release_all(&self) -> Result<(), PlatformError> {
        let keys: Vec<(Key, Vk)> = std::mem::take(&mut *self.held_keys());
        let buttons: Vec<MouseButton> = std::mem::take(&mut *self.held());
        // Release in reverse order of pressing (modifiers last).
        let mut inputs: Vec<INPUT> = keys
            .iter()
            .rev()
            .map(|(_, vk)| key_input(*vk, true))
            .collect();
        inputs.extend(
            buttons
                .iter()
                .map(|b| mouse_input(button_flags(*b).1, 0, 0, 0)),
        );
        if inputs.is_empty() {
            return Ok(());
        }
        send(&inputs)
    }

    fn held_inputs(&self) -> HeldInputs {
        HeldInputs {
            keys: self.held_keys().iter().map(|(k, _)| k.clone()).collect(),
            buttons: self.held().clone(),
        }
    }

    fn capability(&self) -> Capability {
        Capability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Windows does with a normalized coordinate.
    fn denormalize(n: i32, origin: i32, extent: i32) -> i32 {
        origin + i32::try_from(i64::from(n) * i64::from(extent) / 65_536).unwrap()
    }

    #[test]
    fn normalize_round_trips_every_pixel() {
        // Single monitors, a dual setup with a monitor left of the primary, and an
        // ultra-wide triple setup.
        for (origin, extent) in [
            (0, 1920),
            (0, 2560),
            (-1920, 4480),
            (0, 3840),
            (-2560, 7680),
        ] {
            for v in origin..origin + extent {
                let n = normalize(v, origin, extent);
                assert!((0..=65_535).contains(&n));
                assert_eq!(
                    denormalize(n, origin, extent),
                    v,
                    "extent {extent}, pixel {v}"
                );
            }
        }
    }

    #[test]
    fn normalize_clamps_outside_points() {
        assert_eq!(normalize(-50, 0, 1920), 0);
        assert_eq!(normalize(5000, 0, 1920), normalize(1919, 0, 1920));
    }

    #[test]
    fn text_maps_newlines_to_enter_and_uses_unicode_otherwise() {
        let inputs = text_inputs("a\r\nb\u{1F642}");
        // a(2) + Enter(2) + b(2) + emoji as two UTF-16 units (4)
        assert_eq!(inputs.len(), 10);
        // SAFETY: text_inputs only builds keyboard inputs, so `ki` is the active field.
        let enter = unsafe { inputs[2].Anonymous.ki };
        assert_eq!(enter.wVk.0, 0x0D);
    }

    #[test]
    fn release_all_with_nothing_held_is_ok() {
        let input = WindowsInput::default();
        assert!(input.release_all().is_ok());
        assert!(input.held_inputs().is_empty());
    }
}
