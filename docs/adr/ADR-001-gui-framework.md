# ADR-001: GUI framework — eframe/egui

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

We need a cross-platform desktop UI (Windows, macOS, Linux X11 + Wayland) for: a prompt box, a fast-updating live log, a screenshot preview with markers, settings, dialogs, a small always-on-top bar and a tray icon. Constraints: idle CPU < 1%, idle RAM < 150 MB, startup ≤ 1 s, one binary, accessible (screen readers), the UI thread must never block, and the macOS event loop must own the main thread.

## Decision

Use **eframe/egui** (with AccessKit enabled), plus `tray-icon`/`muda` for tray and native menus.

## Alternatives considered

| Option | Pros | Cons |
|---|---|---|
| **eframe/egui** (chosen) | Pure Rust; single binary; small memory; immediate mode fits a live log and preview with markers; repaints on demand (idle ≈ 0 CPU); AccessKit support; mature on winit (X11 + Wayland); MIT/Apache. | Non-native look; complex text layout and IME are weaker than native toolkits; custom widgets for some controls (e.g. virtualized log). |
| Tauri v2 | Web tech UI, rich styling, good a11y via the webview, mobile story. | Webview memory (WebView2/WKWebView/WebKitGTK) often > 150 MB; IPC boundary for every UI update; WebKitGTK quality varies on Linux; two languages. |
| Slint | Declarative, native-ish look, low memory, good embedded story. | Licensing (GPL / royalty-free with attribution / commercial) needs review for our distribution model; smaller desktop ecosystem; a11y still maturing. |
| iced | Elm architecture, pure Rust. | Accessibility less mature; API churn; multi-window support newer. |
| GTK4 / Qt bindings | Native widgets, strong a11y. | Heavy runtime deps on Windows/macOS, non-native on macOS, harder packaging. |

## Consequences

- We build a small design system (tokens, buttons, pills) on top of egui styles (see DESIGN §4).
- Need a virtualized log widget (egui `ScrollArea::show_rows`).
- Mini bar and non-activating windows may need small `platform-*` helpers via raw window handles.
- Accessibility must be verified per OS in M1; if AccessKit coverage is insufficient for screen readers on one OS, we document gaps and file upstream issues.

## Assumptions

- AccessKit in eframe works with NVDA/Narrator, VoiceOver and Orca at the level needed.
- winit's Wayland support covers our window needs (except always-on-top on GNOME, which is a compositor limitation).

## Open questions

- If the product will be closed-source, confirm all egui-ecosystem crates' licenses (MIT/Apache) — expected fine.
