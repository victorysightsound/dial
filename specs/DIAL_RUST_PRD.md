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

### v2.1.0 (Feb 2026) - Ralph-Style Improvements
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
| Lines of code | 2,271 | ~4,300 |
| Startup time | ~190ms | ~14ms |
| Binary size | N/A | 4.0MB |
| Dependencies | Python 3.x | None (static) |

## File Locations

- **Rust source:** `~/projects/dial/dial/`
- **Python legacy:** `~/projects/dial/dial_legacy.py`
- **Symlink:** `~/bin/dial` → `~/projects/dial/dial/target/release/dial`
- **Guide DB:** `~/projects/dial/dial_guide.db`

## CLI Commands (25 total)

All commands implemented with identical behavior to Python, plus Ralph-style additions:

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
│   │   ├── context.rs    # gather_context() algorithm
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
pub const VERSION: &str = "2.1.0";
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

## Key Learnings

1. Use `COALESCE` in SQL SUM queries to handle NULL when aggregating empty tables
2. rusqlite's `query_map` borrows the statement - extract results to a variable before if-else block ends
