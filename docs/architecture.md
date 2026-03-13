# Architecture

Technical overview of DIAL's source code for contributors and anyone who wants to understand how it works.

## Workspace Structure

DIAL is a Rust workspace with three crates:

```
dial/
├── Cargo.toml              # Workspace root, shared dependencies
├── dial-core/              # Library crate — embeddable engine
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          # Public API exports, constants
│       ├── engine.rs       # Engine struct — central API for all operations
│       ├── config.rs       # Key-value config management
│       ├── errors.rs       # DialError enum, Result type alias
│       ├── output.rs       # ANSI color output, terminal formatting
│       ├── health.rs       # Project health score computation
│       ├── db/
│       │   ├── mod.rs      # DB connection, init, migrations, path helpers
│       │   └── schema.rs   # SQL schema, 11 sequential migrations
│       ├── task/
│       │   └── mod.rs      # Task CRUD, dependencies, topological sort
│       ├── spec/
│       │   ├── mod.rs      # Legacy spec indexing, search, display
│       │   └── parser.rs   # Markdown section parser
│       ├── prd/
│       │   ├── mod.rs      # PRD database management
│       │   ├── import.rs   # Markdown import into prd.db
│       │   ├── wizard.rs   # 9-phase AI wizard
│       │   └── templates.rs # 4 PRD templates (spec, architecture, api, mvp)
│       ├── failure/
│       │   ├── mod.rs      # Failure recording, solution auto-suggestion
│       │   ├── patterns.rs # 21 regex patterns, DB-driven pattern matching
│       │   └── solutions.rs # Trust scoring, confidence decay, history
│       ├── iteration/
│       │   ├── mod.rs      # Core loop: iterate, validate, checkpoint
│       │   ├── context.rs  # Context assembly, behavioral signs, prompts
│       │   ├── orchestrator.rs # Auto-run, subprocess spawning, signal parsing
│       │   ├── signal.rs   # Structured JSON signal file (SubagentSignal)
│       │   └── validation.rs # Configurable pipeline execution
│       ├── learning/
│       │   └── mod.rs      # Learning CRUD, pattern linking
│       ├── metrics/
│       │   └── mod.rs      # Metrics computation, trends, export
│       ├── budget/
│       │   └── mod.rs      # Token estimation, priority-ranked assembly
│       ├── event/
│       │   └── mod.rs      # Event enum (40+ variants), EventHandler trait
│       ├── git/
│       │   └── mod.rs      # Git operations, checkpoint/rollback
│       └── provider/
│           └── mod.rs      # Provider trait, request/response types
├── dial-cli/               # Binary crate — CLI interface
│   ├── Cargo.toml
│   └── src/
│       └── main.rs         # Clap arg parsing, command routing
└── dial-providers/         # Provider implementations
    ├── Cargo.toml
    └── src/
        └── lib.rs          # AnthropicProvider, CliPassthrough
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `rusqlite` (0.31) | SQLite bindings with bundled `libsqlite3` and FTS5 |
| `tokio` (1) | Async runtime for all Engine methods |
| `async-trait` (0.1) | Async trait support for Provider |
| `reqwest` (0.12) | HTTP client with rustls-tls (for API providers) |
| `serde` / `serde_json` (1) | Serialization for config, signals, exports |
| `chrono` (0.4) | Timestamps in RFC 3339 format |
| `thiserror` (2) | Error enum derive macros |
| `anyhow` (1) | Error context propagation |
| `regex` (1) | Failure pattern matching |
| `futures` (0.3) | Async stream processing |

All Engine methods are async via tokio. The database layer uses synchronous rusqlite (appropriate for SQLite's single-writer model).

## Data Flow

### Initialization

```
dial init --phase mvp
    │
    ├── Create .dial/ directory
    ├── Create .dial/mvp.db with full schema (14+ tables, triggers, FTS5)
    ├── Run 11 sequential migrations
    ├── Set PRAGMA journal_mode=WAL, busy_timeout=5000
    ├── Insert default config (phase, project_name)
    ├── Write .dial/current_phase = "mvp"
    └── Optionally append DIAL instructions to AGENTS.md
```

### Task Iteration

```
dial iterate
    │
    ├── Open .dial/<current_phase>.db
    ├── SELECT next pending task (respects dependency graph)
    ├── Check attempt count (max 3) and chronic failure count
    ├── Create checkpoint (git stash) if enabled
    ├── BEGIN TRANSACTION
    │   ├── INSERT INTO iterations (task_id, attempt_number, started_at)
    │   ├── UPDATE tasks SET status = 'in_progress', total_attempts += 1
    │   └── COMMIT
    ├── gather_context():
    │   ├── Include behavioral signs
    │   ├── SELECT linked spec/PRD section
    │   ├── FTS search for related specs
    │   ├── IF retry (attempt > 1): SELECT failed diff from previous iteration
    │   │   └── Priority 12: "PREVIOUS ATTEMPT (failed): {error} / {diff}"
    │   ├── SELECT suggested solutions for recent failures
    │   ├── SELECT similar completed tasks
    │   ├── SELECT trusted solutions (confidence >= 0.6)
    │   ├── SELECT pattern-linked learnings
    │   ├── SELECT recent unresolved failures
    │   └── SELECT general learnings (by reference count)
    ├── Write context to .dial/current_context.md
    └── Print "Agent should now implement the task"
```

### Validation

```
dial validate
    │
    ├── Find current in-progress iteration
    ├── Run validation pipeline (ordered steps):
    │   ├── For each step (in sort_order):
    │   │   ├── Execute shell command with timeout
    │   │   ├── Record action + outcome in DB
    │   │   └── If required step fails: abort remaining steps
    │   └── Return ValidationResult
    │
    ├── If all required steps pass:
    │   ├── Drop checkpoint (git stash drop)
    │   ├── git add -A && git commit
    │   ├── UPDATE iterations SET status = 'completed'
    │   ├── UPDATE tasks SET status = 'completed'
    │   ├── Auto-unblock dependent tasks
    │   ├── If solution was suggested: increment confidence
    │   └── Record metrics
    │
    └── If any required step fails:
        ├── Restore checkpoint (git stash pop + checkout)
        ├── detect_failure_pattern(error_text)
        ├── BEGIN TRANSACTION
        │   ├── INSERT/UPDATE failure_patterns
        │   ├── INSERT INTO failures
        │   ├── Find and emit suggested solutions
        │   ├── UPDATE tasks SET total_failures += 1
        │   └── COMMIT
        ├── If attempts < 3: reset task to pending
        └── If attempts >= 3: block task, revert to last good commit
```

### Auto-Run Orchestration

```
dial auto-run --cli claude
    │
    └── Loop:
        ├── Check for .dial/stop file
        ├── Check iteration limit and iteration_mode
        ├── SELECT next pending task (dependency-aware)
        ├── Check chronic failure threshold
        ├── Generate sub-agent prompt with full context
        ├── Write prompt to .dial/subagent_prompt.md
        ├── Spawn subprocess (env_remove CLAUDECODE):
        │   └── claude -p "$(cat .dial/subagent_prompt.md)" 2>&1
        ├── Stream output line by line (prefixed with │)
        ├── Read signals:
        │   ├── Try .dial/signal.json first (structured)
        │   └── Fall back to regex parsing (backward compat)
        ├── Process signals:
        │   ├── Complete → run validation → commit on success
        │   ├── Blocked → block task with reason
        │   └── Learning → record with pattern link if applicable
        ├── Check iteration_mode:
        │   ├── autonomous → continue
        │   ├── review_every:N → pause after Nth completion
        │   └── review_each → pause after every completion
        └── Repeat
```

## Transaction Safety

All multi-step database operations are wrapped in explicit SQLite transactions:

- `record_failure()` — insert failure + update pattern + check solutions
- `task_done()` — update task + auto-unblock dependents
- `iterate()` — create iteration + update task status
- `prd_import()` — multiple section inserts + source tracking

The `with_transaction()` helper in `db.rs` handles BEGIN/COMMIT/ROLLBACK automatically.

## Error Handling

All fallible operations return `Result<T>` where the error type is `DialError`:

```rust
pub enum DialError {
    NotInitialized,
    Database(rusqlite::Error),
    Io(std::io::Error),
    PhaseNotFound(String),
    TaskNotFound(i64),
    SpecSectionNotFound(i64),
    PrdSectionNotFound(String),
    LearningNotFound(i64),
    NoIterationInProgress,
    NoPendingTasks,
    MaxAttemptsExceeded(i64),
    NotGitRepo,
    GitError(String),
    CommandFailed(String),
    CommandTimeout(u64),
    InvalidConfigKey(String),
    InvalidConfig(String),
    SpecsDirNotFound(String),
    WizardError(String),
    TemplateNotFound(String),
    ProviderRequired,
    UserError(String),
}
```

## Signal System

DIAL supports two signal formats for subagent communication:

**Preferred: Structured JSON** (`.dial/signal.json`):
```json
{
  "signals": [
    {"type": "complete", "summary": "Implemented user login with bcrypt"},
    {"type": "learning", "category": "build", "description": "bcrypt requires Node 18+"}
  ],
  "timestamp": "2026-03-12T15:30:00Z"
}
```

**Fallback: Regex parsing** (for backward compatibility):
```
DIAL_COMPLETE: summary of what was done
DIAL_BLOCKED: reason the task can't be completed
DIAL_LEARNING: category: what was learned
```

The orchestrator tries the signal file first, falls back to regex if the file doesn't exist.

## Testing

```bash
cargo test --workspace
```

354 tests covering:
- Unit tests for all core modules (patterns, signals, health, metrics, budget, diffs)
- Integration tests for full lifecycle (init → task → iterate → validate → complete)
- Wizard tests (9-phase flow with mock provider, pause/resume, specificity/sizing/coverage)
- Crash recovery, transaction rollback, and failed diff capture tests

## Building

```bash
# Debug build
cargo build --workspace

# Release build (optimized, ~5MB)
cargo build --workspace --release
```

Release profile uses LTO, single codegen unit, and symbol stripping for minimal binary size.
