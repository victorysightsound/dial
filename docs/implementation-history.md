# DIAL Implementation History

## Version Timeline

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

## Performance

| Metric | v2.0 | v4.1 |
|--------|------|------|
| Startup | ~14ms | ~14ms |
| Binary size | 4.0MB | ~5MB |
| Dependencies | None (static) | None (static) |
| Database | SQLite + FTS5 | SQLite + FTS5 (WAL) |
