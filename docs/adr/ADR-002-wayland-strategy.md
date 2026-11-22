# ADR-002: Wayland strategy — portals, PipeWire, libei

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

On Wayland, clients cannot capture the screen, inject input, read the global cursor, enumerate other windows or register global hotkeys through the display protocol. The sanctioned paths are **xdg-desktop-portal** interfaces implemented per desktop (GNOME: `xdg-desktop-portal-gnome`; KDE: `xdg-desktop-portal-kde`):

- `org.freedesktop.portal.ScreenCast` — PipeWire video streams of monitors/windows, with user consent and optional persistence (`persist_mode`, `restore_token`).
- `org.freedesktop.portal.RemoteDesktop` — synthetic pointer/keyboard events in stream coordinates; newer versions add `ConnectToEIS` (libei) for direct event emulation.
- `org.freedesktop.portal.GlobalShortcuts` — user-approved global shortcuts (KDE; GNOME in recent releases).

X11-only tools (XTest, xdotool) do not reach native Wayland windows; XWayland is not a workaround.

## Decision

1. Use **one RemoteDesktop session that also selects ScreenCast sources**, via `ashpd`, so the user sees one consent dialog for capture + control. Request `persist_mode = 2` and store the `restore_token` in the OS secret store.
2. Pull frames from the PipeWire node with the `pipewire` crate on a dedicated thread; keep the newest frame; a "screenshot" copies the latest frame after a 1–2 frame settle delay.
3. Inject input with the portal's `Notify*` methods; when `ConnectToEIS` is offered, use libei via `reis` (lower latency, keymap-aware).
4. Global STOP hotkey via the **GlobalShortcuts portal** where available; otherwise in-app shortcut, mini-bar STOP and tray STOP, with a capability warning.
5. Windows list/focus: KDE via KWin scripting over D-Bus; GNOME **degrades** (AT-SPI read-only listing; focus unsupported → Claude uses Alt+Tab/Activities via computer use).
6. Fallback when no portal backend is present (e.g. bare wlroots setups without `xdg-desktop-portal-wlr` remote desktop support): input via `ydotool` (user-configured), capture unavailable → clear guidance. Sway/Hyprland are best-effort.
7. Mouse-to-corner fail-safe is unavailable (no global cursor position); reported in the capability matrix.

## Alternatives considered

| Option | Why not |
|---|---|
| Compositor-specific protocols (wlr-screencopy, wlr-virtual-pointer, KDE fake-input) | Not available on GNOME; KDE's fake-input is restricted; maintenance per compositor. Could be added later as extra backends. |
| `ydotool`/uinput as primary input | Needs root setup or udev rules, bypasses consent, keyboard layout issues; acceptable only as a documented fallback. |
| GNOME Shell extension for windows/hotkeys | Extra install, breaks with GNOME versions, review process; maybe v1.x as optional helper. |
| Forcing X11 session | Not acceptable: Wayland is default on Ubuntu/Fedora/KDE. |

## Consequences

- First run on Wayland always shows a system consent dialog; later runs may skip it only where the portal backend honors restore tokens.
- Coordinate mapping must use stream size vs. frame size (fractional scaling) — see ADR-007.
- Our own windows cannot be excluded from the stream; the main window is minimized and the mini bar placed on another monitor when possible.
- CI needs a headless compositor with portals (or a mock portal) — see TEST_PLAN §4.1.

## Assumptions

- Target versions: GNOME ≥ 46, KDE Plasma ≥ 6.0, xdg-desktop-portal ≥ 1.18, PipeWire ≥ 0.3.
- `ashpd`, `pipewire` and `reis` crates are maintained and cover the needed interfaces (checked at M0/M2).

## Open questions

- Exact minimum GNOME version providing the GlobalShortcuts portal and whether shortcut bindings persist across sessions.
- Whether GNOME's portal honors `persist_mode` for RemoteDesktop sessions (or only ScreenCast) on the target versions.
- Should we ship an optional GNOME Shell extension in v1.x for window focus?
