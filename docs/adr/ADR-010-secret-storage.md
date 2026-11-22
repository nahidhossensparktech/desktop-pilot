# ADR-010: Secret storage via `keyring`

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

The Anthropic API key and the Wayland portal restore token must be stored securely, never in plain config files, on Windows, macOS and Linux.

## Decision

- Use the `keyring` crate behind the `SecretStore` trait: Windows Credential Manager, macOS Keychain, Linux Secret Service (GNOME Keyring / KWallet via the Secret Service API).
- Service name `com.desktoppilot.DesktopPilot`; accounts `anthropic_api_key`, `wayland_restore_token`.
- In memory as `secrecy::SecretString`.
- If no Secret Service is available on Linux: do not write the key to disk; accept `ANTHROPIC_API_KEY` from the environment for that run and show a warning with setup guidance.
- **Amendment (2026-11-16):** on Windows the store calls the Win32 `CredWriteW` / `CredReadW` / `CredDeleteW` API directly (generic credential `com.desktoppilot.DesktopPilot/anthropic_api_key`) instead of `keyring`. `keyring` 4.x reorganized into separate store crates, and the direct API is about 80 lines with no extra dependency. macOS and Linux still plan to use `keyring`.
- **Credential order** used by the app: `ANTHROPIC_API_KEY` env → `ANTHROPIC_AUTH_TOKEN` env → saved API key → the Anthropic `ant` CLI login (`ant auth print-credentials --access-token`, sent as `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`). Claude Code's own login is never read: it belongs to Claude Code.

## Alternatives considered

| Option | Why not |
|---|---|
| Encrypted file with a derived key | Key management problem moves elsewhere; weaker than OS stores. |
| Direct platform APIs per OS | More code for the same result; `keyring` is widely used. |
| Plain config with file permissions | Violates the security requirement. |

## Consequences

- macOS may show a Keychain access prompt after app updates if the code signature changes — stable signing identity required (M6).
- Headless/minimal Linux setups need the env-var path.

## Assumptions

- `keyring` supports all target platforms including Windows ARM64 (verify at M0).

## Open questions

- None.
