# DIAL Implementation History

Current official npm package: `getdial`

Earlier release notes below still mention `@victorysightsound/dial-cli` where that was the published package at the time.

## Version Timeline

### v4.2.6 (March 2026) - Built-In Install Management

Patch release focused on letting DIAL manage its own lifecycle across supported install methods:

- Adds `dial upgrade` for Cargo installs, npm installs, and direct binary installs
- Adds `dial uninstall` so users can remove the CLI without touching project `.dial/` state
- Adds a Windows follow-up console flow so self-updates and self-removal can complete after the running `dial.exe` exits
- Documents the new upgrade and uninstall workflow across README, getting started, and CLI reference docs

No schema migrations needed.

### v4.2.5 (March 2026) - Scoped npm Publish Correction

Patch release focused on shipping the public npm package cleanly after npm rejected the original unscoped name:

- Publishes the npm package as `@victorysightsound/dial-cli` while keeping the installed command name as `dial`
- Updates README, getting started, and package docs to use the correct scoped install command
- Corrects the `4.2.4` release notes so they describe npm groundwork accurately rather than implying a completed public publish
- Verifies the scoped npm package through authenticated publish and registry checks

No schema migrations needed.

### v4.2.4 (March 2026) - npm Distribution & Install Clarity

Patch release focused on making DIAL easier to install across platforms and adding npm as a supported distribution channel:

- Adds npm distribution scaffolding so the package can download the matching GitHub release binary for the current platform and expose `dial` as the global command
- Adds release-workflow support for npm publication when `NPM_TOKEN` is configured
- Expands installation guidance across README and getting started so users understand global install scope, PATH behavior, Windows setup, and the difference between the global CLI install and per-project `.dial/` state
- Validates the npm wrapper with package-level tests plus a real packed tarball install that successfully runs `dial --version`

No schema migrations needed.

### v4.2.3 (March 2026) - Agent File Modes & Release Alignment

Patch release focused on making agent instruction files predictable for end users and keeping release metadata aligned with shipped behavior:

- Adds explicit `--agents local|shared|off` handling to both `dial init` and `dial new`
- Makes `local` the default so DIAL creates `AGENTS.md` for local AI tooling while hiding `/AGENTS.md`, `/CLAUDE.md`, and `/GEMINI.md` through `.git/info/exclude`
- Keeps `shared` available for repositories that want to commit agent instruction files intentionally
- Keeps `off` available for users who do not want agent instruction files created at all
- Excludes newly created top-level agent instruction files from validation and auto-run task commits unless they already exist in `HEAD`
- Fixes the exported CLI/library version constant so the binary reports the actual shipped version
- Verifies `local`, `shared`, and `off` behavior on both macOS and native Windows, plus a fresh local macOS 9-phase `dial new` wizard run

No schema migrations needed.

### v4.2.2 (March 2026) - Guided Wizard Trust & Windows Auto-Run Hardening

Patch release focused on making the guided wizard read like a guided operator and proving the full Windows autonomous loop in a seeded fixture repo:

- Adds startup orientation, plain-English phase narration, long-wait heartbeats, a phase-5 planning checkpoint, and stronger launch/PRD-only trust messaging
- Resolves `--from` source documents before backend execution so native Windows runs stop searching temp directories for the original scenario file
- Hardens `.dial/signal.json` reads against transient empty-file races and BOM-prefixed content so Windows subagent completion signals remain reliable
- Makes validation and auto-run fail loudly when the final git commit cannot be created and restores missing git author identity from the latest commit author when possible
- Normalizes autonomous task commit subjects into short human commit messages
- Filters brittle optional inline `node -e` pipeline steps for local Windows Node.js projects so generated validation stays on stable `build` / `test` gates
- Validates a full native Windows seeded run on `gbi-video` with `dial new --template spec --from ..\\windows-e2e-autorun-scenario.md --wizard-backend codex` followed by `dial auto-run --cli codex`

No schema migrations needed.

### v4.2.1 (March 2026) - Windows Hardening & Wizard Quality

Patch release focused on proving the wizard in native Windows CLI runs and tightening the remaining quality edges:

- Hardens native Windows CLI backend execution for `codex` and `copilot`
- Routes long structured Codex prompts through stdin/schema files so Windows command-line length limits do not break later wizard phases
- Runs wizard subprocesses from a neutral temporary working directory to avoid Windows CLI shim and npm launcher edge cases
- Tightens Copilot-facing wizard prompts and placeholder detection so generated PRD sections and task lists stay concrete
- Fixes the parallel test current-working-directory race so the full workspace suite is reliable under normal parallel execution again

416 tests. No schema migrations needed.

### v4.2.0 (March 2026) - Wizard Backend & Mac Hardening

Feature release focused on making the project wizard backend-agnostic, observable, and resilient in real CLI-backed runs:

- Shared wizard backend resolution for both `dial new` and `dial spec wizard`
- Supports `codex`, `claude`, `copilot`, `gemini`, and `openai-compatible`
- Uses the active session backend when detectable and otherwise requires an explicit backend if multiple CLIs are installed
- Adds phase progress diagnostics, launch summaries, and backend-specific hardening for Copilot and Codex
- Fixes `dial new --resume`, persistent wizard state updates, and phase-5 linked task creation
- Adds JSON repair/regenerate recovery so malformed backend output can self-heal instead of aborting the run

399 tests. No schema migrations needed.

### v4.1.3 (March 2026) - Documentation Alignment

Patch release to bring the top-level changelog back in sync with shipped releases:

- Adds `4.1.1` and `4.1.2` entries to `CHANGELOG.md`
- Publishes a canonical release so the repository, GitHub release, crates.io packages, and local installs all reflect the same documentation state
- No runtime behavior changes

No schema migrations needed.

### v4.1.2 (March 2026) - Release Alignment & Publishability

Patch release to align all distribution channels on one canonical revision:

- Rolls the crates.io publish metadata fix into an official tagged release
- Keeps GitHub release assets, crates.io packages, `main`, and local installs aligned on the same version
- No runtime behavior changes beyond the Unicode-dash hardening shipped in v4.1.1

No schema migrations needed.

### v4.1.1 (March 2026) - Command Input Hardening

Patch release focused on preventing Unicode dash characters from breaking command execution:

- Normalizes obvious Unicode dash flag input in `build_cmd`, `test_cmd`, and validation pipeline commands
- Re-checks commands before execution so existing bad config values no longer block `dial validate`
- Adds prompt guidance telling AI providers to use ASCII hyphen-minus characters in commands, flags, JSON, and code
- Covers config writes, wizard-generated commands, manual pipeline steps, and execution-time validation with regression tests

No schema migrations needed.

### v4.1.0 (March 2026) — Loop Accuracy & Hardening

7 enhancements targeting first-attempt success rate, spec quality, and reliability:

**Failed Attempt Diff Capture:**
- Captures `git diff` and `git diff --stat` before checkpoint restore on validation failure
- On retry (attempt > 1), includes previous failed diff at context priority 12
- Prevents the AI from repeating the same failed approach

**Spec Specificity Enforcement (Phase 4):**
- Phase 4 (Gap Analysis) now includes a SPECIFICITY CHECK pass
- Rates each PRD section as SPECIFIC, NEEDS_DETAIL, or VAGUE
- Rewrites VAGUE sections with concrete acceptance criteria before Phase 5 proceeds

**Task Sizing Analysis (Phase 6):**
- Phase 6 (Task Review) now evaluates scope, specificity, and testability per task
- Splits tasks touching more than 3 files into smaller focused tasks with dependencies
- Rewrites vague descriptions, merges tasks too small for a separate iteration
- Annotates sizing: [S]mall, [M]edium, [L]arge, [XL] needs review

**Test Coverage Generation (Phase 7):**
- Phase 7 (Build & Test) now generates dedicated test tasks paired with feature tasks
- Test tasks depend on their feature tasks in the dependency graph
- Suggests test framework and validation pipeline steps based on tech stack

**Checkpoint Conflict Recovery:**
- `checkpoint_restore()` now recovers from `git stash pop` merge conflicts
- Falls back to `git reset --hard HEAD` + `git stash drop` to guarantee a clean working tree
- Prevents error-out when user makes manual commits between iterate and validate

**Secret Detection Before Staging:**
- Pre-commit safety check scans staged files against 13 dangerous patterns (`.env`, `.pem`, `.key`, `id_rsa`, `credentials.json`, etc.)
- Automatically unstages flagged files and warns before committing
- Prevents accidental secret commits from `git add -A`

**Resilient JSON Parsing (Wizard):**
- 4-attempt extraction strategy: markdown-aware → brute-force brace matching → re-prompt AI → brute-force retry
- Handles AI responses wrapped in explanatory text, nested objects, escaped quotes

364 tests (up from 308). No schema migrations needed.

### v4.0.0 (March 2026) — Engine Hardening

10 architectural improvements across three tiers:

**Tier 1 — Reliability:**
- Transaction safety with `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` wrappers on all multi-step DB operations
- Checkpoint/rollback system using `git stash` for atomic iterations
- Structured subagent signals via `.dial/signal.json` (with regex fallback for backward compat)
- Solution auto-suggestion: trusted solutions actively surfaced when matching failure patterns recur

**Tier 2 — Intelligence:**
- Cross-iteration failure tracking with cumulative attempt/failure counters and chronic failure auto-blocking
- Similar completed task context via FTS search for proven approaches
- Per-pattern metrics aggregating cost, time, and resolution rates by failure pattern
- Learning-to-pattern linking so learnings auto-surface when their associated pattern recurs

**Tier 3 — Usability:**
- Dry run / preview mode (`dial iterate --dry-run`) showing task selection, context assembly, and prompt without execution
- Project health score (`dial health`) with 6 weighted factors and trend detection

308 tests. Migration 11 adds columns to tasks and learnings tables.

### v3.2.0 (March 2026) — Unified Project Wizard

Rewrites the 5-phase PRD wizard into a 9-phase guided flow (`dial new`) covering init through autonomous iteration:

- Phases 1-5: Vision, Functionality, Technical, Gap Analysis, Generate (existing)
- Phase 6: Task Review — AI reorders, deduplicates, adds dependency relationships
- Phase 7: Build & Test Config — AI suggests build/test commands and pipeline steps
- Phase 8: Iteration Mode — configures autonomous, review_every:N, or review_each
- Phase 9: Launch Summary — prints project summary, ready for `dial auto-run`

201 tests. Fix for nested Claude Code sessions (`CLAUDECODE` env var removal).

### v3.1.0 (March 2026) — PRD Wizard & Structured Spec Database

Adds standalone `prd.db` with hierarchical sections, terminology, and AI-assisted wizard:

- Separate PRD database alongside phase database
- Hierarchical sections with dotted notation (1, 1.1, 1.2.1)
- FTS5 full-text search with porter tokenizer
- Terminology tracking with canonical terms and variants
- 5-phase AI wizard: Vision, Functionality, Technical, Gap Analysis, Generate
- 4 templates: spec, architecture, api, mvp
- Pause/resume with state persisted to prd.db

142 tests.

### v3.0.0 (March 2025) — Rust Workspace Rewrite

Complete ground-up rewrite as a Rust workspace with embeddable library:

- **Workspace structure:** `dial-core` (library), `dial-cli` (binary), `dial-providers` (AI backends)
- **Engine struct:** Central async API wrapping all operations via tokio
- **10 sequential SQLite migrations** auto-applied on database open
- Task dependencies with topological sort and cycle detection
- Event system with `EventHandler` trait for lifecycle notifications
- Provider abstraction with `Provider` trait for pluggable AI backends
- Configurable validation pipeline with per-step timeouts
- Token budget management with priority-ranked context assembly
- DB-driven failure patterns (21 seeded, plus clustering for new pattern discovery)
- Solution provenance with confidence decay and history tracking
- Approval gates (auto, review, manual modes)
- Metrics with daily trend aggregation and JSON/CSV export
- Crash recovery (`dial recover`)

115 tests.

### v2.2.0 (February 2026) — Automated Orchestration

Added `dial auto-run` for fully automated orchestration with fresh AI subprocesses per task. Supports Claude Code, Codex CLI, and Gemini CLI. 25 CLI commands.

### v2.1.0 (February 2026) — Behavioral Guardrails

Added behavioral "signs" (6 guardrails included in every context), `dial context` for regeneration, `dial orchestrate` for sub-agent prompts. 24 commands.

### v2.0.0 (February 2026) — Initial Rust Rewrite

Complete rewrite from Python to Rust. 13x startup improvement (~190ms Python to ~14ms Rust). Single 4MB static binary. 22 commands with identical behavior to Python.

## Architecture Evolution

| Version | Structure | Async | Tests |
|---------|-----------|-------|-------|
| 2.0-2.2 | Single crate (`dial/`) | No (sync) | 15 |
| 3.0.0 | Workspace (core + cli + providers) | Yes (tokio) | 115 |
| 3.1.0 | + PRD database | Yes | 142 |
| 3.2.0 | + 9-phase wizard | Yes | 201 |
| 4.0.0 | + Engine hardening | Yes | 308 |
| 4.1.0 | + Loop accuracy, hardening & spec enforcement | Yes | 364 |
| 4.1.1 | + Command input hardening | Yes | 364+ |
| 4.1.2 | + Release alignment & publishability | Yes | 364+ |
| 4.1.3 | + Documentation alignment | Yes | 364+ |
| 4.2.0 | + Wizard backends, visibility & Mac hardening | Yes | 399 |
| 4.2.1 | + Windows CLI hardening & wizard quality fixes | Yes | 416 |
| 4.2.2 | + Guided wizard trust & Windows auto-run hardening | Yes | 416+ |
| 4.2.3 | + Agent file modes & release alignment | Yes | 416+ |
| 4.2.4 | + npm distribution & install clarity | Yes | 416+ |
| 4.2.5 | + scoped npm publish correction | Yes | 416+ |
| 4.2.6 | + built-in install management | Yes | 416+ |

## Performance

| Metric | v2.0 | v4.2.6 |
|--------|------|------|
| Startup | ~14ms | ~14ms |
| Binary size | 4.0MB | ~5MB |
| Dependencies | None (static) | None (static) |
| Database | SQLite + FTS5 | SQLite + FTS5 (WAL) |
