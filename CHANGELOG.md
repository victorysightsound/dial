# Changelog

## 4.2.6 — 2026-03-30

Built-in install management release.

### Upgrade and Uninstall Commands
- adds `dial upgrade` so DIAL can upgrade itself for Cargo installs, npm installs, and direct binary installs
- adds `dial uninstall` so users can remove the CLI without touching project `.dial/` directories
- keeps direct-binary upgrades aligned with GitHub releases by downloading the matching platform asset automatically

### Windows Install Management
- adds a Windows follow-up console flow for `dial upgrade` and `dial uninstall` so DIAL can replace or remove `dial.exe` after the current process exits
- avoids the common Windows locked-executable failure mode when users try to self-update or self-remove the CLI

### Documentation
- documents the new self-managed upgrade and uninstall flow in the README, getting started guide, and CLI reference
- keeps the install instructions aligned across Cargo, npm, direct binaries, macOS, Linux, and Windows

No schema migrations needed.

---

## 4.2.5 — 2026-03-30

npm publish correction and release alignment patch.

### npm Distribution
- publishes the npm package as `@victorysightsound/dial-cli` after npm rejected the unscoped `dial-cli` name as too similar to an existing package
- keeps the global command name as `dial` while using the scoped package name for installation and upgrades
- verifies the public package path end to end with authenticated npm publish, registry checks, and install documentation updates

### Documentation Accuracy
- updates README, getting started, and npm package docs to use the correct scoped install command
- corrects the `4.2.4` release notes so they describe the npm distribution groundwork accurately instead of implying a successful public npm publish

No schema migrations needed.

---

## 4.2.4 — 2026-03-30

Installation and distribution patch.

### npm Distribution
- adds npm distribution scaffolding in the repository so the package can download the matching GitHub release binary for the current platform and expose `dial` as the global command
- validates the npm wrapper with package-level tests and a real packed tarball install that runs `dial --version`
- adds optional npm publish automation to the tag release workflow when `NPM_TOKEN` is configured

### Installation Guidance
- expands README and getting started installation docs across GitHub binaries, npm, Cargo, and source builds
- clarifies that DIAL is installed globally while `.dial/` remains per-project working data
- adds more explicit Windows PATH guidance, including where to place `dial.exe`, how PATH works, and when to reopen the terminal
- updates the Unix installer script messaging so the global install scope and PATH step are clearer

No schema migrations needed.

---

## 4.2.3 — 2026-03-30

Agent file mode and release-alignment patch.

### Agent File Handling
- adds explicit `--agents local|shared|off` handling to both `dial init` and `dial new`
- makes `local` the default so DIAL creates `AGENTS.md` for local AI tooling without polluting normal `git status`
- uses `.git/info/exclude` for local mode so `/AGENTS.md`, `/CLAUDE.md`, and `/GEMINI.md` stay local-only by default
- keeps `shared` available for teams that want to commit agent instruction files intentionally
- keeps `off` available for users who do not want agent instruction files created at all
- preserves `dial init --no-agents` as a compatibility alias for `--agents off`

### Commit Hygiene
- excludes newly created top-level agent instruction files from validation and auto-run task commits unless they already exist in `HEAD`
- keeps tracked agent instruction files commitable when the user intentionally maintains them in the repository

### Documentation & Release Accuracy
- updates README, getting started, configuration, architecture, and CLI reference docs to explain the new agent-file modes and default behavior
- fixes the exported CLI/library version constant so the binary reports the shipped version correctly

### Verification
- adds unit coverage for local/shared/off agent-file setup behavior
- adds regression coverage for excluding new top-level agent files from task commits while preserving tracked agent file changes
- validates native Windows `dial init` behavior for `local`, `shared`, and `off` on `gbi-video`
- validates local macOS `dial init` behavior for `local`, `shared`, and `off`
- validates a fresh local macOS `dial new --template mvp --wizard-backend codex --agents local` run through all 9 wizard phases

No schema migrations needed.

---

## 4.2.2 — 2026-03-29

Guided wizard trust and observability release.

### Guided Wizard UX
- adds startup orientation for both `dial new` and `dial spec wizard` so users are told what the wizard will do, what it will not do, and how to resume safely
- adds plain-English narration for all nine wizard phases so the runtime output reads like guidance instead of only diagnostics
- keeps technical prompt/timing diagnostics available while demoting them behind the new guidance layer

### Wizard Trust Messaging
- emits long-wait heartbeat events while provider-backed phases are still running so quiet Windows terminal periods feel active instead of hung
- adds a planning checkpoint after phase 5 in the full wizard to reinforce that DIAL is still planning and has not started implementation
- strengthens launch and PRD-only completion messaging so it is explicit that `dial auto-run` is always a separate user-triggered step

### Windows Auto-Run Hardening
- resolves `--from` source documents before the wizard hands work to the backend so native Windows runs do not waste time trying to rediscover the original scenario file from a temp workdir
- hardens `.dial/signal.json` reads against transient empty-file races and BOM-prefixed content so Windows subagent completion signals are consumed reliably
- makes validation and auto-run fail loudly when a post-validation git commit cannot be created instead of falsely marking tasks complete
- restores missing local git author identity from the latest commit author during validation and auto-run so seeded fixture repos can commit cleanly on Windows smoke machines
- normalizes autonomous task commit subjects into short human commit messages instead of raw task prose
- filters brittle optional inline `node -e` validation steps for local Windows Node.js projects so the generated pipeline stays on `build` and `test` instead of noisy shell-specific preflights

### Verification
- adds unit coverage for wizard orientation, phase presentation, and heartbeat behavior
- adds integration coverage for full-wizard checkpoint emission and PRD-only completion semantics
- adds regression coverage for signal-file retry/BOM parsing, commit-failure rollback, commit-subject normalization, and phase-7 Windows pipeline filtering
- updates README and CLI reference to match the new guided-default wizard behavior
- validates a full native Windows seeded auto-run on `gbi-video` with `dial new --template spec --from ..\\windows-e2e-autorun-scenario.md --wizard-backend codex` followed by `dial auto-run --cli codex`

No schema migrations needed.

---

## 4.2.1 — 2026-03-28

Wizard stability and Windows validation patch release.

### Native Windows Backend Hardening
- hardens native Windows CLI backend execution for `codex` and `copilot`
- routes structured Codex wizard prompts through stdin and schema files so long prompts do not hit Windows command-line length limits
- runs wizard subprocesses from a neutral temporary working directory instead of the project root to avoid Windows CLI shim edge cases
- keeps Copilot and Codex structured-output flows reliable in native Windows runs

### Wizard Quality & Test Hardening
- tightens Copilot-facing wizard prompts and quality checks so generated PRD sections and task lists stay concrete instead of drifting into placeholder content
- allows legitimate explanatory mentions of tokens like `TODO` and `TBD` inside generated PRD prose while still rejecting actual placeholder responses
- fixes the parallel test current-working-directory race so the workspace suite is reliable under normal parallel execution again

### Verification
- fresh native Windows runs completed for `dial new --template mvp --wizard-backend codex`
- fresh native Windows runs completed for `dial new --template mvp --wizard-backend copilot`
- workspace test suite passed with 416 tests

No schema migrations needed.

---

## 4.2.0 — 2026-03-28

Wizard backend and Mac hardening release.

### Wizard Backend Selection
- adds shared backend resolution for both `dial new` and `dial spec wizard`
- supports `codex`, `claude`, `copilot`, `gemini`, and `openai-compatible` wizard backends
- uses the active session backend when it can be detected and requires an explicit choice when multiple backends are installed but no clear hint exists
- adds `--wizard-backend` and `--wizard-model` flags for both wizard entry points

### Wizard Reliability & Visibility
- emits clearer phase progress, prompt size, timing, and launch summary output during wizard runs
- fixes resume behavior so paused `dial new --resume` runs reopen the existing project instead of reinitializing it
- updates wizard state in place, fixes generated task linkage to `prd_section_id`, and keeps later phases resumable
- adds JSON repair and regenerate recovery paths so malformed backend responses no longer abort the wizard immediately

### CLI Backend Hardening
- adds GitHub Copilot CLI support across wizard and auto-run paths
- tunes Codex CLI passthrough for noninteractive wizard calls by disabling web search and lowering reasoning/verbosity
- prefers the chosen wizard CLI when writing `ai_cli` during phase 8 so launch configuration matches the backend that completed the wizard

### Verification
- fresh Mac end-to-end runs completed for `dial new --template mvp --wizard-backend copilot`
- fresh Mac end-to-end runs completed for default `dial new --template mvp` with automatic Codex selection
- fresh Mac end-to-end runs completed for `dial spec wizard --template mvp --wizard-backend copilot`
- workspace test suite passed with 399 tests

No schema migrations needed.

---

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
