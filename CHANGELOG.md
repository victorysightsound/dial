# Changelog

## 3.2.0 — 2026-03-12

### Unified Project Wizard (`dial new`)

Rewrites the 5-phase PRD wizard into a seamless 9-phase guided flow that takes a user from zero to autonomous iteration with one command.

#### New Command: `dial new`
- **One command to go from zero to ready**: `dial new --template mvp` handles init, spec generation, task creation, build/test config, and iteration mode selection
- **Full pause/resume**: close terminal at any phase, `dial new --resume` picks up where you left off
- **`dial spec wizard` unchanged**: still runs phases 1-5 only for PRD-only workflows

#### New Wizard Phases
- **Phase 6 — Task Review**: AI reviews auto-generated tasks, reorders by implementation sequence, adds missing tasks, removes redundant ones, sets dependency relationships
- **Phase 7 — Build & Test Config**: AI suggests build/test commands and validation pipeline steps based on technical stack
- **Phase 8 — Iteration Mode**: AI recommends iteration mode (autonomous, review every N tasks, or review each task) based on project complexity
- **Phase 9 — Launch Summary**: Prints configured project summary, ready for `dial auto-run`

#### Iteration Mode Support
- **`autonomous`** — run all tasks, commit on pass, no stops (default)
- **`review_every:N`** — pause for review after every N completed tasks
- **`review_each`** — pause after every task for approval
- `auto_run` reads `iteration_mode` from config and pauses with `awaiting_approval` status
- Resume with `dial approve` or stop with `dial reject`

#### Bug Fixes
- **Fix nested Claude Code sessions**: `auto_run` now unsets `CLAUDECODE` env var before spawning subagent, preventing "cannot launch inside another session" errors
- **Fix release workflow**: build from workspace root instead of legacy `dial/` subdirectory
- **Switch to rustls**: replaced native-tls/OpenSSL with rustls for cross-compilation support

#### Testing
- 201 total tests (up from 142)
- 27 new unit tests for WizardPhase enum, prompt builders, JSON parsing
- 3 new integration test suites for phases 6-8
- Full 9-phase wizard integration test with mock provider and resume verification

---

## 3.1.0 — 2026-03-11

### PRD Wizard & Structured Spec Database

Adds a standalone PRD database (`prd.db`) with hierarchical sections, terminology tracking, and an AI-assisted wizard for creating or refining specifications.

#### PRD Database
- **Separate `prd.db`** alongside the main phase database — sections, terminology, sources, wizard state, metadata
- **Hierarchical sections** with dotted notation (1, 1.1, 1.2.1), parent linkage, sort order, word counts
- **FTS5 full-text search** with porter tokenizer on sections and terminology, auto-synced via triggers
- **Terminology tracking** — canonical terms with variants (JSON), definitions, categories
- **Source file tracking** — records which files were imported with file size and modification timestamps

#### PRD Wizard
- **5-phase AI wizard**: Vision → Functionality → Technical → Gap Analysis → Generate
- **4 templates**: `spec`, `architecture`, `api`, `mvp` — each with purpose-built section structures
- **`--from` mode** — feed an existing document through the wizard for AI-assisted refinement
- **Pause/resume** — wizard state persisted to `prd.db`, resume with `--resume`
- **Auto-generates** PRD sections, terminology entries, and linked DIAL tasks on completion

#### Enhanced Markdown Parser
- Code fence awareness (backtick and tilde fences)
- Hierarchical section ID generation using dotted notation with counter arrays
- Parent chain determination via backward level search
- Duplicate ID handling with `_2`, `_3` suffixes
- Multi-file import with automatic top-level ID offsetting

#### New CLI Commands
- `dial spec import --dir <path>` — import markdown files into prd.db
- `dial spec wizard --template <name> [--from <doc>] [--resume]` — run the PRD wizard
- `dial spec migrate` — migrate legacy `spec_sections` into prd.db
- `dial spec term add/list/search` — terminology management
- `dial spec check` — PRD health summary
- `dial spec prd <section_id>` — show section by dotted ID
- `dial spec prd-search <query>` — full-text search PRD sections

#### Engine & Integration
- 10 new Engine methods: `prd_import`, `prd_search`, `prd_show`, `prd_list`, `prd_term_add`, `prd_term_list`, `prd_term_search`, `prd_wizard`, `prd_migrate`, plus `prd_conn` helper
- Context assembly prefers `prd.db` when available, falls back to `spec_sections` for backward compatibility
- Migration 10: `prd_section_id TEXT` column on tasks table for PRD section linking
- 7 new event variants: `PrdImported`, `WizardPhaseStarted`, `WizardPhaseCompleted`, `WizardCompleted`, `WizardPaused`, `WizardResumed`, `TermAdded`
- 4 new error variants: `PrdSectionNotFound`, `WizardError`, `TemplateNotFound`, `ProviderRequired`
- `dial index` now shows deprecation notice suggesting `dial spec import`
- 13 new integration tests covering import pipeline, terminology CRUD, section CRUD, context assembly, wizard state persistence, backward compatibility

---

## 3.0.0 — 2025-03-11

Complete ground-up rewrite as a Rust workspace with embeddable library.

### Architecture
- **Workspace restructure** — three crates: `dial-core` (library), `dial-cli` (binary), `dial-providers` (AI backends)
- **Engine struct** — central API wrapping all operations as async methods
- **Versioned migrations** — 10 sequential SQLite migrations, auto-run on database open
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
