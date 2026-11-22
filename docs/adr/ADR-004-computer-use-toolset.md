# ADR-004: Computer use via `computer_toolset_20260801`

- Status: Accepted (2026-11-16)
- Date: 2026-11-16
- Source checked: `https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool` (2026-11-16)

## Context

The project brief asked us to look up the current computer use tool version and beta header instead of using remembered strings. Findings:

- The current tool is the **computer toolset `computer_toolset_20260801`**, GA, **no beta header**. Tool entry: `{"type": "computer_toolset_20260801"}` with optional `configs` (per-member `enabled`, `defer_loading`) and `cache_control`. `name`, `display_width_px`, `display_height_px`, `display_number`, `enable_zoom` are rejected.
- 17 member tools: `screenshot`, `zoom`, `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `mouse_move`, `left_mouse_down`, `left_mouse_up`, `cursor_position`, `scroll`, `type`, `key`, `hold_key`, `wait`.
- Each call is a `tool_use` whose `name` is the member and which carries `"toolset_name": "computer"`; several may come per turn (batch). Each `tool_result` must echo `"toolset_name": "computer"`.
- The earlier `computer_20251124` tool (beta `computer-use-2025-11-24`) is **rejected with a 400** by `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5` and `claude-fable-5-2` on the Claude API.
- Images: long edge ≤ 2576 px and ≤ 4,784 visual tokens (≈ 3.75 MP); the API does not downscale; coordinates are in screenshot pixels.

## Decision

Use `computer_toolset_20260801` for all four default models. Dispatch tool calls on the pair (`toolset_name`, `name`). Implement all 17 members. Keep `zoom` enabled (implemented by cropping the last raw frame). Model profiles in `llm::ModelCapabilities` carry the tool version so a future version is a profile change, not a loop rewrite.

## Alternatives considered

| Option | Why not |
|---|---|
| `computer_20251124` | Rejected by all four target models on the Claude API. |
| Only custom tools (own `click`, `type`, …) | Loses the model's trained computer-use behavior and the server-side prompt-injection screening for screenshots. |

## Consequences

- Batch execution semantics (in order, stop at first failure, "Not executed" results) are part of the executor.
- Our custom tool names must not collide with member names (registry check).
- No display size is sent: the system prompt states the screenshot size, and every screenshot we return must already fit the limits.

## Assumptions

- The toolset remains GA and unchanged until M3; re-verify then.

## Open questions

- Does server-side `fallbacks: "default"` ever route a computer-use request to a model that does not accept the toolset? Verify at M3 (TRD open question 3).
