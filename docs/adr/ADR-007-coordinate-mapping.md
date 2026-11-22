# ADR-007: Typed coordinate spaces and mapping

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

Claude returns coordinates in the pixel space of the (downscaled) screenshot we send. OSes consume input coordinates in different spaces: Windows and X11 use physical virtual-desktop pixels; macOS CGEvent uses global points (physical / backing scale); Wayland portals use stream-logical coordinates, which differ from frame pixels under fractional scaling. Multi-monitor layouts can have negative origins and mixed scale factors. Off-by-scale bugs are the most common failure in desktop agents.

## Decision

- Separate newtypes: `ModelPoint`, `CapturePoint`, `DesktopPhysical`, `DesktopLogical` (incl. stream-logical). No implicit conversions; conversions only via `core::coords::Mapper` built from `MonitorInfo` + frame size + downscale factor.
- `InputController::coordinate_space()` declares which space it consumes; the mapper produces exactly that.
- Downscale factor formula and pixel-center rounding as in TRD §5.2; respect 2576 px long edge and 4,784 visual tokens.
- One target monitor per task; points outside → tool error.
- Property tests for round trips and bounds; table tests for real display configurations.

## Alternatives considered

| Option | Why not |
|---|---|
| Plain `(i32, i32)` tuples | Easy to mix spaces; bugs only visible on HiDPI hardware. |
| Capture the whole virtual desktop as one image | Very large images, more downscaling, worse accuracy; mixed-DPI stitching issues. |

## Consequences

- Platform crates must report accurate `MonitorInfo` (origin, size, scale) — tested in platform smoke tests.
- Display changes mid-task trigger re-mapping (pause + recapture).

## Assumptions

- Each OS exposes per-monitor scale and origin reliably (Windows per-monitor DPI v2, NSScreen, Xrandr, portal stream properties).

## Open questions

- None.
