# ADR-003: Claude client over raw HTTP with own SSE parser

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

Anthropic provides official SDKs for Python, TypeScript, Java, Go, Ruby, C# and PHP, but **not for Rust**. We need streaming (SSE), precise control over beta headers per model, byte-exact replay of content blocks (thinking signatures), cancellation within 5 ms, and retries that never re-execute tools.

## Decision

Implement `llm::ClaudeClient` with `reqwest` (rustls, streaming) against `POST /v1/messages`, with:
- request/response types in `llm::types` written from the API reference (serde, `#[serde(other)]`/untagged fallbacks for unknown block types so new API blocks don't crash parsing; unknown blocks are preserved as raw JSON for replay);
- our own SSE parser (small, fuzz-tested);
- our own `RetryPolicy` integrated with `CancellationToken`;
- a `ModelCapabilities` profile per model deciding headers and parameters;
- `LlmProvider` trait in front so `core` is provider-neutral.

Unofficial community Rust crates are not used: their coverage of the newest features (computer toolset, preserved thinking, context editing, fallbacks) is uncertain and we would still need to verify every field.

## Alternatives considered

| Option | Why not |
|---|---|
| Community Rust SDK crate | Lags new API features; we'd need to fork for the toolset/thinking details anyway. |
| Run an official SDK in a sidecar (Python/TS) | Extra runtime, packaging and IPC; defeats single-binary goal. |
| `eventsource-stream` / `reqwest-eventsource` | Viable; `reqwest-eventsource` adds its own reconnect semantics that conflict with our retry rules. `eventsource-stream` remains a fallback if our parser proves troublesome. |

## Consequences

- We own API drift: fixtures in record mode (TEST_PLAN §3.1) and a re-verification step at the start of M3 and each release.
- Unknown content block types are preserved and replayed (forward compatibility).

## Assumptions

- The REST + SSE contract stays stable under `anthropic-version: 2026-10-21`.

## Open questions

- None.
