//! Executes the 17 members of the computer use toolset (TRD §3.1, FR-11).

use std::time::Duration;

use dp_platform::{DesktopPoint, Direction, Key, MouseButton};
use serde_json::{Value, json};

use super::{Preview, Run};
use crate::keys;
use crate::motion;
use crate::screen;

/// Text typed per call, so FORCE STOP lands between chunks (ADR-005).
const TYPE_CHUNK_CHARS: usize = 8;
const MAX_TYPE_CHARS: usize = 2_000;
const MAX_WAIT_SECS: f64 = 300.0;
const DRAG_DURATION: Duration = Duration::from_millis(350);

/// Members that only look and never change anything.
pub(crate) fn is_read_only(name: &str) -> bool {
    matches!(
        name,
        "screenshot" | "zoom" | "cursor_position" | "wait" | "mouse_move" | "scroll"
    )
}

fn coord(input: &Value, key: &str) -> Result<Option<(i64, i64)>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() == 2 => {
            let x = a[0].as_f64().ok_or("coordinate must be numbers")?;
            let y = a[1].as_f64().ok_or("coordinate must be numbers")?;
            Ok(Some((x.round() as i64, y.round() as i64)))
        }
        Some(_) => Err(format!("`{key}` must be [x, y]")),
    }
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Describes an action in plain words for the log and the confirmation dialog.
pub(crate) fn describe(name: &str, input: &Value) -> String {
    let at = coord(input, "coordinate")
        .ok()
        .flatten()
        .map(|(x, y)| format!(" at ({x}, {y})"))
        .unwrap_or_default();
    let mods = text(input, "text")
        .filter(|_| name.contains("click") || name == "scroll")
        .map(|m| format!(" holding {m}"))
        .unwrap_or_default();
    match name {
        "screenshot" => "Take a screenshot".to_owned(),
        "zoom" => format!(
            "Zoom into region {}",
            input.get("region").cloned().unwrap_or_default()
        ),
        "left_click" => format!("Left-click{at}{mods}"),
        "right_click" => format!("Right-click{at}{mods}"),
        "middle_click" => format!("Middle-click{at}{mods}"),
        "double_click" => format!("Double-click{at}{mods}"),
        "triple_click" => format!("Triple-click{at}{mods}"),
        "left_click_drag" => {
            let from = coord(input, "start_coordinate").ok().flatten();
            let to = coord(input, "coordinate").ok().flatten();
            match (from, to) {
                (Some(f), Some(t)) => format!("Drag from ({}, {}) to ({}, {})", f.0, f.1, t.0, t.1),
                _ => "Drag".to_owned(),
            }
        }
        "mouse_move" => format!("Move the mouse{at}"),
        "left_mouse_down" => "Press the left mouse button".to_owned(),
        "left_mouse_up" => "Release the left mouse button".to_owned(),
        "cursor_position" => "Read the cursor position".to_owned(),
        "scroll" => {
            let dir = text(input, "scroll_direction").unwrap_or("down");
            let n = input
                .get("scroll_amount")
                .and_then(Value::as_u64)
                .unwrap_or(3);
            format!("Scroll {dir} {n}×{at}{mods}")
        }
        "type" => {
            let t = text(input, "text").unwrap_or_default();
            let shown: String = t.chars().take(80).collect();
            let more = if t.chars().count() > 80 { "…" } else { "" };
            format!("Type \"{shown}{more}\"")
        }
        "key" => {
            let repeat = input.get("repeat").and_then(Value::as_u64).unwrap_or(1);
            let times = if repeat > 1 {
                format!(" ×{repeat}")
            } else {
                String::new()
            };
            format!("Press {}{times}", text(input, "text").unwrap_or("?"))
        }
        "hold_key" => format!(
            "Hold {} for {} s",
            text(input, "text").unwrap_or("?"),
            input.get("duration").and_then(Value::as_f64).unwrap_or(0.0)
        ),
        "wait" => format!(
            "Wait {} s",
            input.get("duration").and_then(Value::as_f64).unwrap_or(0.0)
        ),
        other => format!("Unknown action `{other}`"),
    }
}

/// The model-space point an action targets, for the preview marker.
pub(crate) fn target_point(input: &Value) -> Option<(i64, i64)> {
    coord(input, "coordinate").ok().flatten()
}

impl Run<'_> {
    async fn sleep(&self, d: Duration) -> Result<(), String> {
        tokio::select! {
            () = self.cancel.cancelled() => Err("stopped".to_owned()),
            () = tokio::time::sleep(d) => Ok(()),
        }
    }

    fn check_cancel(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            Err("stopped".to_owned())
        } else {
            Ok(())
        }
    }

    fn desktop(&self, (x, y): (i64, i64)) -> Result<DesktopPoint, String> {
        let map = self.map.as_ref().ok_or("take a screenshot first")?;
        map.to_desktop(x, y, &self.platform.input.coordinate_space())
    }

    /// Moves the mouse to a screenshot point: a short visible glide when configured
    /// (grabbing the mouse during it stops the task), otherwise a jump.
    async fn move_to(&self, p: (i64, i64)) -> Result<(), String> {
        self.check_cancel()?;
        let d = self.desktop(p)?;
        if let DesktopPoint::Physical { x, y } = d
            && !self.cfg.mouse_motion.is_zero()
        {
            return motion::smooth_move(
                &*self.platform.input,
                (x, y),
                self.cfg.mouse_motion,
                self.cancel,
            )
            .await
            .map_err(|e| e.to_string());
        }
        self.platform
            .input
            .mouse_move(d)
            .await
            .map_err(|e| e.to_string())
    }

    async fn press_keys(&self, combo: &[String]) -> Result<(), String> {
        for k in combo {
            self.check_cancel()?;
            self.platform
                .input
                .key(&Key(k.clone()), Direction::Press)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    async fn release_keys(&self, combo: &[String]) -> Result<(), String> {
        let mut first_err = None;
        for k in combo.iter().rev() {
            if let Err(e) = self
                .platform
                .input
                .key(&Key(k.clone()), Direction::Release)
                .await
            {
                first_err.get_or_insert(e.to_string());
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Holds optional modifier keys (e.g. "shift" or "ctrl+shift") around `action`.
    async fn with_modifiers(
        &self,
        mods: Option<&str>,
        clicks: u32,
        button: MouseButton,
    ) -> Result<(), String> {
        let combo = match mods {
            Some(m) => keys::parse_combo(m)?,
            None => Vec::new(),
        };
        self.press_keys(&combo).await?;
        let mut result = Ok(());
        for i in 0..clicks {
            if let Err(e) = self.check_cancel() {
                result = Err(e);
                break;
            }
            if let Err(e) = self.platform.input.button(button, Direction::Click).await {
                result = Err(e.to_string());
                break;
            }
            if i + 1 < clicks {
                // Well inside the double-click time, but separate events.
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }
        let released = self.release_keys(&combo).await;
        result.and(released)
    }

    pub(crate) async fn capture(&mut self) -> Result<screen::Shot, String> {
        let monitors = self
            .platform
            .screen
            .monitors()
            .await
            .map_err(|e| e.to_string())?;
        let monitor = monitors
            .iter()
            .find(|m| m.is_primary)
            .or(monitors.first())
            .cloned()
            .ok_or("no monitor found")?;
        let frame = self
            .platform
            .screen
            .capture(&monitor.id)
            .await
            .map_err(|e| e.to_string())?;
        let map = screen::ScreenMap::new(
            monitor,
            frame.width,
            frame.height,
            self.cfg.screenshot_long_edge,
        );
        let shot = tokio::task::block_in_place(|| screen::screenshot(&frame, &map))?;
        self.preview(&shot, None);
        self.map = Some(map);
        self.frame = Some(frame);
        self.last_shot = Some(shot.clone());
        Ok(shot)
    }

    pub(crate) fn preview(&self, shot: &screen::Shot, marker: Option<(i64, i64)>) {
        self.emit(super::AgentEvent::Screenshot(Preview {
            rgba: shot.rgba.clone(),
            width: shot.width,
            height: shot.height,
            marker,
        }));
    }

    /// Runs one toolset member. Returns the `tool_result` content.
    pub(crate) async fn execute(&mut self, name: &str, input: &Value) -> Result<Value, String> {
        self.check_cancel()?;
        let ok = || Ok(json!("OK"));
        match name {
            "screenshot" => {
                let shot = self.capture().await?;
                Ok(json!([shot.image_block()]))
            }
            "zoom" => {
                let region: Vec<i64> = input
                    .get("region")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_f64)
                            .map(|v| v.round() as i64)
                            .collect()
                    })
                    .unwrap_or_default();
                let region: [i64; 4] = region
                    .try_into()
                    .map_err(|_| "`region` must be [x0, y0, x1, y1]")?;
                let (frame, map) = match (&self.frame, &self.map) {
                    (Some(f), Some(m)) => (f, m),
                    _ => return Err("take a screenshot first".to_owned()),
                };
                let shot = tokio::task::block_in_place(|| screen::zoom(frame, map, region))?;
                self.preview(&shot, None);
                Ok(json!([shot.image_block()]))
            }
            "left_click" | "right_click" | "middle_click" | "double_click" | "triple_click" => {
                if let Some(p) = coord(input, "coordinate")? {
                    self.move_to(p).await?;
                }
                let (button, clicks) = match name {
                    "right_click" => (MouseButton::Right, 1),
                    "middle_click" => (MouseButton::Middle, 1),
                    "double_click" => (MouseButton::Left, 2),
                    "triple_click" => (MouseButton::Left, 3),
                    _ => (MouseButton::Left, 1),
                };
                self.with_modifiers(text(input, "text"), clicks, button)
                    .await?;
                ok()
            }
            "left_click_drag" => {
                let start =
                    coord(input, "start_coordinate")?.ok_or("`start_coordinate` is required")?;
                let end = coord(input, "coordinate")?.ok_or("`coordinate` is required")?;
                let combo = match text(input, "text") {
                    Some(m) => keys::parse_combo(m)?,
                    None => Vec::new(),
                };
                self.move_to(start).await?;
                self.press_keys(&combo).await?;
                let input_ctl = self.platform.input.clone();
                let result = async {
                    input_ctl
                        .button(MouseButton::Left, Direction::Press)
                        .await
                        .map_err(|e| e.to_string())?;
                    match self.desktop(end)? {
                        DesktopPoint::Physical { x, y } => {
                            motion::smooth_move(&*input_ctl, (x, y), DRAG_DURATION, self.cancel)
                                .await
                                .map_err(|e| e.to_string())
                        }
                        other => input_ctl.mouse_move(other).await.map_err(|e| e.to_string()),
                    }
                }
                .await;
                let released = input_ctl
                    .button(MouseButton::Left, Direction::Release)
                    .await
                    .map_err(|e| e.to_string());
                let keys_released = self.release_keys(&combo).await;
                result.and(released).and(keys_released)?;
                ok()
            }
            "mouse_move" => {
                let p = coord(input, "coordinate")?.ok_or("`coordinate` is required")?;
                self.move_to(p).await?;
                ok()
            }
            "left_mouse_down" | "left_mouse_up" => {
                let dir = if name == "left_mouse_down" {
                    Direction::Press
                } else {
                    Direction::Release
                };
                self.platform
                    .input
                    .button(MouseButton::Left, dir)
                    .await
                    .map_err(|e| e.to_string())?;
                ok()
            }
            "cursor_position" => {
                let map = self.map.as_ref().ok_or("take a screenshot first")?;
                match self
                    .platform
                    .input
                    .cursor_position()
                    .await
                    .map_err(|e| e.to_string())?
                {
                    Some(DesktopPoint::Physical { x, y }) => {
                        let (mx, my) = map.to_model(x, y);
                        Ok(json!(format!("{mx}, {my}")))
                    }
                    _ => Err("cursor position is not available on this platform".to_owned()),
                }
            }
            "scroll" => {
                if let Some(p) = coord(input, "coordinate")? {
                    self.move_to(p).await?;
                }
                let amount = i32::try_from(
                    input
                        .get("scroll_amount")
                        .and_then(Value::as_u64)
                        .unwrap_or(3)
                        .min(50),
                )
                .unwrap_or(3);
                let (dx, dy) = match text(input, "scroll_direction").unwrap_or("down") {
                    "up" => (0, -amount),
                    "down" => (0, amount),
                    "left" => (-amount, 0),
                    "right" => (amount, 0),
                    other => return Err(format!("unknown scroll_direction `{other}`")),
                };
                let combo = match text(input, "text") {
                    Some(m) => keys::parse_combo(m)?,
                    None => Vec::new(),
                };
                self.press_keys(&combo).await?;
                let r = self
                    .platform
                    .input
                    .scroll(dx, dy)
                    .await
                    .map_err(|e| e.to_string());
                let released = self.release_keys(&combo).await;
                r.and(released)?;
                ok()
            }
            "type" => {
                let t = input
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("`text` is required")?;
                if t.chars().count() > MAX_TYPE_CHARS {
                    return Err(format!(
                        "text is longer than {MAX_TYPE_CHARS} characters; type it in parts"
                    ));
                }
                let chars: Vec<char> = t.chars().collect();
                for chunk in chars.chunks(TYPE_CHUNK_CHARS) {
                    self.check_cancel()?;
                    let s: String = chunk.iter().collect();
                    self.platform
                        .input
                        .text(&s)
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::time::sleep(Duration::from_millis(8)).await;
                }
                ok()
            }
            "key" => {
                let t = text(input, "text").ok_or("`text` is required")?;
                let repeat = input
                    .get("repeat")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 100);
                let sequence = keys::parse_key_sequence(t)?;
                for _ in 0..repeat {
                    for combo in &sequence {
                        self.press_keys(combo).await.inspect_err(|_| {
                            let _ = self.platform.input.release_all();
                        })?;
                        self.release_keys(combo).await?;
                        tokio::time::sleep(Duration::from_millis(15)).await;
                    }
                }
                ok()
            }
            "hold_key" => {
                let t = text(input, "text").ok_or("`text` is required")?;
                let secs = input
                    .get("duration")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5)
                    .clamp(0.0, MAX_WAIT_SECS);
                let combo = keys::parse_combo(t)?;
                self.press_keys(&combo).await?;
                let slept = self.sleep(Duration::from_secs_f64(secs)).await;
                let released = self.release_keys(&combo).await;
                slept.and(released)?;
                ok()
            }
            "wait" => {
                let secs = input
                    .get("duration")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
                    .clamp(0.0, MAX_WAIT_SECS);
                self.sleep(Duration::from_secs_f64(secs)).await?;
                ok()
            }
            other => Err(format!("unknown computer action `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_are_plain_language() {
        assert_eq!(
            describe("left_click", &json!({"coordinate": [10, 20]})),
            "Left-click at (10, 20)"
        );
        assert_eq!(
            describe(
                "left_click",
                &json!({"coordinate": [1, 2], "text": "shift"})
            ),
            "Left-click at (1, 2) holding shift"
        );
        assert_eq!(describe("key", &json!({"text": "ctrl+s"})), "Press ctrl+s");
        assert_eq!(
            describe("key", &json!({"text": "Down", "repeat": 3})),
            "Press Down ×3"
        );
        assert_eq!(describe("type", &json!({"text": "hi"})), "Type \"hi\"");
        assert_eq!(
            describe(
                "scroll",
                &json!({"scroll_direction": "up", "scroll_amount": 5})
            ),
            "Scroll up 5×"
        );
        assert!(is_read_only("screenshot") && !is_read_only("left_click") && !is_read_only("type"));
    }

    #[test]
    fn coordinates_are_validated() {
        assert_eq!(
            coord(&json!({"coordinate": [3.6, 4.2]}), "coordinate").unwrap(),
            Some((4, 4))
        );
        assert_eq!(coord(&json!({}), "coordinate").unwrap(), None);
        assert!(coord(&json!({"coordinate": [1]}), "coordinate").is_err());
        assert!(coord(&json!({"coordinate": ["a", 2]}), "coordinate").is_err());
    }
}
