# Changelog

## 4.1.3 — 2026-03-28

Documentation alignment release. No runtime behavior changes.

### Changelog Catch-Up
- adds the missing `4.1.1` and `4.1.2` release notes to this changelog
- keeps the repository, release tag, crates.io packages, and installed binary aligned on one canonical version after the documentation update

---

## 4.1.2 — 2026-03-28

Release alignment and publishability patch. No runtime behavior changes beyond the Unicode-dash hardening shipped in `4.1.1`.

### Release Alignment & Publishability
- rolls the crates.io publish metadata fix into an official tagged release
- keeps GitHub release assets, crates.io packages, `main`, and local installs aligned on the same revision

---

## 4.1.1 — 2026-03-28

Command input hardening release focused on preventing Unicode dash characters from breaking command execution.

### Command Input Hardening
- normalizes obvious Unicode dash flag input in `build_cmd`, `test_cmd`, and validation pipeline commands
- re-checks commands before execution so existing bad config values no longer block `dial validate`
- adds prompt guidance telling AI providers to use ASCII hyphen-minus characters in commands, flags, JSON, and code
- covers config writes, wizard-generated commands, manual pipeline steps, and execution-time validation with regression tests

---

## 4.1.0 — 2026-03-13

4 enhancements to the wizard and iteration systems. 354 total tests (up from 308).

### Failed Attempt Diff Capture
- `git_diff()` and `git_diff_stat()` helpers in `git.rs` capture the working tree diff as a String
- On validation failure, captures diff and diff stat **before** `checkpoint_restore()` wipes the working tree
- Stores both in the iteration `notes` field with `FAILED_DIFF_STAT:` and `FAILED_DIFF:` prefixes (diff truncated to 2000 chars)
- On retry attempts (attempt > 1), context assembly includes the previous failed diff at priority 12:
  `PREVIOUS ATTEMPT (failed): Error: {error} / Changes attempted: {diff_stat} {diff} / DO NOT repeat this approach.`
- Slots between FTS specs (priority 10) and suggested solutions (priority 15) in the context budget

### Spec Specificity Enforcement (Phase 4 Enhancement)
- Phase 4 (GapAnalysis) prompt now includes a SPECIFICITY CHECK section
- AI reviews each PRD section for vague language (`should`, `might`, `could`, `etc.`, `various`) and flags sections lacking acceptance criteria
- Each section rated as `SPECIFIC`, `NEEDS_DETAIL`, or `VAGUE`
- `VAGUE` sections are rewritten with concrete acceptance criteria before proceeding to Phase 5
- Rewritten sections are updated in `prd.db` and preserved in `gathered_info` for Phase 5 to reference

### Task Sizing Analysis (Phase 6 Enhancement)
- Phase 6 (TaskReview) prompt now includes a TASK SIZING ANALYSIS section
- Evaluates each task for scope (1-3 files), specificity (concrete enough for AI), and testability (verifiable by build+test)
- Tasks exceeding 3 files or covering multiple features are split into smaller tasks with dependency relationships
- Vague descriptions rewritten to be concrete (e.g., "Build auth system" → "Add bcrypt password hashing to User model with cost factor 12")
- Tasks too small for a separate iteration are merged
- Each task annotated as `[S]mall`, `[M]edium`, `[L]arge`, or `[XL]needs-review`
- New `TaskSplit` event variant in `event.rs`

### Test Coverage Generation (Phase 7 Enhancement)
- Phase 7 (BuildTestConfig) prompt now includes a TEST STRATEGY section
- AI reviews feature tasks and determines whether each needs a dedicated test task or inline tests
- Complex features get separate test tasks with dependencies on the feature task
- Test task descriptions are specific: "Write integration tests for POST /users: valid input 201, duplicate email 409, missing fields 422"
- Suggests test framework based on tech stack (cargo test, pytest, jest, go test)
- Suggests validation pipeline steps with `sort_order`, `required` flag, and `timeout`

### Testing
- 354 total tests (up from 308)
- New unit tests: git_diff helpers, diff truncation, retry context inclusion, specificity prompt content, sizing response parsing, test task parsing
- New integration tests: fail→capture→retry cycle, vague section rewrite, task splitting, feature-test task pairing

---

## 4.0.0 — 2026-03-12

10 new features across 3 tiers: foundation infrastructure, intelligent context, and analytics/observability. 308 total tests (up from 201).

### Tier 1 — Foundation

#### Transaction Safety
- `with_transaction()` helper in `db.rs` wraps closures in `BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK`
- Uses `BEGIN IMMEDIATE` (not plain `BEGIN`) to acquire write locks upfront in WAL mode, preventing `SQLITE_BUSY` contention
- Wrapped operations: `record_failure()`, `task_done()` + auto-unblock, `iterate()` + `validate()`, `prd_import()`

#### Checkpoint/Rollback System
- `checkpoint_create()`, `checkpoint_restore()`, `checkpoint_drop()` in `git.rs` using `git stash push -u`
- Automatic checkpoint before task execution during `iterate()`; restore on validation failure, drop on success
- `enable_checkpoints` config key (default: true)
- New events: `CheckpointCreated`, `CheckpointRestored`, `CheckpointDropped`

#### Structured Subagent Signals
- New `SubagentSignal` enum: `Complete{summary}`, `Blocked{reason}`, `Learning{category, description}`
- Subagents write `.dial/signal.json` instead of printing `DIAL_` lines to stdout
- Orchestrator reads `signal.json` first, falls back to regex parsing for backward compatibility
- `#[serde(tag = "type", rename_all = "snake_case")]` for clean tagged-union JSON serialization

### Tier 2 — Intelligence

#### Solution Auto-Suggestion
- `find_solutions_for_pattern()` queries trusted solutions (confidence >= 0.6) for matched failure patterns
- `SolutionSuggested` event emitted with failure ID and matching solutions
- Context assembly includes known fixes at priority 15: `KNOWN FIX (confidence: 0.85): description`
- Successful validation after a suggestion auto-increments solution confidence by +0.15

#### Cross-Iteration Failure Tracking
- Tasks track `total_attempts`, `total_failures`, `last_failure_at` across all iterations
- `Engine::chronic_failures(threshold)` returns tasks exceeding failure threshold
- Auto-run auto-blocks tasks exceeding `max_total_failures` config (default: 10)
- `ChronicFailureDetected` event variant
- CLI: `dial task chronic --threshold N`

#### Similar Completed Task Context
- FTS query against `tasks_fts` finds completed tasks matching keywords from current task description
- Stop word stripping (the, a, an, is, for, to, of, in, and, or) for better search relevance
- Context includes up to 3 similar tasks at priority 25: `SIMILAR COMPLETED TASK: {desc}\nApproach: {notes}\nCommit: {hash}`

#### Learning-to-Pattern Linking
- `learn()` accepts optional `pattern_id` and `iteration_id` parameters
- Auto-links learnings to failure patterns when called during an iteration with recorded failures
- `Engine::learnings_for_pattern(pattern_id)` queries linked learnings
- Context assembly includes pattern-linked learnings at priority 35
- CLI: `dial learnings list --pattern <id>`

### Tier 3 — Analytics & Observability

#### Per-Pattern Metrics
- `PatternMetrics` struct: occurrences, resolution times, token/cost consumption, auto/manual/unresolved counts
- `compute_pattern_metrics()` joins failures, iterations, provider_usage, and solutions tables
- CLI: `dial patterns metrics` with table output, `--format json`, `--sort` (cost/time/occurrences)

#### Dry Run / Preview Mode
- `DryRunResult` struct shows what would happen without creating iteration records or spawning subagents
- Fields: task, context items included/excluded with token sizes, prompt preview (first 500 chars), suggested solutions, dependency status
- CLI: `dial iterate --dry-run [--format json]`, `dial auto-run --dry-run`

#### Project Health Score
- `compute_health()` with 6 weighted factors: success rate (0.30), success trend (0.15), solution confidence (0.15), blocked task ratio (0.15), learning utilization (0.10), pattern resolution rate (0.15)
- `Trend` enum: `Improving`, `Stable`, `Declining` (compares current vs 7 days ago)
- Color-coded output: green (>= 70), yellow (>= 40), red (< 40)
- CLI: `dial health [--format json]`

### Database Migration
- **Migration 11**: adds `total_attempts`, `total_failures`, `last_failure_at` columns to `tasks` table; adds `pattern_id` and `iteration_id` columns to `learnings` table (all nullable/defaulted)

### Testing
- 308 total tests (up from 201)
- New unit tests: transaction commit/rollback, pattern metrics aggregation, signal parsing, health score computation, solution auto-suggestion, chronic failure detection
- New integration tests: checkpoint create/restore/drop cycle, learning-to-pattern auto-linking, per-pattern metrics, dry run no-side-effects verification, full health score lifecycle

---

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
