//! Mouse and keyboard input through CGEvent (TRD TR-M5).
//!
//! Coordinates: the rest of the app works in physical pixels (screenshots, smooth
//! motion, the corner stop), so this controller accepts and reports
//! `DesktopPoint::Physical` like the Windows one, and converts to CGEvent's global
//! points per display (TR-M4: divide by the display's backing scale).
//!
//! Needs the Accessibility permission; without it macOS drops posted events silently,
//! so every action checks first and triggers the system prompt once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dp_platform::{
    Capability, CoordinateSpace, DesktopPoint, Direction, HeldInputs, InputController, Key,
    MonitorInfo, MouseButton, PermissionKind, PlatformError,
};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton,
    CGPreflightPostEventAccess, CGRequestPostEventAccess, CGScrollEventUnit,
};

use crate::keys;
use crate::screen::list_monitors;

/// Lines per wheel "notch" (Windows scrolls 3 lines per notch by default).
const LINES_PER_NOTCH: i32 = 3;
/// Clicks closer than this (time and distance) count as a double/triple click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f64 = 4.0;
const KEY_RETURN: CGKeyCode = 0x24;
const KEY_TAB: CGKeyCode = 0x30;

#[derive(Clone, Copy, Debug)]
struct LastClick {
    button: MouseButton,
    at: CGPoint,
    time: Instant,
    count: i64,
}

/// Mouse and keyboard controller. Remembers what it pressed so `release_all` can let
/// go of it (FORCE STOP, SECURITY §4.3).
#[derive(Debug, Default)]
pub(crate) struct MacInput {
    held_buttons: Mutex<Vec<MouseButton>>,
    held_keys: Mutex<Vec<(Key, CGKeyCode)>>,
    last_click: Mutex<Option<LastClick>>,
    prompted: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked mid-update; release_all must
    // keep working in that situation.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn os_err(what: &str) -> PlatformError {
    PlatformError::Os(format!("{what} failed"))
}

/// Physical pixel → global point, using the display that contains the pixel.
fn to_points(monitors: &[MonitorInfo], x: i32, y: i32) -> CGPoint {
    let contains = |m: &&MonitorInfo| {
        let (ox, oy) = m.origin_physical;
        let (w, h) = (m.size_physical.0 as i32, m.size_physical.1 as i32);
        x >= ox && x < ox + w && y >= oy && y < oy + h
    };
    let Some(m) = monitors.iter().find(contains).or(monitors.first()) else {
        return CGPoint::new(f64::from(x), f64::from(y));
    };
    CGPoint::new(
        m.origin_logical.0 + f64::from(x - m.origin_physical.0) / m.scale_factor,
        m.origin_logical.1 + f64::from(y - m.origin_physical.1) / m.scale_factor,
    )
}

/// Global point → physical pixel, using the display that contains the point.
fn to_physical(monitors: &[MonitorInfo], p: CGPoint) -> (i32, i32) {
    let contains = |m: &&MonitorInfo| {
        let (ox, oy) = m.origin_logical;
        let w = f64::from(m.size_physical.0) / m.scale_factor;
        let h = f64::from(m.size_physical.1) / m.scale_factor;
        p.x >= ox && p.x < ox + w && p.y >= oy && p.y < oy + h
    };
    let Some(m) = monitors.iter().find(contains).or(monitors.first()) else {
        return (p.x.round() as i32, p.y.round() as i32);
    };
    (
        m.origin_physical.0 + ((p.x - m.origin_logical.0) * m.scale_factor).round() as i32,
        m.origin_physical.1 + ((p.y - m.origin_logical.1) * m.scale_factor).round() as i32,
    )
}

fn cursor_points() -> Result<CGPoint, PlatformError> {
    let event = CGEvent::new(None).ok_or_else(|| os_err("CGEventCreate"))?;
    Ok(CGEvent::location(Some(&event)))
}

fn button_types(b: MouseButton) -> (CGEventType, CGEventType, CGEventType, CGMouseButton) {
    match b {
        MouseButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGEventType::LeftMouseDragged,
            CGMouseButton::Left,
        ),
        MouseButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGEventType::RightMouseDragged,
            CGMouseButton::Right,
        ),
        MouseButton::Middle => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            CGEventType::OtherMouseDragged,
            CGMouseButton::Center,
        ),
    }
}

fn post(event: &CGEvent) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
}

fn mouse_event(
    kind: CGEventType,
    at: CGPoint,
    button: CGMouseButton,
    flags: CGEventFlags,
    click_count: i64,
) -> Result<(), PlatformError> {
    let event = CGEvent::new_mouse_event(None, kind, at, button)
        .ok_or_else(|| os_err("CGEventCreateMouseEvent"))?;
    CGEvent::set_flags(Some(&event), flags);
    if click_count > 0 {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventClickState,
            click_count,
        );
    }
    post(&event);
    Ok(())
}

fn key_event(code: CGKeyCode, down: bool, flags: CGEventFlags) -> Result<(), PlatformError> {
    let event = CGEvent::new_keyboard_event(None, code, down)
        .ok_or_else(|| os_err("CGEventCreateKeyboardEvent"))?;
    CGEvent::set_flags(Some(&event), flags);
    post(&event);
    Ok(())
}

fn unicode_event(units: &[u16], down: bool) -> Result<(), PlatformError> {
    let event = CGEvent::new_keyboard_event(None, 0, down)
        .ok_or_else(|| os_err("CGEventCreateKeyboardEvent"))?;
    // Typed text ignores held modifiers, as Unicode input does on Windows.
    CGEvent::set_flags(Some(&event), CGEventFlags::empty());
    // SAFETY: `units` is a live slice of `units.len()` UTF-16 code units.
    unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr()) };
    post(&event);
    Ok(())
}

impl MacInput {
    fn ensure_permission(&self) -> Result<(), PlatformError> {
        if CGPreflightPostEventAccess() {
            return Ok(());
        }
        if !self.prompted.swap(true, Ordering::Relaxed) {
            // Shows the system prompt and adds us to the Accessibility list.
            let _ = CGRequestPostEventAccess();
        }
        Err(PlatformError::PermissionDenied(
            PermissionKind::Accessibility,
        ))
    }

    /// Flags of the modifier keys we are holding.
    fn flags(&self) -> CGEventFlags {
        lock(&self.held_keys)
            .iter()
            .filter_map(|(_, c)| keys::modifier_flag(*c))
            .fold(CGEventFlags::empty(), |a, f| a | f)
    }

    /// Click count for a press of `button` at `at` (2 = double click, …).
    fn click_count(&self, button: MouseButton, at: CGPoint) -> i64 {
        let mut last = lock(&self.last_click);
        let now = Instant::now();
        let count = match *last {
            Some(l)
                if l.button == button
                    && now.duration_since(l.time) <= MULTI_CLICK_TIME
                    && (l.at.x - at.x).abs() <= MULTI_CLICK_DISTANCE
                    && (l.at.y - at.y).abs() <= MULTI_CLICK_DISTANCE =>
            {
                l.count + 1
            }
            _ => 1,
        };
        *last = Some(LastClick {
            button,
            at,
            time: now,
            count,
        });
        count
    }

    fn current_click_count(&self, button: MouseButton) -> i64 {
        match *lock(&self.last_click) {
            Some(l) if l.button == button => l.count,
            _ => 1,
        }
    }
}

#[async_trait]
impl InputController for MacInput {
    fn coordinate_space(&self) -> CoordinateSpace {
        CoordinateSpace::DesktopPhysical
    }

    async fn mouse_move(&self, p: DesktopPoint) -> Result<(), PlatformError> {
        self.ensure_permission()?;
        let at = match p {
            DesktopPoint::Physical { x, y } => to_points(&list_monitors()?, x, y),
            DesktopPoint::Logical { x, y } => CGPoint::new(x, y),
        };
        // While a button is held, macOS expects "dragged" events, or drags do nothing.
        let held = lock(&self.held_buttons).first().copied();
        let (kind, button) = match held {
            Some(b) => {
                let (_, _, dragged, cg) = button_types(b);
                (dragged, cg)
            }
            None => (CGEventType::MouseMoved, CGMouseButton::Left),
        };
        mouse_event(kind, at, button, self.flags(), 0)
    }

    async fn button(&self, b: MouseButton, dir: Direction) -> Result<(), PlatformError> {
        self.ensure_permission()?;
        let at = cursor_points()?;
        let (down, up, _, cg) = button_types(b);
        let flags = self.flags();
        match dir {
            Direction::Press => {
                let count = self.click_count(b, at);
                mouse_event(down, at, cg, flags, count)?;
                let mut held = lock(&self.held_buttons);
                if !held.contains(&b) {
                    held.push(b);
                }
            }
            Direction::Release => {
                let count = self.current_click_count(b);
                mouse_event(up, at, cg, flags, count)?;
                lock(&self.held_buttons).retain(|h| *h != b);
            }
            Direction::Click => {
                let count = self.click_count(b, at);
                mouse_event(down, at, cg, flags, count)?;
                mouse_event(up, at, cg, flags, count)?;
            }
        }
        Ok(())
    }

    async fn scroll(&self, dx: i32, dy: i32) -> Result<(), PlatformError> {
        self.ensure_permission()?;
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        // Positive dy scrolls down and positive dx right; macOS wheel values are the
        // opposite (positive = up / left).
        let event = CGEvent::new_scroll_wheel_event2(
            None,
            CGScrollEventUnit::Line,
            2,
            dy.saturating_mul(-LINES_PER_NOTCH),
            dx.saturating_mul(-LINES_PER_NOTCH),
            0,
        )
        .ok_or_else(|| os_err("CGEventCreateScrollWheelEvent2"))?;
        CGEvent::set_flags(Some(&event), self.flags());
        post(&event);
        Ok(())
    }

    async fn key(&self, key: &Key, dir: Direction) -> Result<(), PlatformError> {
        self.ensure_permission()?;
        let code = keys::resolve(&key.0)
            .ok_or_else(|| PlatformError::NotFound(format!("unknown key name `{}`", key.0)))?;
        match dir {
            Direction::Press => {
                {
                    let mut held = lock(&self.held_keys);
                    if !held.iter().any(|(_, c)| *c == code) {
                        held.push((key.clone(), code));
                    }
                }
                // Flags now include this key if it is a modifier.
                key_event(code, true, self.flags())?;
            }
            Direction::Release => {
                lock(&self.held_keys).retain(|(_, c)| *c != code);
                key_event(code, false, self.flags())?;
            }
            Direction::Click => {
                let flags =
                    self.flags() | keys::modifier_flag(code).unwrap_or(CGEventFlags::empty());
                key_event(code, true, flags)?;
                key_event(code, false, self.flags())?;
            }
        }
        Ok(())
    }

    async fn text(&self, chunk: &str) -> Result<(), PlatformError> {
        self.ensure_permission()?;
        let mut chars = chunk.chars().peekable();
        while let Some(c) = chars.next() {
            // Newlines and tabs as real key presses (many apps ignore them as text).
            let special = match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    Some(KEY_RETURN)
                }
                '\n' => Some(KEY_RETURN),
                '\t' => Some(KEY_TAB),
                _ => None,
            };
            if let Some(code) = special {
                key_event(code, true, CGEventFlags::empty())?;
                key_event(code, false, CGEventFlags::empty())?;
                continue;
            }
            let mut buf = [0_u16; 2];
            let units = c.encode_utf16(&mut buf);
            unicode_event(units, true)?;
            unicode_event(units, false)?;
        }
        Ok(())
    }

    async fn cursor_position(&self) -> Result<Option<DesktopPoint>, PlatformError> {
        let (x, y) = to_physical(&list_monitors()?, cursor_points()?);
        Ok(Some(DesktopPoint::Physical { x, y }))
    }

    fn release_all(&self) -> Result<(), PlatformError> {
        let keys: Vec<(Key, CGKeyCode)> = std::mem::take(&mut *lock(&self.held_keys));
        let buttons: Vec<MouseButton> = std::mem::take(&mut *lock(&self.held_buttons));
        let mut result = Ok(());
        // Release in reverse order of pressing (modifiers last).
        for (_, code) in keys.iter().rev() {
            if let Err(e) = key_event(*code, false, CGEventFlags::empty()) {
                result = Err(e);
            }
        }
        if !buttons.is_empty() {
            let at = cursor_points().unwrap_or(CGPoint::new(0.0, 0.0));
            for b in buttons {
                let (_, up, _, cg) = button_types(b);
                if let Err(e) = mouse_event(up, at, cg, CGEventFlags::empty(), 0) {
                    result = Err(e);
                }
            }
        }
        result
    }

    fn held_inputs(&self) -> HeldInputs {
        HeldInputs {
            keys: lock(&self.held_keys)
                .iter()
                .map(|(k, _)| k.clone())
                .collect(),
            buttons: lock(&self.held_buttons).clone(),
        }
    }

    fn capability(&self) -> Capability {
        if CGPreflightPostEventAccess() {
            Capability::Available
        } else {
            Capability::Unavailable {
                reason: "Accessibility permission not granted".to_owned(),
                workaround: Some("System Settings ▸ Privacy & Security ▸ Accessibility".to_owned()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dp_platform::MonitorId;

    use super::*;

    fn retina_pair() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                id: MonitorId("1".into()),
                name: "main".into(),
                origin_physical: (0, 0),
                size_physical: (2880, 1800),
                origin_logical: (0.0, 0.0),
                scale_factor: 2.0,
                is_primary: true,
            },
            MonitorInfo {
                id: MonitorId("2".into()),
                name: "right".into(),
                // Origins are scaled by the largest scale (2), see screen.rs.
                origin_physical: (2880, 0),
                size_physical: (1920, 1080),
                origin_logical: (1440.0, 0.0),
                scale_factor: 1.0,
                is_primary: false,
            },
        ]
    }

    #[test]
    fn physical_and_points_round_trip_on_retina() {
        let m = retina_pair();
        let p = to_points(&m, 1000, 600);
        assert_eq!((p.x, p.y), (500.0, 300.0));
        assert_eq!(to_physical(&m, p), (1000, 600));
    }

    #[test]
    fn points_on_second_display_use_its_scale() {
        let m = retina_pair();
        let p = CGPoint::new(1500.0, 10.0);
        assert_eq!(to_physical(&m, p), (2940, 10));
        let back = to_points(&m, 2940, 10);
        assert_eq!((back.x, back.y), (1500.0, 10.0));
    }

    #[test]
    fn release_all_with_nothing_held_is_ok() {
        let input = MacInput::default();
        assert!(input.release_all().is_ok());
        assert!(input.held_inputs().is_empty());
    }
}
