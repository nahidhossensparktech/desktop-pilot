# Desktop Pilot — Product Requirements Document (PRD)

| Field | Value |
|---|---|
| Status | Draft v0.1, awaiting approval |
| Date | 2026-11-16 |
| Owner | Product / Lead engineer |
| Related | [TRD](TRD.md) · [ARCHITECTURE](ARCHITECTURE.md) · [DESIGN](DESIGN.md) · [SECURITY](SECURITY.md) · [TEST_PLAN](TEST_PLAN.md) · [ROADMAP](ROADMAP.md) |

---

## 1. Problem statement

Browser automation agents can operate web pages, but most real work spans several programs: a file manager, a mail client, an IDE, system settings, a music player. Today a user who wants "take the numbers from this PDF, put them in a spreadsheet and email it to Sam" has three options:

1. Do every click by hand.
2. Write a script (AutoHotkey, AppleScript, `xdotool`). That needs programming skill, breaks when the UI changes, and is per-OS.
3. Use a cloud "computer use" demo inside a throw-away VM. The VM cannot see the user's own apps, files or sign-ins.

No native, cross-platform, safety-first tool lets a person drive **their own desktop** with natural language while staying in control.

## 2. Vision

> *Desktop Pilot is a co-pilot for the whole operating system. You say what you want; it sees the screen, plans, and carries out the clicks and keystrokes step by step. You can watch, approve, pause or stop at any moment.*

Guiding principles (they shape every later decision):

1. **The user is always in control.** Stop is always one click or one key away and takes effect in under 100 ms.
2. **Safe by default.** Risky actions ask first. The shell is off. On-screen text is data, never instructions.
3. **Native and light.** A single Rust binary that starts in under a second and idles near zero CPU.
4. **Equal on every OS.** Windows, macOS and Linux (X11 and Wayland) are all first-class.
5. **Private.** Nothing leaves the machine except the calls to the Claude API that the user started.

## 3. Target users and personas

| Persona | Description | Goals | Pain points | What success looks like |
|---|---|---|---|---|
| **P1: Priya, power user** | Product manager, 10+ apps open, keyboard-shortcut fan. Windows at work, macOS at home. | Hand off repetitive multi-app chores (rename files, move data between apps, set up a meeting workspace). | Scripts are brittle; RPA tools are heavy and need IT. | Finishes 80% of her daily chores with one prompt and a glance. |
| **P2: Omar, non-technical user** | Retired teacher, uses a laptop for mail, photos and video calls. | "Make the text bigger", "attach the photo from yesterday to an email to my daughter". | Does not know where settings are; fears breaking things. | Trusts the app because it asks before anything risky and explains what it does. |
| **P3: Dev, developer / automator** | Backend developer on Fedora + KDE Wayland. | Automate GUI-only tools, reproduce UI bugs, test desktop apps. | `xdotool` does not work on Wayland; no cross-platform equivalent. | A reliable, scriptable agent with logs, step replay and a debug panel. |
| **P4: Ana, accessibility user (secondary)** | Has limited fine-motor control. | Do mouse-heavy tasks by typing (later speaking). | Small click targets, drag-and-drop. | Can complete common tasks without precise mouse use. |

## 4. User stories with acceptance criteria

IDs are `US-xx`. "Given / When / Then" acceptance criteria are testable. Benchmark tasks in [TEST_PLAN §6](TEST_PLAN.md) map to these stories.

### Core flow

**US-01: Run a natural-language task.**
As Priya, I type *"open Spotify and play my liked songs"* and press Ctrl/Cmd+Enter, and the agent does it.
- Given Spotify is installed and signed in, when I run the prompt, then within 60 s Spotify is focused, "Liked Songs" is open and playback has started.
- The activity log shows each step (thought summary, action, result) in order.
- The status bar shows state, step count, tokens used and elapsed time while it runs.
- When the task ends, state becomes **Completed** and Claude's final message is shown.

**US-02: Follow-up prompt in the same session.**
As Priya, after US-01 I type *"now turn the volume down to 30%"*, and the agent continues with the conversation context.
- The previous conversation is reused (no "what is Spotify?" re-discovery).
- "New session" clears the context; the next prompt starts fresh.

**US-03: Choose a model.**
As Dev, I pick a model from a dropdown before running.
- The dropdown lists every model in the config file (default list: `claude-sonnet-5-5` (default), `claude-opus-5-5`, `claude-haiku-5-5`, `claude-fable-5-2`).
- Adding a new `[[models]]` entry to the config and restarting makes it appear, with no code change.
- The model can be changed between prompts; the change applies to the next request.

### Control and safety

**US-04: FORCE STOP.**
As any user, I press the big red STOP button, the global hotkey (default Ctrl+Alt+Esc / ⌃⌥⎋), or slam the mouse into the top-left screen corner, and everything stops.
- No further input is injected more than 100 ms after the trigger (measured, see [TEST_PLAN §5.3](TEST_PLAN.md)).
- Any held key or mouse button is released.
- The in-flight HTTP request is aborted.
- State becomes **Stopped**; the log shows "Stopped by user (hotkey)" with the trigger source.

**US-05: Pause and resume.**
As Priya, I press Pause; the agent finishes the current single atomic action, then waits. Resume continues from the same point.
- While paused, no input is injected and no new API request is started.
- A pause longer than the configurable idle limit (default 10 min) turns into Stopped.

**US-06: Confirm a risky action.**
As Omar, when the agent is about to click "Send" in my mail client, a dialog shows the screenshot with the target highlighted, the proposed action in plain words, and **Approve / Reject / Stop**.
- In the default mode ("Ask for risky actions") only actions classified as risky ([SECURITY §4.2](SECURITY.md)) ask for confirmation.
- Reject sends Claude a tool result saying the user declined, and Claude must choose another path or ask.
- With no answer, the dialog stays open forever (no auto-approve). The agent stays in AwaitingConfirmation.

**US-07: Choose the confirmation mode.**
As Dev, I set confirmation to "Fully automatic" for a trusted workflow. As Omar, I set "Ask every action".
- The mode is shown in the status bar at all times.
- Switching to Fully automatic shows a one-time warning. Hard-blocked actions (shell, password typing) still always ask, see [SECURITY §4.2](SECURITY.md).

### Apps, windows and context

**US-08: Launch an app by name.**
As Omar, I say *"open the calculator"* and the right app opens, even though it is called "Calculator", "gnome-calculator" or "Rechner".
- `launch_app` fuzzy-matches against the installed-app catalog (display names, localized names, executable names).
- When there are several close matches, the agent picks the best one and logs the alternatives.

**US-09: Work with windows.**
As Priya, I say *"bring my Excel window with the budget to the front"*.
- `list_open_windows` returns title, app, PID and bounds; `focus_window` raises the window.
- On Wayland compositors that do not allow this, the agent says so and falls back to the taskbar/Alt+Tab via computer use.

**US-10: Precise targeting via the accessibility tree.**
As Dev, I want the agent to click "Save" reliably even when the button moved.
- `get_ui_tree` returns a pruned tree (role, name, bounds, enabled, focused) of the focused window.
- Bounds are returned in the same screenshot pixel space that Claude uses for clicks.

**US-11: Clipboard.**
As Priya, *"copy the order number from this page and paste it into the form"*.
- `read_clipboard` / `write_clipboard` work with plain text. Content that looks like a secret is masked in logs.

**US-12: Shell command (opt-in).**
As Dev, I enable the shell tool in Settings and ask *"list the 5 biggest files in Downloads"*.
- The tool is absent from the tool list sent to Claude until enabled.
- Every shell command is shown in a confirmation dialog, in every confirmation mode.
- Output is truncated to a configurable size (default 16 KB) before being returned.

### Onboarding, settings, history

**US-13: First run onboarding.**
As Omar on macOS, the first time I open the app a wizard explains and requests Accessibility and Screen Recording permissions, and checks they were granted.
- On Wayland the wizard triggers the portal consent dialog once and stores the restore token.
- The wizard can be re-opened from Settings ▸ Permissions.
- Until required permissions are granted, Run is disabled with a clear reason.

**US-14: Enter an API key safely.**
As any user, I paste my Anthropic API key once; it is stored in the OS keychain and never shown again in full.
- The key never appears in logs, config files, crash reports or screenshots of the app.
- "Test key" validates it with a minimal API call and shows success or the API error.

**US-15: Session history.**
As Priya, I can browse past sessions (prompt, model, outcome, steps, tokens, duration) and open one to read its log.
- Screenshots are not stored unless "Save screenshots locally" is on.
- Sessions can be deleted one by one or all at once.

**US-16: Mini mode.**
As Priya, while the agent works the main window hides and a small always-on-top bar shows status plus STOP.
- The mini bar is excluded from screenshots where the OS allows it (see [TRD §5.6](TRD.md)); elsewhere it hides during capture.

**US-17: Graceful degradation.**
As Dev on GNOME Wayland, if a capability is missing (e.g. window focus), the app tells me which feature is limited and why, and never crashes.
- Settings ▸ About ▸ Capabilities shows a matrix of available / degraded / unavailable features for this machine.

## 5. Functional requirements

Priority uses MoSCoW: **M**ust, **S**hould, **C**ould, **W**on't (v1).

### 5.1 Prompting and sessions

| ID | Requirement | Priority |
|---|---|---|
| FR-01 | Multi-line prompt input; Ctrl+Enter (Win/Linux) / Cmd+Enter (macOS) runs it; Enter adds a newline. | M |
| FR-02 | A session keeps the conversation history so follow-up prompts continue it. "New session" starts a new history. | M |
| FR-03 | Model selector populated from the config model registry; default `claude-sonnet-5-5`. | M |
| FR-04 | Per-model defaults (effort, max tokens) come from config and can be overridden in Settings. | S |
| FR-05 | Session history is persisted locally (metadata + log; screenshots only if opted in). | S |
| FR-06 | Export a session log as Markdown/JSON. | C |
| FR-07 | Voice input (push-to-talk). | W (M7) |

### 5.2 Agent loop

| ID | Requirement | Priority |
|---|---|---|
| FR-10 | Loop: prompt + screenshot → Claude → tool calls → execute in order → tool results (+ screenshot when needed) → repeat until `end_turn`, max steps, timeout, token budget, error or stop. | M |
| FR-11 | Support every member of the current computer use toolset (`computer_toolset_20260801`): `screenshot`, `zoom`, `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `mouse_move`, `left_mouse_down`, `left_mouse_up`, `cursor_position`, `scroll`, `type`, `key`, `hold_key`, `wait`. | M |
| FR-12 | Batched actions in one response run in order; on the first failure later actions are not run and are reported as "Not executed". | M |
| FR-13 | Stream the response (SSE) and show progress-update notes and text live in the log. | M |
| FR-14 | Configurable limits: max steps (default 50), per-task timeout (default 15 min), per-task token budget (default 400k input+output). | M |
| FR-15 | Configurable delay between actions (default 150 ms) so the user can follow along. | S |
| FR-16 | Handle API errors with retries and backoff; show a clear message for non-retryable errors (bad key, no credit, refusal). | M |
| FR-17 | Handle `refusal` stop reason: show the category to the user and end the task cleanly. | M |

### 5.3 Platform capabilities

| ID | Requirement | Priority |
|---|---|---|
| FR-20 | Capture a screenshot of the selected monitor, downscale it to the configured size, and map model coordinates back to physical pixels exactly. | M |
| FR-21 | Inject mouse (move, buttons, drag, scroll) and keyboard (text with Unicode, key combos, hold) input. | M |
| FR-22 | Work on Windows 10/11 (x64, ARM64), macOS 13+ (arm64, x86_64), Linux X11 and Wayland (GNOME, KDE). | M |
| FR-23 | Detect the Linux session type at runtime (`XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`) and choose the backend. | M |
| FR-24 | Multi-monitor: the user selects the target monitor; the agent works on one monitor per task. | M |
| FR-25 | Agent can switch the target monitor with a custom tool. | C |
| FR-26 | Exclude the app's own windows from screenshots, or hide them during capture. | M |
| FR-27 | Capability report: each OS backend reports what works, what is degraded and why. | M |

### 5.4 Custom tools

| ID | Requirement | Priority |
|---|---|---|
| FR-30 | `list_installed_apps` (cached, refreshed on demand / every 24 h). | M |
| FR-31 | `launch_app(name)` with fuzzy matching. | M |
| FR-32 | `list_open_windows`, `focus_window(title)`. | M (degraded on some Wayland compositors) |
| FR-33 | `get_ui_tree(window)` from UIA / AX / AT-SPI, pruned and size-limited. | S |
| FR-34 | `read_clipboard`, `write_clipboard` (text). | M |
| FR-35 | `run_shell_command`: off by default, always confirmed, timeout, output cap. | S |
| FR-36 | Tools are registered in a registry so new ones can be added without changing the agent loop. | M |

### 5.5 Safety

| ID | Requirement | Priority |
|---|---|---|
| FR-40 | FORCE STOP via UI button, global hotkey and mouse-to-corner fail-safe. | M |
| FR-41 | Pause/Resume. | M |
| FR-42 | Three confirmation modes; default "Ask for risky actions". | M |
| FR-43 | Rule-based risk classifier for actions (see SECURITY). | M |
| FR-44 | Confirmation dialog with screenshot, highlighted target, plain-language action, Approve/Reject/Stop. | M |
| FR-45 | Secrets in OS keychain only; masked in logs. | M |
| FR-46 | System prompt marks on-screen content as untrusted data. | M |

### 5.6 UI

| ID | Requirement | Priority |
|---|---|---|
| FR-50 | Main window per [DESIGN §3.1](DESIGN.md). | M |
| FR-51 | Mini mode (always-on-top bar) while acting. | S |
| FR-52 | Settings screen. | M |
| FR-53 | Permission onboarding wizard. | M |
| FR-54 | Light/dark theme following the OS, with override. | S |
| FR-55 | Tray / menu-bar icon with Stop and Show. | S |
| FR-56 | UI localization framework; ships English. | S (framework) / C (more languages) |

### 5.7 Distribution

| ID | Requirement | Priority |
|---|---|---|
| FR-60 | Signed MSI (Windows), signed + notarized DMG (macOS), AppImage + .deb + .rpm (Linux). | M |
| FR-61 | Auto-update with signature check. | S |
| FR-62 | Flatpak. | C |

## 6. Non-functional requirements

| ID | Category | Requirement | How measured |
|---|---|---|---|
| NFR-01 | Startup | Cold start to interactive main window ≤ 1.0 s on a 2020-era laptop (≤ 1.5 s on Linux with portal checks). | Timestamp from process start to first frame, CI benchmark + manual. |
| NFR-02 | Idle CPU | < 1% of one core when idle (no task, window visible). | `perf`/Activity Monitor/Task Manager over 60 s. |
| NFR-03 | Idle memory | < 150 MB RSS idle; < 400 MB during a task with 10 screenshots in memory. | OS tools; CI smoke check on Linux. |
| NFR-04 | Stop latency | FORCE STOP → last injected input ≤ 100 ms (p99); held keys released ≤ 100 ms. | Automated harness ([TEST_PLAN §5.3](TEST_PLAN.md)). |
| NFR-05 | Local step overhead | Local time per step (capture + encode + execute, excluding model time and configured delay) ≤ 300 ms p50, ≤ 600 ms p95 at 1280×800. | Tracing spans. |
| NFR-06 | UI responsiveness | UI never blocks; frame time ≤ 16 ms p95 while a task runs. | egui frame timing. |
| NFR-07 | Reliability | No crash on missing capability; every platform error becomes a typed error shown to the user. Crash-free sessions ≥ 99.5%. | Local crash counter (opt-in telemetry). |
| NFR-08 | Privacy | Only the Claude API receives data. Screenshots in memory only by default. No telemetry unless opted in, and then local only. | Network audit test (egress allowlist). |
| NFR-09 | Security | API key only in OS secret store; never logged. | Log scanning test with canary key. |
| NFR-10 | Accessibility | App UI fully keyboard-navigable; screen-reader labels via AccessKit; WCAG 2.2 AA contrast. | Manual checklist + contrast check. |
| NFR-11 | Localization | All UI strings in resource files (Fluent); RTL layout not required in v1. | Lint: no hard-coded user strings. |
| NFR-12 | Binary size | ≤ 25 MB per platform installer (excluding optional voice models). | CI artifact size check. |
| NFR-13 | Portability | Builds from one workspace on all targets; OS code only in `platform-*` crates. | CI matrix + `cargo deny`/grep lint. |
| NFR-14 | Cost awareness | Tokens and estimated cost shown live; prompt caching on by default. | Visual + `usage` fields. |

## 7. Feature list (MoSCoW)

| Feature | M | S | C | W |
|---|:-:|:-:|:-:|:-:|
| Prompt, Run, model selector | ● | | | |
| Agent loop with computer use toolset | ● | | | |
| FORCE STOP (button, hotkey, corner) | ● | | | |
| Pause / Resume | ● | | | |
| Confirmation modes + risk classifier | ● | | | |
| Live log + screenshot preview with click marker | ● | | | |
| Secure key storage | ● | | | |
| Apps: list + launch | ● | | | |
| Windows: list + focus | ● | | | |
| Clipboard tools | ● | | | |
| Permission onboarding | ● | | | |
| Capability report / graceful degradation | ● | | | |
| Multi-monitor (select target) | ● | | | |
| UI tree tool | | ● | | |
| Shell tool (opt-in) | | ● | | |
| Mini mode | | ● | | |
| Session history | | ● | | |
| Tray / menu-bar icon | | ● | | |
| Auto-update | | ● | | |
| Agent-driven monitor switching | | | ● | |
| Flatpak | | | ● | |
| Session export | | | ● | |
| Extra UI languages | | | ● | |
| Voice input | | | | ● (M7) |
| Other LLM providers | | | | ● |
| Scheduled / unattended tasks | | | | ● |
| Recording a demo and replaying it without the model ("macros") | | | | ● |
| Remote control from phone | | | | ● |

## 8. Out of scope for v1

- LLM providers other than Anthropic (the architecture allows them later, see [ARCHITECTURE §9](ARCHITECTURE.md)).
- Running unattended / on a schedule / headless on servers.
- Controlling elevated (admin) windows on Windows unless the user starts Desktop Pilot as admin.
- Controlling another machine (remote desktop).
- Mobile OSes.
- Image, video or file attachments in the prompt (Could for v1.x).
- Enterprise policy management (MDM templates), SSO.
- Guaranteed operation on Wayland compositors other than GNOME (Mutter) and KDE (KWin). Sway/Hyprland are best-effort.

## 9. Success metrics

| Metric | Target v1.0 | How collected |
|---|---|---|
| Benchmark task success rate (TEST_PLAN §6 suite, Sonnet 5.5) | ≥ 85% on each OS | Benchmark runner, manual on release candidates |
| Average steps per successful benchmark task | ≤ 12 | Session log |
| Average wall time per benchmark task | ≤ 90 s | Session log |
| FORCE STOP latency | p99 ≤ 100 ms | Stop-latency harness |
| False-negative rate of risk classifier on the risky-action corpus | 0% for "hard-risky" class; ≤ 5% overall | Unit test corpus |
| Unwanted destructive actions in dogfooding | 0 | Bug reports |
| Crash-free sessions | ≥ 99.5% | Opt-in local telemetry |
| Time from install to first successful task (new user) | ≤ 5 min (macOS incl. permissions) | Usability test, 5 users per OS |
| Idle CPU / RAM | < 1% / < 150 MB | Release checklist |

## 10. Release plan

| Release | Milestones | Contents | Audience |
|---|---|---|---|
| **MVP (internal alpha)** | M0–M3 | Main window, model selector, key storage, screenshots + input on all OSes, agent loop with computer use, streaming log, FORCE STOP + hotkey. Confirmation = "ask every action" only. | Team, dogfooding |
| **Beta** | M4–M5 | Custom tools, risk classifier + modes, onboarding, mini mode, multi-monitor/HiDPI polish, session history. Unsigned builds from CI. | 20–50 invited testers per OS |
| **v1.0** | M6 | Signed/notarized installers, auto-update, docs, benchmark ≥ 85%. | Public |
| **v1.x** | M7+ | Voice input, Flatpak, agent monitor switching, more languages, session export, file attachments. | Public |

---

## Assumptions

1. Users bring their own Anthropic API key; there is no Desktop Pilot backend or account system.
2. The current Claude computer use API is the `computer_toolset_20260801` toolset (GA, no beta header), accepted by all four default models on the Claude API (verified on platform.claude.com on 2026-11-16, see [TRD §3](TRD.md)).
3. Target hardware: 8 GB RAM or more, 1080p–4K displays, 1–3 monitors.
4. The user is present during tasks (attended automation).
5. English is the only shipped UI language in v1.0, although the agent understands prompts in any language Claude supports.
6. Wayland support targets GNOME 46+ and KDE Plasma 6.x with xdg-desktop-portal ≥ 1.18.

## Open questions

1. **Default hotkey.** Is Ctrl+Alt+Esc acceptable? (On Windows, Ctrl+Shift+Esc opens Task Manager; Ctrl+Alt+Esc is free but close to it.) Alternative: Ctrl+Alt+Shift+S.
2. **Budget defaults.** Are max 50 steps / 15 min / 400k tokens per task the right defaults, or should we default lower for cost safety?
3. **Telemetry.** Is purely local, opt-in telemetry enough, or do you want an optional anonymous upload later?
4. **Distribution channels.** Do you have (or plan) an Apple Developer ID and a Windows code-signing certificate (OV/EV or Azure Trusted Signing)?
5. **Licensing.** Open source (MIT/Apache-2.0) or proprietary? This affects GUI crate choice and dependency licensing (see ADR-001).
6. **Fable 5.2 cost.** Fable 5.2 is several times the price of Sonnet 5.5. Should it carry a cost warning in the model selector?
7. **Data retention.** Fable 5.2 requires 30-day retention (not available under zero data retention). Do any target users need ZDR?
8. **Success-rate bar.** Is ≥ 85% benchmark success the right v1.0 gate?
