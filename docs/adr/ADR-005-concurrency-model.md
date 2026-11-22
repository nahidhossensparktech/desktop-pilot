# ADR-005: Concurrency model and the stop path

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

The UI must never block; the agent does network I/O, image processing and OS calls; FORCE STOP must halt input within 100 ms and release held keys even if the agent is busy; macOS requires the GUI event loop (and global-hotkey/tray setup) on the main thread.

## Decision

- **Main thread:** eframe event loop. UI ↔ agent via bounded `tokio::sync::mpsc` (`AgentCommand` in, `AgentEvent` out); the agent calls `ctx.request_repaint()` after sending.
- **Tokio multi-thread runtime** (2 workers) created before eframe; one agent task per running task.
- **Stop:** `tokio_util::sync::CancellationToken` hierarchy (session → task → step). Stop triggers (button, hotkey thread, corner watcher thread, tray) hold a clone and call `cancel()` **directly**, and the hotkey/corner threads also call `InputController::release_all()` directly.
- **Pause:** `tokio::sync::watch<RunMode>`; checked between atomic actions and before each request.
- Every await point in the agent is `select!`-ed with `cancelled()`; input execution is chunked (≤ 8 chars per `type` chunk, ≤ 10 ms drag segments, sleeps in `select!`); `is_cancelled()` is checked right before each OS input call.
- Blocking OS work runs on `spawn_blocking` or dedicated threads (COM/UIA thread on Windows, PipeWire loop thread on Linux).

## Alternatives considered

| Option | Why not |
|---|---|
| Stop routed through the UI command channel | Adds a frame of latency and fails if the UI thread is busy. |
| `AtomicBool` stop flag only | Cannot be awaited; HTTP and sleeps would not wake. (We still use `is_cancelled()` as a sync check.) |
| Agent on its own OS thread without async | Streaming HTTP + timers + cancellation become manual. |

## Consequences

- `InputController::release_all()` must be thread-safe, idempotent and synchronous.
- Tests use `tokio::time::pause()` for deterministic timing; the stop-latency harness measures real latency.

## Assumptions

- `global-hotkey` callbacks are delivered on a thread we control (or via a channel we can poll on a dedicated thread) without waiting for an egui frame.

## Open questions

- On macOS, `global-hotkey` events are delivered through the main run loop; confirm in M3 that delivery is not delayed while egui is idle (we may need a dedicated wake-up or a CGEventTap-free alternative).
