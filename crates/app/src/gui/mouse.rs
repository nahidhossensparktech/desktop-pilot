//! The mouse test panel (debug panel "Actions", DESIGN §3.7).

use std::time::Duration;

use dp_core::motion::{self, MotionError};
use dp_platform::{DesktopPoint, Direction, InputController, MonitorInfo, MouseButton};
use eframe::egui;
use tokio_util::sync::CancellationToken;

use super::widgets::STOP_RED;

/// A mouse action from the test panel.
#[derive(Clone, Copy, Debug)]
pub(super) enum MouseAction {
    Move {
        to: (i32, i32),
        smooth: Option<Duration>,
    },
    Click {
        at: (i32, i32),
        smooth: Option<Duration>,
    },
    Square {
        center: (i32, i32),
        half: i32,
        duration: Duration,
    },
}

#[derive(Debug)]
pub(super) enum Outcome {
    Done,
    Cancelled,
    TookOver(String),
    Failed(String),
}

impl From<Result<(), MotionError>> for Outcome {
    fn from(r: Result<(), MotionError>) -> Self {
        match r {
            Ok(()) => Self::Done,
            Err(MotionError::Cancelled) => Self::Cancelled,
            Err(e @ MotionError::UserTookOver { .. }) => Self::TookOver(e.to_string()),
            Err(e) => Self::Failed(e.to_string()),
        }
    }
}

/// What the panel asks the app to do.
#[derive(Debug)]
pub(super) enum MouseRequest {
    Run(String, MouseAction),
    Pick,
}

/// Panel state.
#[derive(Debug)]
pub(super) struct MouseTest {
    pub target: (i32, i32),
    pub smooth: bool,
    pub duration_ms: u32,
    pub picking: bool,
}

impl Default for MouseTest {
    fn default() -> Self {
        Self {
            target: (0, 0),
            smooth: true,
            duration_ms: 600,
            picking: false,
        }
    }
}

pub(super) fn monitor_center(m: &MonitorInfo) -> (i32, i32) {
    let (x, y) = m.origin_physical;
    let (w, h) = m.size_physical;
    (
        x + i32::try_from(w / 2).unwrap_or(0),
        y + i32::try_from(h / 2).unwrap_or(0),
    )
}

pub(super) fn describe_monitor(i: usize, m: &MonitorInfo) -> String {
    format!(
        "{}: {}×{} at ({}, {}) {:.0}%{}",
        i + 1,
        m.size_physical.0,
        m.size_physical.1,
        m.origin_physical.0,
        m.origin_physical.1,
        m.scale_factor * 100.0,
        if m.is_primary { " primary" } else { "" }
    )
}

fn on_screen(monitors: &[MonitorInfo], p: (i32, i32)) -> bool {
    monitors.is_empty()
        || monitors.iter().any(|m| {
            let (x, y) = m.origin_physical;
            let (w, h) = m.size_physical;
            let (w, h) = (i32::try_from(w).unwrap_or(0), i32::try_from(h).unwrap_or(0));
            p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h
        })
}

/// Our own window in physical pixels (approximate), to refuse clicks on ourselves.
fn own_window_contains(ctx: &egui::Context, p: (i32, i32)) -> bool {
    ctx.input(|i| {
        let vp = i.viewport();
        match (vp.outer_rect, vp.native_pixels_per_point) {
            (Some(rect), Some(ppp)) => {
                let (px, py) = (p.0 as f32 / ppp, p.1 as f32 / ppp);
                rect.expand(4.0).contains(egui::pos2(px, py))
            }
            _ => false,
        }
    })
}

impl MouseTest {
    fn smooth_duration(&self) -> Option<Duration> {
        self.smooth
            .then(|| Duration::from_millis(u64::from(self.duration_ms)))
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        cursor: Option<(i32, i32)>,
        monitors: &[MonitorInfo],
    ) -> Option<MouseRequest> {
        let mut request = None;
        ui.horizontal(|ui| {
            ui.label("Cursor:");
            match cursor {
                Some((x, y)) => ui.monospace(format!("{x}, {y}")),
                None => ui.weak("not available on this platform yet"),
            };
        });
        ui.horizontal(|ui| {
            ui.label("Target:");
            ui.add(
                egui::DragValue::new(&mut self.target.0)
                    .prefix("x ")
                    .speed(2.0),
            );
            ui.add(
                egui::DragValue::new(&mut self.target.1)
                    .prefix("y ")
                    .speed(2.0),
            );
            let label = if self.picking {
                "Picking…"
            } else {
                "Pick (3 s)"
            };
            if ui
                .add_enabled(!self.picking && !busy, egui::Button::new(label))
                .on_hover_text(
                    "Point the mouse somewhere; its position becomes the target after 3 seconds",
                )
                .clicked()
            {
                request = Some(MouseRequest::Pick);
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.smooth, "Smooth");
            ui.add_enabled(
                self.smooth,
                egui::Slider::new(&mut self.duration_ms, 100..=3000).suffix(" ms"),
            );
        });

        let target = self.target;
        let valid = on_screen(monitors, target);
        if !valid {
            ui.colored_label(STOP_RED, "Target is outside every monitor.");
        }
        let primary = monitors.iter().find(|m| m.is_primary).or(monitors.first());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!busy && valid, egui::Button::new("Move"))
                .clicked()
            {
                request = Some(MouseRequest::Run(
                    format!("Move to ({}, {})", target.0, target.1),
                    MouseAction::Move {
                        to: target,
                        smooth: self.smooth_duration(),
                    },
                ));
            }
            let on_self = own_window_contains(ui.ctx(), target);
            let click = ui.add_enabled(
                !busy && valid && !on_self,
                egui::Button::new("Move + Click"),
            );
            let click = if on_self {
                click.on_disabled_hover_text("The target is inside Desktop Pilot's own window")
            } else {
                click
            };
            if click.clicked() {
                request = Some(MouseRequest::Run(
                    format!("Left-click at ({}, {})", target.0, target.1),
                    MouseAction::Click {
                        at: target,
                        smooth: self.smooth_duration(),
                    },
                ));
            }
            if let Some(m) = primary {
                let center = monitor_center(m);
                if ui
                    .add_enabled(!busy, egui::Button::new("Center"))
                    .on_hover_text("Move to the center of the primary monitor")
                    .clicked()
                {
                    self.target = center;
                    request = Some(MouseRequest::Run(
                        "Move to screen center".to_owned(),
                        MouseAction::Move {
                            to: center,
                            smooth: self.smooth_duration(),
                        },
                    ));
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Demo: square"))
                    .on_hover_text("Trace a square around the screen center")
                    .clicked()
                {
                    let half = (150.0 * m.scale_factor).round() as i32;
                    let duration = Duration::from_millis(u64::from(self.duration_ms.max(300)) * 4);
                    request = Some(MouseRequest::Run(
                        "Draw a square".to_owned(),
                        MouseAction::Square {
                            center,
                            half,
                            duration,
                        },
                    ));
                }
            }
        });
        request
    }
}

/// Runs one mouse action on the tokio runtime.
pub(super) async fn perform(
    input: &dyn InputController,
    action: MouseAction,
    token: &CancellationToken,
) -> Result<(), MotionError> {
    async fn go(
        input: &dyn InputController,
        to: (i32, i32),
        smooth: Option<Duration>,
        token: &CancellationToken,
    ) -> Result<(), MotionError> {
        match smooth {
            Some(d) => motion::smooth_move(input, to, d, token).await,
            None => motion::jump(input, to, token).await,
        }
    }

    match action {
        MouseAction::Move { to, smooth } => go(input, to, smooth, token).await,
        MouseAction::Click { at, smooth } => {
            go(input, at, smooth, token).await?;
            if token.is_cancelled() {
                return Err(MotionError::Cancelled);
            }
            input.button(MouseButton::Left, Direction::Click).await?;
            Ok(())
        }
        MouseAction::Square {
            center,
            half,
            duration,
        } => {
            let start = match input.cursor_position().await? {
                Some(DesktopPoint::Physical { x, y }) => (x, y),
                _ => center,
            };
            let corners = [
                (center.0 - half, center.1 - half),
                (center.0 + half, center.1 - half),
                (center.0 + half, center.1 + half),
                (center.0 - half, center.1 + half),
                (center.0 - half, center.1 - half),
            ];
            let leg = duration / 4;
            for corner in corners {
                motion::smooth_move(input, corner, leg, token).await?;
            }
            motion::smooth_move(input, start, leg, token).await
        }
    }
}
