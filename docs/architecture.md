# Architecture

Technical overview of DIAL's source code for contributors and anyone who wants to understand how it works.

## Crate Structure

DIAL is a single Rust crate that produces both a binary and a library.

```
dial/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry point, clap arg parsing, command routing
│   ├── lib.rs               # Public API exports, constants
│   ├── config.rs            # Key-value config management
│   ├── errors.rs            # DialError enum, Result type alias
│   ├── output.rs            # ANSI color output, terminal formatting
│   ├── db/
│   │   ├── mod.rs           # DB connection, init, migration, path helpers
│   │   └── schema.rs        # SQL schema constant (CREATE TABLE statements)
│   ├── task/
│   │   ├── mod.rs           # Task CRUD operations (add, list, done, block, etc.)
│   │   └── models.rs        # Task struct, TaskStatus enum, FromRow impl
│   ├── spec/
│   │   ├── mod.rs           # Spec indexing, search, display
│   │   └── parser.rs        # Markdown section parser
│   ├── failure/
│   │   ├── mod.rs           # Failure recording, display, trusted solution lookup
│   │   ├── patterns.rs      # 21 regex patterns, detect_failure_pattern()
│   │   └── solutions.rs     # Solution CRUD, trust scoring, application tracking
│   ├── iteration/
│   │   ├── mod.rs           # Core loop: iterate_once, validate_current, run_loop
│   │   ├── context.rs       # Context assembly, behavioral signs, subagent prompts
│   │   ├── orchestrator.rs  # Auto-run: subprocess spawning, signal parsing
│   │   └── validation.rs    # Build/test execution, action/outcome recording
│   ├── learning/
│   │   └── mod.rs           # Learning CRUD, reference counting
│   └── git/
│       └── mod.rs           # Git operations (commit, revert, status checks)
└── tests/
    └── integration/         # Integration tests
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` (4) | CLI argument parsing with derive macros |
| `rusqlite` (0.31) | SQLite bindings with bundled `libsqlite3` |
| `serde` / `serde_json` (1) | Serialization (used for config) |
| `chrono` (0.4) | Timestamps in RFC 3339 format |
| `thiserror` (1) | Error enum derive macros |
| `anyhow` (1) | Error context propagation |
| `regex` (1) | Failure pattern matching |
| `lazy_static` (1.4) | Compile-time static regex patterns |
| `dirs` (5) | Platform-appropriate paths |
| `walkdir` (2) | Recursive directory traversal for spec indexing |

DIAL is fully synchronous - no async runtime.

## Data Flow

### Initialization

```
dial init --phase mvp
    │
    ├── Create .dial/ directory
    ├── Create .dial/mvp.db with full schema (14 tables, triggers, FTS5)
    ├── Set PRAGMA journal_mode=WAL, busy_timeout=5000
    ├── Insert default config (phase, project_name)
    ├── Write .dial/current_phase = "mvp"
    └── Optionally append DIAL instructions to AGENTS.md
```

### Task Iteration

```
dial iterate
    │
    ├── get_db(None)  →  Open .dial/<current_phase>.db
    ├── SELECT next pending task (ORDER BY priority, id)
    ├── Check attempt count (max 3)
    ├── INSERT INTO iterations (task_id, attempt_number, started_at)
    ├── UPDATE tasks SET status = 'in_progress'
    ├── gather_context():
    │   ├── Include behavioral signs
    │   ├── SELECT linked spec section
    │   ├── FTS search for related specs
    │   ├── SELECT trusted solutions (confidence >= 0.6)
    │   ├── SELECT recent unresolved failures
    │   └── SELECT learnings (sorted by reference count)
    ├── Write context to .dial/current_context.md
    └── Print "Agent should now implement the task"
```

### Validation

```
dial validate
    │
    ├── Find current in-progress iteration
    ├── Run build_cmd:
    │   ├── INSERT INTO actions (type = "build")
    │   ├── Execute shell command with timeout
    │   └── INSERT INTO outcomes (success, output, duration)
    ├── Run test_cmd:
    │   ├── INSERT INTO actions (type = "test")
    │   ├── Execute shell command with timeout
    │   └── INSERT INTO outcomes (success, output, duration)
    │
    ├── If all pass:
    │   ├── git add -A && git commit
    │   ├── UPDATE iterations SET status = 'completed', commit_hash = ...
    │   └── UPDATE tasks SET status = 'completed'
    │
    └── If any fail:
        ├── detect_failure_pattern(error_text)
        ├── INSERT/UPDATE failure_patterns (increment count)
        ├── INSERT INTO failures (pattern_id, error_text)
        ├── Find trusted solutions for this pattern
        ├── UPDATE iterations SET status = 'failed'
        ├── If attempts < 3: UPDATE tasks SET status = 'pending'
        └── If attempts >= 3: block task, git revert to last good commit
```

### Auto-Run Orchestration

```
dial auto-run --cli claude
    │
    └── Loop:
        ├── Check for .dial/stop file
        ├── Check iteration limit
        ├── SELECT next pending task
        ├── Check attempt count
        ├── INSERT INTO iterations
        ├── generate_subagent_prompt():
        │   ├── gather_context() with signs
        │   └── Add DIAL signal instructions
        ├── Write prompt to .dial/subagent_prompt.md
        ├── Spawn subprocess:
        │   └── claude -p "$(cat .dial/subagent_prompt.md)" 2>&1
        ├── Stream output line by line
        ├── Parse DIAL signals:
        │   ├── DIAL_COMPLETE: → mark complete, run validation
        │   ├── DIAL_BLOCKED: → block task
        │   └── DIAL_LEARNING: → record learning
        ├── On DIAL_COMPLETE + validation pass:
        │   ├── git commit
        │   ├── Complete iteration and task
        │   └── Continue to next task
        └── On failure:
            ├── Record failure pattern
            ├── Reset task to pending
            └── Continue (retry on next loop)
```

## Error Handling

All fallible operations return `Result<T>` where the error type is `DialError`:

```rust
pub enum DialError {
    NotInitialized,          // .dial/current_phase doesn't exist
    Database(rusqlite::Error),
    Io(std::io::Error),
    PhaseNotFound(String),
    TaskNotFound(i64),
    SpecSectionNotFound(i64),
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
    UserError(String),
}
```

Errors use `thiserror` for automatic `Display` implementations. The `main()` function catches errors, prints them in red, and exits with code 1.

## Signal Parsing

The `SubagentResult::parse()` function in `orchestrator.rs` extracts DIAL signals from AI output. It uses regex patterns that handle common formatting variations:

```
Pattern: (?i)[\*`]*DIAL[_\s]COMPLETE[\*`:]+\s*(.+)

Matches:
  DIAL_COMPLETE: message
  DIAL COMPLETE: message
  **DIAL_COMPLETE:** message
  `DIAL_COMPLETE:` message
  dial_complete: message
```

The parser filters out:
- Template placeholders (lines containing `<summary>`, `<reason>`, etc.)
- Instruction lines (containing "output:" or "output `")
- Header lines (starting with `#`)

This prevents the AI's echo of DIAL's own instructions from being parsed as actual signals.

## Testing

```bash
cd dial
cargo test
```

Tests cover:
- Failure pattern detection (4 tests in `failure/patterns.rs`)
- DIAL signal parsing (11 tests in `iteration/orchestrator.rs`)
  - Basic signal formats
  - Markdown formatting variations
  - Case insensitivity
  - Multiple signals in one output
  - Template placeholder filtering
  - Instruction line filtering

## Building

```bash
# Debug build (fast compile, slower runtime)
cargo build

# Release build (optimized, ~4MB)
cargo build --release
```

Release profile uses LTO, single codegen unit, and symbol stripping for minimal binary size.

## Adding New Failure Patterns

To add a new failure pattern, edit `src/failure/patterns.rs`:

```rust
lazy_static! {
    pub static ref FAILURE_PATTERNS: Vec<FailurePattern> = vec![
        // ... existing patterns ...

        // Add yours:
        FailurePattern::new(r"your_regex_pattern", "PatternName", "category"),
    ];
}
```

Categories should be one of: `import`, `syntax`, `runtime`, `test`, `build`, or a new category if none fit. Patterns are case-insensitive by default.

## Adding New CLI Commands

1. Add the command variant to the `Commands` enum in `main.rs`
2. Add the match arm in `run_command()`
3. Implement the function in the appropriate module
4. Export it from `lib.rs` if it should be part of the public API
