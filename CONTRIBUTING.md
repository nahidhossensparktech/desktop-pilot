# Contributing to Desktop Pilot

## Before you start

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (crate layout and rules) and [docs/SECURITY.md](docs/SECURITY.md) (safety controls you must not weaken). Bigger design changes need an ADR in `docs/adr/` first.

## Ground rules

1. **OS code only in `crates/platform-*`.** Everything else talks to the OS through the traits in `dp-platform`. `cfg(target_os)` is allowed only in `platform-*` crates and `crates/app/src/platform_init.rs`. `cargo xtask check-deps` enforces this.
2. **The UI never blocks.** Long work runs on the tokio runtime; UI ↔ agent communication goes through channels.
3. **Errors:** `thiserror` in libraries, `anyhow` in the binary. Platform backends never panic on OS failures; they return `PlatformError` and report capabilities honestly.
4. **Logging:** `tracing`. Never log secrets, typed passwords, clipboard contents or full API bodies.
5. **`unsafe`** only in `platform-*`, every block with a `// SAFETY:` comment. Other crates use `#![forbid(unsafe_code)]`.
6. **Dependencies:** add the latest stable version, read its current docs, and add it to `[workspace.dependencies]`. Licenses must pass `cargo deny check`.
7. **Claude API usage:** follow [docs/TRD.md §3](docs/TRD.md). History is append-only (ADR-006). Never send `temperature`/`top_p`/`top_k` or forced `tool_choice`.

## Checks (all must pass, CI runs them on Windows, macOS and Linux)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-deps
cargo deny check
```

Clippy must be clean on every OS, because each OS compiles different code. If you can't test on an OS, say so in the PR so a reviewer can.

## Tests

See [docs/TEST_PLAN.md](docs/TEST_PLAN.md). Pure logic gets unit tests (property tests for coordinate math). Agent behavior gets integration tests against the mock Claude server. Platform code gets platform smoke tests and the manual checklist.

## Pull requests

- One topic per PR; reference the milestone (M0–M7) and requirement IDs (FR-xx / NFR-xx) it touches.
- Update docs and the README when behavior changes.
- Each milestone ends with a demo checklist in [docs/ROADMAP.md](docs/ROADMAP.md).

## Reporting security issues

Do not open a public issue. See [SECURITY.md](SECURITY.md).
