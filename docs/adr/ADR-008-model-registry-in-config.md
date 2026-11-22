# ADR-008: Model registry in config

- Status: Accepted (2026-11-16)
- Date: 2026-11-16

## Context

The user must be able to select models in the UI, and new models must be addable without code changes. Models differ in defaults (effort), features (fallbacks, task budgets, thinking display), price, and the computer-use tool version they accept.

## Decision

- `[[models]]` entries in `config.toml` (fields: `id`, `display_name`, `default`, `effort`, `max_tokens`, `computer_tool`, `fallbacks`, `task_budget`, `thinking_display`, prices, `cost_warning`). Defaults shipped: `claude-sonnet-5-5` (default), `claude-opus-5-5`, `claude-haiku-5-5`, `claude-fable-5-2`.
- `llm::ModelCapabilities` turns an entry into headers/parameters. Unknown future features need a code change; a new model with known features needs only a config entry.
- "Test key" calls `GET /v1/models` to mark entries the key cannot access.
- Model can change between prompts in a session (thinking blocks from the previous model are dropped by the API, as expected).

## Alternatives considered

| Option | Why not |
|---|---|
| Hard-coded enum | Requires a release for every new model. |
| Fully dynamic from `/v1/models` | The endpoint lists capabilities, but not our UX defaults (effort, cost warning); combine instead: config is the source, API marks availability. |

## Consequences

- Config schema migration must preserve user-added models.
- Prices in config may go stale; shown as "estimated".

## Assumptions

- Model IDs listed above are valid on the Claude API (checked against the Anthropic API reference on 2026-11-16).

## Open questions

- Should the app offer to auto-add models that `/v1/models` returns but config lacks?
