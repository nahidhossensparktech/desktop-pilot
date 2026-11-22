# ADR-009: Deterministic rule-based risk classifier

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

In the default confirmation mode only risky actions should ask the user. The classifier must be fast (< 200 ms including element lookup), predictable, testable, and must not be steerable by on-screen content or by the model being classified.

## Decision

- Deterministic rules in `core::risk`, data in `rules.toml` (localized keyword lists, app-id lists for terminals/file managers/mail/chat/settings, key-combo lists, secret regexes, Luhn check).
- Inputs: action, target element (a11y role/name/value at point, best effort with 150 ms timeout), focused window (app id/title), typed text, tool name.
- Output: `RiskLevel {None, Low, Medium, Hard}` + category + human-readable reason for the dialog.
- Unknown/uncertain → err higher.
- Corpus-based tests: 0 false negatives on Hard.

## Alternatives considered

| Option | Why not |
|---|---|
| Ask Claude to self-rate risk | The model being steered by an injection could under-rate; not deterministic. |
| Second LLM call as classifier | Adds latency and cost per action; also exposed to the same screen content. Could be an *additional* signal in v1.x (only to raise risk, never lower). |
| OCR of the click area | Heavy dependency; a11y data covers most apps. Revisit if needed. |

## Consequences

- Some false positives (e.g. "Send" in non-messaging contexts) — acceptable.
- Keyword lists need localization maintenance.

## Assumptions

- Accessibility metadata is available for mainstream apps on all OSes.

## Open questions

- Which languages must be covered at v1.0 (SECURITY open question 4)?
