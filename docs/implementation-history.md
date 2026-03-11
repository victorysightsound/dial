# DIAL Rust Implementation

## Status: COMPLETE (Feb 2026)

**Current Version:** 2.2.0

## Overview

DIAL (Deterministic Iterative Agent Loop) rewritten from Python to Rust for:
- Faster startup (~14ms vs ~190ms Python - 13x improvement)
- Single binary distribution (4.0MB, no runtime dependencies)
- Alignment with preferred tech stack (Rust core)

## Version History

### v2.2.0 (Feb 2026) - Automated Orchestration
- Added `dial auto-run` for fully automated orchestration with fresh AI subprocesses
- Supports Claude Code, Codex CLI, and Gemini CLI
- Parses DIAL signals: `DIAL_COMPLETE`, `DIAL_BLOCKED`, `DIAL_LEARNING`
- Total commands: 25

### v2.1.0 (Feb 2026) - Behavioral Guardrails and Context Regeneration
- Added behavioral "signs" (guardrails) to context output
- Added `dial context` for fresh context regeneration
- Added `dial orchestrate` for sub-agent prompt generation
- Added learning capture prompts after successful validation
- Total commands: 24

### v2.0.0 (Feb 2026) - Initial Rust Rewrite
- Complete rewrite from Python to Rust
- 22 commands with identical behavior to Python

## Implementation Results

| Metric | Python | Rust |
|--------|--------|------|
| Lines of code | 2,271 | ~4,800 |
| Startup time | ~190ms | ~14ms |
| Binary size | N/A | 4.0MB |
| Dependencies | Python 3.x | None (static) |

## File Locations

- **Rust source:** `./dial/`
- **Guide DB:** `./dial_guide.db`

## CLI Commands (25 total)

All commands implemented with identical behavior to Python, plus v2.1/v2.2 additions:

| Command | Subcommands |
|---------|-------------|
| init | (--phase, --import-solutions, --no-agents) |
| index | (--dir) |
| config | set, show |
| task | add, list, next, done, block, cancel, search |
| spec | search, show, list |
| iterate | |
| validate | |
| run | (--max) |
| stop | |
| status | |
| history | (-n) |
| failures | (-a) |
| solutions | (-t) |
| learn | (-c) |
| learnings | list, search, delete |
| stats | |
| revert | |
| reset | |
| context | Fresh context regeneration (v2.1) |
| orchestrate | Sub-agent prompt generation (v2.1) |
| auto-run | Automated orchestration (--max, --cli) (v2.2) |

## Technical Design

### Crate Structure

```
dial/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI entry, clap routing
│   ├── lib.rs            # Public API exports
│   ├── config.rs         # Config key-value management
│   ├── errors.rs         # DialError enum
│   ├── output.rs         # Colored terminal output
│   ├── db/
│   │   ├── mod.rs        # Connection, get_db(), WAL+busy_timeout
│   │   └── schema.rs     # SCHEMA constant, migrations
│   ├── task/
│   │   ├── mod.rs        # Task CRUD, status transitions
│   │   └── models.rs     # Task, TaskStatus
│   ├── spec/
│   │   ├── mod.rs        # Index, search
│   │   └── parser.rs     # Markdown section parsing
│   ├── failure/
│   │   ├── mod.rs        # Failure CRUD
│   │   ├── patterns.rs   # 21 regex patterns
│   │   └── solutions.rs  # Trust scoring
│   ├── iteration/
│   │   ├── mod.rs        # iterate_once(), run loop
│   │   ├── context.rs    # gather_context(), signs, subagent prompts
│   │   ├── orchestrator.rs # auto_run(), subprocess spawning
│   │   └── validation.rs # run_build(), run_test()
│   ├── learning/
│   │   └── mod.rs        # Learning CRUD
│   └── git/
│       └── mod.rs        # Git operations
└── tests/
    └── integration/
```

### Dependencies

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }
rusqlite = { version = "0.31", features = ["bundled", "functions"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1"
anyhow = "1"
regex = "1"
lazy_static = "1.4"
dirs = "5"
walkdir = "2"
```

No async/tokio - DIAL is synchronous.

### Constants

```rust
pub const VERSION: &str = "2.2.0";
pub const MAX_FIX_ATTEMPTS: u32 = 3;
pub const TRUST_THRESHOLD: f64 = 0.6;
pub const TRUST_INCREMENT: f64 = 0.15;
pub const TRUST_DECREMENT: f64 = 0.20;
pub const INITIAL_CONFIDENCE: f64 = 0.3;
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;
pub const DEFAULT_PHASE: &str = "default";
```

## Database Compatibility

The SQLite schema is identical between Python and Rust versions. Existing `.dial/*.db` files work with both versions without migration.

## Failure Pattern Detection

21 patterns implemented across 5 categories:

- **import:** ImportError, ModuleNotFoundError
- **syntax:** SyntaxError, IndentationError
- **runtime:** NameError, TypeError, ValueError, AttributeError, KeyError, IndexError, FileNotFoundError, PermissionError, ConnectionError, TimeoutError
- **test:** TestFailure (FAILED.*test_), AssertionError
- **build:** RustCompileError, CargoBuildError, NpmError, TypeScriptError

## Building

```bash
cd ~/projects/dial/dial
cargo build --release
```

## Testing

```bash
cd ~/projects/dial/dial
cargo test
```

## Tested AI CLI Commands

All three supported CLIs have been tested with `dial auto-run`:

| CLI | Command | Notes |
|-----|---------|-------|
| Claude Code | `claude -p "prompt"` | Works directly |
| Codex CLI | `codex exec --skip-git-repo-check` | Pipe stdin |
| Gemini CLI | `gemini -p -` | Pipe stdin |

## DIAL Signals

Subagents should output these signals for orchestrator parsing:

```
DIAL_COMPLETE: <summary of what was done>
DIAL_BLOCKED: <reason for blockage>
DIAL_LEARNING: <category>: <what was learned>
```

Signal parsing is regex-based, case-insensitive, and ignores template placeholders.

## Unit Tests

15 tests covering:
- Failure pattern detection (4 tests)
- Signal parsing with various formats (11 tests)
  - Standard signals
  - Case variations
  - Markdown formatting
  - Template placeholder filtering

## Key Learnings

1. Use `COALESCE` in SQL SUM queries to handle NULL when aggregating empty tables
2. rusqlite's `query_map` borrows the statement - extract results to a variable before if-else block ends
3. Template placeholders in prompts can false-match signal parsing - filter lines containing `<placeholder>`
4. Context rot is a real problem - behavioral "signs" help remind agents of critical rules

## Future Roadmap

Features not yet implemented (planned for future versions):

### v2.3 - Task Dependencies
- Add `--depends-on` flag to `dial task add`
- Block tasks until dependencies complete
- Topological sort for task ordering

### v2.4 - Parallel Execution
- Run independent tasks in parallel
- Configure max concurrent subagents
- Aggregate results from parallel runs

### v2.5 - Enhanced Reliability
- Dry-run mode (`dial auto-run --dry-run`)
- Rate limit detection and automatic backoff
- Cost tracking and budget limits
- Progress webhooks for monitoring

### v2.6 - Advanced Features
- Web UI dashboard for monitoring runs
- MCP server integration
- Custom signal definitions
- Plugin system for custom validators
