//! Smooth, cancellable mouse movement with user-takeover detection.
//!
//! Used by the debug panel now and by the agent's `left_click_drag` later. Movement is
//! split into ≤ 10 ms segments so FORCE STOP takes effect within one segment
//! (ADR-005). Before every segment the cursor is read back: if it is not where we put
//! it, the user grabbed the mouse, and we stop immediately (SECURITY §4.3).

use std::time::Duration;

use dp_platform::{DesktopPoint, InputController, PlatformError};
use tokio_util::sync::CancellationToken;

/// Time between movement segments.
pub const STEP_INTERVAL: Duration = Duration::from_millis(10);
/// How far (physical px) the cursor may drift from where we put it before we treat it
/// as the user taking over.
pub const TAKEOVER_TOLERANCE_PX: i32 = 6;

#[derive(Debug, thiserror::Error)]
pub enum MotionError {
    #[error("stopped")]
    Cancelled,
    #[error("you moved the mouse at ({}, {}), so Desktop Pilot stopped and gave control back to you", .at.0, .at.1)]
    UserTookOver { at: (i32, i32) },
    #[error("smooth movement needs an input backend with physical pixel coordinates")]
    UnsupportedCoordinates,
    #[error(transparent)]
    Platform(#[from] PlatformError),
}

/// Smoothstep easing: slow start, fast middle, slow end.
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The point at progress `t` (0..=1, eased) between `from` and `to`; exactly `to` at 1.
pub fn interpolate(from: (i32, i32), to: (i32, i32), t: f64) -> (i32, i32) {
    if t >= 1.0 {
        return to;
    }
    let e = ease_in_out(t);
    let lerp = |a: i32, b: i32| a + (f64::from(b - a) * e).round() as i32;
    (lerp(from.0, to.0), lerp(from.1, to.1))
}

async fn physical_cursor(input: &dyn InputController) -> Result<(i32, i32), MotionError> {
    match input.cursor_position().await? {
        Some(DesktopPoint::Physical { x, y }) => Ok((x, y)),
        _ => Err(MotionError::UnsupportedCoordinates),
    }
}

fn drifted(actual: (i32, i32), expected: (i32, i32)) -> bool {
    (actual.0 - expected.0).abs() > TAKEOVER_TOLERANCE_PX
        || (actual.1 - expected.1).abs() > TAKEOVER_TOLERANCE_PX
}

/// Moves the cursor to `to` over `duration`, stopping on cancel or user takeover.
///
/// Progress follows the clock, not a step count, so the movement takes `duration`
/// even when the OS timer is coarser than [`STEP_INTERVAL`] (about 15.6 ms on Windows).
pub async fn smooth_move(
    input: &dyn InputController,
    to: (i32, i32),
    duration: Duration,
    cancel: &CancellationToken,
) -> Result<(), MotionError> {
    let from = physical_cursor(input).await?;
    let mut expected = from;
    let start = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return Err(MotionError::Cancelled),
            () = tokio::time::sleep(STEP_INTERVAL) => {}
        }
        let actual = physical_cursor(input).await?;
        if drifted(actual, expected) {
            return Err(MotionError::UserTookOver { at: actual });
        }
        let t = if duration.is_zero() {
            1.0
        } else {
            start.elapsed().as_secs_f64() / duration.as_secs_f64()
        };
        let point = interpolate(from, to, t);
        if cancel.is_cancelled() {
            return Err(MotionError::Cancelled);
        }
        input
            .mouse_move(DesktopPoint::Physical {
                x: point.0,
                y: point.1,
            })
            .await?;
        expected = point;
        if t >= 1.0 {
            return Ok(());
        }
    }
}

/// Moves instantly to `to`.
pub async fn jump(
    input: &dyn InputController,
    to: (i32, i32),
    cancel: &CancellationToken,
) -> Result<(), MotionError> {
    if cancel.is_cancelled() {
        return Err(MotionError::Cancelled);
    }
    input
        .mouse_move(DesktopPoint::Physical { x: to.0, y: to.1 })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use dp_platform::{Capability, CoordinateSpace, Direction, HeldInputs, Key, MouseButton};

    use super::*;

    /// Fake controller whose cursor sits where it was last moved, optionally nudged by a
    /// simulated user after `nudge_after` moves.
    #[derive(Debug)]
    struct FakeInput {
        pos: Mutex<(i32, i32)>,
        moves: Mutex<Vec<(i32, i32)>>,
        nudge_after: Option<usize>,
    }

    impl FakeInput {
        fn at(x: i32, y: i32) -> Self {
            Self {
                pos: Mutex::new((x, y)),
                moves: Mutex::new(Vec::new()),
                nudge_after: None,
            }
        }
    }

    #[async_trait]
    impl InputController for FakeInput {
        fn coordinate_space(&self) -> CoordinateSpace {
            CoordinateSpace::DesktopPhysical
        }
        async fn mouse_move(&self, p: DesktopPoint) -> Result<(), PlatformError> {
            let DesktopPoint::Physical { x, y } = p else {
                unreachable!()
            };
            let mut moves = self.moves.lock().unwrap();
            moves.push((x, y));
            let mut pos = self.pos.lock().unwrap();
            *pos = (x, y);
            if self.nudge_after == Some(moves.len()) {
                pos.0 += 50; // the user yanks the mouse
            }
            Ok(())
        }
        async fn button(&self, _: MouseButton, _: Direction) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn scroll(&self, _: i32, _: i32) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn key(&self, _: &Key, _: Direction) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn text(&self, _: &str) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn cursor_position(&self) -> Result<Option<DesktopPoint>, PlatformError> {
            let (x, y) = *self.pos.lock().unwrap();
            Ok(Some(DesktopPoint::Physical { x, y }))
        }
        fn release_all(&self) -> Result<(), PlatformError> {
            Ok(())
        }
        fn held_inputs(&self) -> HeldInputs {
            HeldInputs::default()
        }
        fn capability(&self) -> Capability {
            Capability::Available
        }
    }

    #[test]
    fn interpolation_is_monotonic_and_exact_at_the_end() {
        let points: Vec<_> = (0..=40)
            .map(|i| interpolate((0, 0), (1000, -500), f64::from(i) / 40.0))
            .collect();
        assert_eq!(points[0], (0, 0));
        assert_eq!(*points.last().unwrap(), (1000, -500));
        assert!(
            points
                .windows(2)
                .all(|w| w[1].0 >= w[0].0 && w[1].1 <= w[0].1)
        );
        assert_eq!(interpolate((5, 5), (9, 9), 7.0), (9, 9));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_duration_jumps_in_one_move() {
        let input = FakeInput::at(1, 1);
        let token = CancellationToken::new();
        smooth_move(&input, (300, 200), Duration::ZERO, &token)
            .await
            .unwrap();
        assert_eq!(*input.moves.lock().unwrap(), vec![(300, 200)]);
    }

    #[tokio::test(start_paused = true)]
    async fn duration_follows_the_clock() {
        let input = FakeInput::at(0, 0);
        let token = CancellationToken::new();
        let start = tokio::time::Instant::now();
        smooth_move(&input, (100, 0), Duration::from_millis(600), &token)
            .await
            .unwrap();
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(600) && took <= Duration::from_millis(620));
    }

    #[tokio::test(start_paused = true)]
    async fn smooth_move_reaches_target() {
        let input = FakeInput::at(100, 100);
        let token = CancellationToken::new();
        smooth_move(&input, (700, 400), Duration::from_millis(300), &token)
            .await
            .unwrap();
        assert_eq!(*input.pos.lock().unwrap(), (700, 400));
        assert_eq!(input.moves.lock().unwrap().len(), 30);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_stops_movement() {
        let input = FakeInput::at(0, 0);
        let token = CancellationToken::new();
        token.cancel();
        let err = smooth_move(&input, (500, 500), Duration::from_millis(500), &token)
            .await
            .unwrap_err();
        assert!(matches!(err, MotionError::Cancelled));
        assert!(input.moves.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn user_takeover_stops_movement() {
        let input = FakeInput {
            nudge_after: Some(5),
            ..FakeInput::at(0, 0)
        };
        let token = CancellationToken::new();
        let err = smooth_move(&input, (900, 0), Duration::from_millis(500), &token)
            .await
            .unwrap_err();
        assert!(matches!(err, MotionError::UserTookOver { .. }));
        assert_eq!(
            input.moves.lock().unwrap().len(),
            5,
            "no move after takeover"
        );
    }
}
