# Changelog

## 3.0.0 — 2025-03-11

Complete ground-up rewrite as a Rust workspace with embeddable library.

### Architecture
- **Workspace restructure** — three crates: `dial-core` (library), `dial-cli` (binary), `dial-providers` (AI backends)
- **Engine struct** — central API wrapping all operations as async methods
- **Versioned migrations** — 9 sequential SQLite migrations, auto-run on database open
- **Full async** — all Engine methods are async via tokio

### New Features

#### Task Dependencies (Phase 1)
- Dependency graph with topological sort for task selection
- Cycle detection prevents circular dependencies
- Auto-unblock: completing a task automatically unblocks dependents
- CLI: `dial task add "foo" --after 3`, `dial task deps 3`

#### Event System (Phase 2)
- `Event` enum and `EventHandler` trait for lifecycle notifications
- Multiple handler registration with ordered emission
- All terminal output routed through events (no direct println in core)

#### Provider Abstraction (Phase 3)
- `Provider` trait for pluggable AI backends
- `CliPassthrough` provider (shell-out, backwards compatible)
- `AnthropicProvider` with streaming, token counting, cost tracking
- `provider_usage` table tracks tokens, cost, model, duration per iteration

#### Configurable Validation Pipeline (Phase 4)
- Ordered validation steps with per-step timeout
- Required vs optional steps, fail-fast mode
- Backwards compatible: auto-creates pipeline from `build_cmd`/`test_cmd`
- CLI: `dial pipeline show/add/remove`

#### Token Budget Management (Phase 5)
- Approximate token counting (chars/4 heuristic)
- Priority-ranked context assembly: task spec > FTS specs > trusted solutions > failures > learnings

#### DB-Driven Failure Patterns (Phase 6)
- 21 seeded patterns across 5 categories (import, syntax, runtime, test, build)
- Pattern detection reads from database instead of hardcoded regex
- Unknown error clustering: groups recurring errors, suggests new patterns
- Promotion workflow: suggested → confirmed → trusted
- CLI: `dial patterns list/add/promote/suggest`

#### Solution Provenance (Phase 7)
- Source tracking, version, last-validated timestamp
- Confidence decay: -0.05 per 30 days of staleness
- Solution history table records all confidence changes
- CLI: `dial solutions refresh/history/decay`

#### Approval Gates (Phase 8)
- Three modes: Auto (default), Review (pause for inspection), Manual (require explicit approve)
- CLI: `dial approve`, `dial reject "reason"`
- Rejection resets task to pending for re-iteration

#### Metrics & Trends (Phase 9)
- `metrics` table with timestamped entries per iteration
- Dashboard: success rate, task completion, token usage, cost, duration
- Export formats: text, JSON, CSV
- Daily trend aggregation over configurable window
- CLI: `dial stats`, `dial stats --format json`, `dial stats --trend 30`

#### Polish & Release (Phase 10)
- Crash recovery: `dial recover` resets dangling in-progress iterations
- V2 migration: `dial migrate-v2` converts v2 databases to v3 schema
- Rustdoc on all public types, traits, and methods
- README rewrite with architecture diagram, library usage, and getting started guide

### Testing
- 115 tests total (43 unit + 72 integration)
- Full lifecycle integration test covering init → task → iterate → validate → complete
- Crash recovery test

### Breaking Changes
- Database schema v3 is not backwards compatible with v2 (use `dial migrate-v2` to convert)
- `dial-core` is now a library crate; direct function imports replaced by `Engine` methods
- All public API is async
- Event system replaces direct stdout output

## 2.2.0

Previous monolithic release. See git history for details.
