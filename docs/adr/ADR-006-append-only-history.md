# ADR-006: Append-only history; server-side context editing; checkpoint compaction

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

Computer-use sessions accumulate many screenshots. The classic approach — delete old screenshots client-side and keep the last N — conflicts with **preserved thinking** on the current models (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.2): each thinking block is bound to the exact conversation prefix; editing, deleting or reordering earlier turns, or changing `system`/`tools`, invalidates later thinking blocks and is rejected with a 400 on enforced accounts. Server-side context editing and compaction do **not** count as edits. Separately, request payloads grow with every image, and above 20 images a stricter per-image size limit applies.

## Decision

1. **Append-only `Session`.** Never mutate earlier messages. Freeze `system` and `tools` (sorted) for the session. Operator updates go in appended `role: "system"` messages.
2. **Server-side context editing** (`context-management-2025-06-27`, `clear_tool_uses_20250919`) clears old tool results from the model's view past a token trigger, keeping the most recent few.
3. **Checkpoint compaction** to bound payload size: when > 18 images, > 16 MB serialized, or on `413`, request a summary, then start a new message list with summary + original goal + fresh screenshot. Never inside an open tool round.
4. Evaluate server-side compaction (`compact-2026-01-12`) in M3 as a replacement for (3).
5. Recovery: on a "bound to a different conversation" 400, strip thinking blocks once and retry; log as a bug.
6. A unit test asserts byte-identical request prefixes between consecutive requests.

### Amendment (2026-11-16, first Windows implementation)

The first implementation uses a simpler scheme than (2)–(4), to be revisited:

- Every request sets `thinking.block_binding.prefix_mismatch_behavior: "drop_block"` (beta `thinking-binding-controls-2026-08-01`). If the history was edited, the API **drops** the affected thinking blocks instead of returning a 400.
- When the history holds more than 12 screenshots, all but the newest 5 are replaced with a short text note ("older screenshot removed"). This is a deliberate history edit: it costs one prompt-cache reset and the thinking blocks after the edit point, about every 7 steps.
- Server-side context editing and checkpoint compaction ((2)–(3)) are not used yet.

Why: it bounds request size and image count with no extra summary request, and fails safe through `drop_block`. Measure cache hit rate and task success before switching to (2)–(3).

## Alternatives considered

| Option | Why not |
|---|---|
| Client-side sliding window of screenshots | Breaks preserved thinking (400s) and the prompt cache every turn. |
| Strip all thinking every turn | Loses reasoning continuity and cache; higher cost per task. |
| Files API references for screenshots | Smaller requests, but images stored server-side until deleted (privacy default is memory-only). Open question. |
| Message threads beta (send only new messages) | Cannot be combined with compaction and most context editing; best-effort. Revisit later. |

## Consequences

- Long tasks occasionally pay one cache reset at a checkpoint.
- Session code is simpler to reason about (immutable log).

## Assumptions

- Exact `clear_tool_uses_20250919` configuration fields (trigger/keep) are confirmed in M3 against live docs.

## Open questions

- Files API vs checkpoint compaction (TRD open question 2).
