# Desktop Pilot

Control your computer with natural language. Desktop Pilot is a native Rust desktop app that sees your screen, launches apps, and clicks, types and scrolls for you, step by step, powered by the Claude API's computer use tool. It runs on **Windows 10/11, macOS 13+ and Linux (X11 and Wayland)**.

> **Status: Windows 11 agent preview.** On Windows 10/11 you type a request, and Claude looks at your screen and does it with the mouse and keyboard, asking before each action by default. macOS and Linux build and open the window, but cannot control input yet. Not yet built: app/window/clipboard tools and the risk classifier (M4), multi-monitor selection, session history. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Try it (Windows 11)

1. **Connect to Claude**, in one of these ways (first match wins):
   - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` environment variable;
   - an API key saved in **Settings ▸ Save key** (stored in Windows Credential Manager);
   - the Anthropic **`ant` CLI**: install it (below), run `ant auth login` once, and Desktop Pilot uses that login automatically.
2. Run `cargo run --release -p desktop-pilot`.
3. Type a request (e.g. *"Open Notepad and type hello"*), choose a model, press **Run** or Ctrl+Enter.
4. The window shrinks to a small bar at the top-right. By default it asks **once per session** ("Allow Desktop Pilot to use your mouse and keyboard for this session?"; follow-up prompts and voice follow-ups don't ask again, **New session** does), then works on its own, asking again only for **security-sensitive steps**: installers and setup windows, permission/UAC prompts, Windows Settings, Control Panel, Registry, terminals, the Run dialog, deleting, Ctrl+Alt+Del, typing passwords, keys or card numbers, typing shell commands, and Enter in mail/chat apps. Other modes in **Settings ▸ Safety**: ask before every action, or never ask. Clicks are judged by the window they happen in, since button names aren't visible to Desktop Pilot yet.

**Token saving (on by default):** 1024 px screenshots, a fresh screenshot returned with each action (saves a whole turn each time), and, with the Claude Code engine, Desktop Pilot's short system prompt instead of Claude Code's large default one. The log shows turns and tokens after each task. The mouse glides visibly to each target (Settings ▸ Mouse glide; 0 = jump).

**Stopping:** the **STOP** button, **Esc** (in the window), **Ctrl+Alt+Esc** (anywhere), or push the mouse into the **top-left screen corner**. **Pause** finishes the current action, then waits.

Installing the `ant` CLI on Windows: download `ant_<version>_windows_amd64.zip` from [the releases page](https://github.com/anthropics/anthropic-cli/releases), check its SHA-256 against `ant_<version>_checksums.txt`, and put `ant.exe` in a folder on your PATH. Then run `ant auth login` (opens the browser; uses your Claude Console account and its API billing).

Desktop Pilot never uses Claude Code's own login.

### Engine: Claude Code (your Claude plan) or Claude API

**Settings ▸ Engine** chooses who runs your prompt:
- **Claude Code** (default): the window runs your installed Claude Code CLI in headless mode (`claude -p`), which uses its own login, e.g. your Claude Pro plan. No API key needed. Desktop Pilot serves its tools to that Claude Code process from a private local endpoint (127.0.0.1, random secret), so approvals, the compact bar, STOP and the log all work as usual. Claude Code's own tools (shell, files, web) are turned off for these runs. Follow-ups continue the same Claude Code conversation; **New session** starts a fresh one.
- **Claude API**: Desktop Pilot calls the API itself (API key or `ant` login). Needs Console credit.

### Use it from Claude Code (MCP mode, no API key)

`desktop-pilot --mcp` runs Desktop Pilot as an MCP server, so **Claude Code is the brain and Desktop Pilot is the hands**. Claude Code uses its own login (e.g. your Claude Pro plan), so no API key or Console credit is needed for Desktop Pilot.

1. Build: `cargo build --release -p desktop-pilot`, then copy `target\release\desktop-pilot.exe` to a stable folder, e.g. `%LOCALAPPDATA%\DesktopPilot\bin\`. A separate copy means rebuilding never fails because Claude Code has the file open.
2. Register it once, for your user:
   ```
   claude mcp add --scope user desktop-pilot -- "%LOCALAPPDATA%\DesktopPilot\bin\desktop-pilot.exe" --mcp
   ```
3. Start a **new** Claude Code session and ask, e.g. *"Use desktop-pilot to open Notepad and type hello"*. Claude Code asks your permission before each tool call.

The tools match Claude's computer-use actions: `screenshot`, `zoom`, `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `mouse_move`, `left_mouse_down`, `left_mouse_up`, `cursor_position`, `scroll`, `type`, `key`, `hold_key`, `wait`. Coordinates are in the pixels of the latest screenshot.

**Stopping:** press **Ctrl+Alt+Esc** or push the mouse into the **top-left corner**. The current action stops, held keys are released, and every tool refuses to act for 30 seconds, so Claude Code has to check with you. Close the Desktop Pilot *window* while using MCP mode: only one process can own the Ctrl+Alt+Esc hotkey.

Screenshots use a lot of tokens, so long tasks use up your plan's usage limit faster.

### Mouse test panel

In **Mouse test**:
- **Move**: moves the cursor to the target X/Y (physical pixels; smooth or instant).
- **Pick (3 s)**: point at something on screen; after 3 seconds its position becomes the target.
- **Move + Click**: moves there and left-clicks. Refused when the target is inside Desktop Pilot's own window.
- **Center** / **Demo: square**: move to the screen center, or trace a square around it.
- **STOP** or **Esc** stops at once. **Moving the mouse yourself** during a movement also stops it and gives you control back.

## Safety first

Desktop Pilot acts on your real desktop with your real accounts. It is designed so that you stay in control:

- **FORCE STOP** at any time: the red button, a global hotkey (default Ctrl+Alt+Esc / ⌃⌥⎋) or moving the mouse into the top-left corner. Stop takes effect within 100 ms and releases any held keys.
- Risky actions (sending messages, payments, deleting files, typing passwords, shell commands, system settings) ask for your approval first.
- Text on screen is treated as untrusted data, never as instructions.
- Your API key is stored in the OS keychain only. Screenshots stay in memory and go only to the Claude API.

Details: [docs/SECURITY.md](docs/SECURITY.md). *(These features arrive in M1–M5; this section describes the target behavior.)*

## Build from source

Requirements:
- Rust (rustup). The pinned toolchain in `rust-toolchain.toml` is installed automatically. Minimum supported Rust version: **1.95**.
- Windows: Visual Studio Build Tools with the "Desktop development with C++" workload (MSVC linker).
- macOS: Xcode Command Line Tools.
- Linux: a C toolchain plus GUI development packages, e.g. on Ubuntu `sudo apt install build-essential cmake clang libasound2-dev libxkbcommon-dev libxcb1-dev libwayland-dev libgl1-mesa-dev` (cmake + clang build the local Whisper speech engine; ALSA is for the microphone). Package: `packaging/linux/build-deb.sh` (on Debian/Ubuntu) or `packaging/linux/build-deb-docker.sh` (anywhere with Docker). Portal/PipeWire packages are added in M2.

```sh
git clone <repo-url> desktop-pilot && cd desktop-pilot
cargo build --release
cargo run -p desktop-pilot -- --version        # prints version and detected platform/session
cargo run -p desktop-pilot -- --capabilities   # what works on this machine
```

Example output on Windows 11:

```
desktop-pilot 0.1.0 os=windows os_version=10.0.26200 arch=x86_64 session=windows
```

On Linux the session is detected from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY` (`session=x11` or `session=wayland`).

## Development

```sh
cargo fmt --all                                         # format
cargo clippy --workspace --all-targets -- -D warnings   # lint (CI fails on warnings)
cargo test --workspace                                  # tests
cargo xtask check-deps                                  # architecture dependency rules
cargo deny check                                        # licenses / advisories (cargo install cargo-deny)
```

Logging: set `DESKTOP_PILOT_LOG=debug` (uses `tracing` env-filter syntax).

## Repository layout

| Path | Contents |
|---|---|
| `crates/app` | `desktop-pilot` binary: GUI (M1+), wiring, OS backend selection |
| `crates/core` | `dp-core`: agent loop, state machine, tool trait, risk classifier, coordinate mapping |
| `crates/llm` | `dp-llm`: Claude API client (raw HTTP + SSE) |
| `crates/platform` | `dp-platform`: OS abstraction traits (no OS code) |
| `crates/platform-{windows,macos,linux}` | OS implementations; the only place OS APIs are called |
| `crates/tools` | `dp-tools`: custom tools (apps, windows, clipboard, UI tree, shell) |
| `crates/config` | `dp-config`: settings, model registry, paths |
| `crates/test-support` | Mock platform and mock Claude server for tests |
| `xtask` | Repository automation (`cargo xtask check-deps`) |
| `docs/` | PRD, TRD, architecture, design, security, test plan, roadmap, ADRs |

## Permissions per OS

*(Applies from M2.)* macOS needs Accessibility and Screen Recording permission. Linux Wayland asks for screen-sharing and remote-control consent through the desktop portal. Windows needs no permissions, but cannot control windows running as administrator unless Desktop Pilot also runs as administrator. The first-run wizard walks you through this (M5).

## Documentation

- [Product requirements](docs/PRD.md) · [Technical requirements](docs/TRD.md) · [Architecture](docs/ARCHITECTURE.md) · [UI/UX design](docs/DESIGN.md)
- [Security & safety](docs/SECURITY.md) · [Test plan](docs/TEST_PLAN.md) · [Roadmap](docs/ROADMAP.md) · [ADRs](docs/adr/README.md)
- [Contributing](CONTRIBUTING.md)

## License

Not decided yet; all rights reserved until then. See [LICENSE](LICENSE).
