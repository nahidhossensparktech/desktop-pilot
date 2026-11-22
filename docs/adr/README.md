# Architecture Decision Records

Format: [MADR](https://adr.github.io/madr/)-style — Context, Decision, Alternatives, Consequences, Assumptions, Open questions. One file per decision. Status values: Proposed → Accepted → (Superseded by ADR-xxx).

| ADR | Title | Status |
|---|---|---|
| [ADR-001](ADR-001-gui-framework.md) | GUI framework: eframe/egui | Accepted |
| [ADR-002](ADR-002-wayland-strategy.md) | Wayland strategy: RemoteDesktop + ScreenCast portals, PipeWire, libei | Accepted |
| [ADR-003](ADR-003-claude-client-raw-http.md) | Claude client over raw HTTP with own SSE parser | Accepted |
| [ADR-004](ADR-004-computer-use-toolset.md) | Computer use via `computer_toolset_20260801` | Accepted |
| [ADR-005](ADR-005-concurrency-model.md) | Concurrency model and the stop path | Accepted |
| [ADR-006](ADR-006-append-only-history.md) | Append-only history; server-side context editing; checkpoint compaction | Accepted |
| [ADR-007](ADR-007-coordinate-mapping.md) | Typed coordinate spaces and mapping | Accepted |
| [ADR-008](ADR-008-model-registry-in-config.md) | Model registry in config | Accepted |
| [ADR-009](ADR-009-risk-classifier.md) | Deterministic rule-based risk classifier | Accepted |
| [ADR-010](ADR-010-secret-storage.md) | Secret storage via `keyring` | Accepted |

Planned later: ADR-011 updater crate (M6), ADR-012 voice transcription (M7).
