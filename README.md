<div align="center">
  <img src="assets/dial-icon.svg" width="160" alt="DIAL">
  <h1>DIAL</h1>
  <p><strong>Deterministic Iterative Agent Loop</strong></p>
  <p>
    A Rust library and CLI for autonomous AI-assisted software development.<br>
    Persistent memory. Failure pattern detection. Structured task execution.<br>
    Build entire projects iteratively without losing context or repeating mistakes.
  </p>
  <p>
    <a href="#quick-start">Quick Start</a>&ensp;&middot;&ensp;<a href="#library-usage">Library</a>&ensp;&middot;&ensp;<a href="docs/cli-reference.md">CLI Reference</a>&ensp;&middot;&ensp;<a href="docs/ai-integration.md">AI Integration</a>
  </p>
</div>

---

## The Problem

When you use AI coding assistants (Claude Code, Codex, Gemini) to build software iteratively, they hit predictable failure modes:

- **Context window exhaustion** - conversation history grows until the AI loses track of what it already built
- **Reasoning loss between loops** - decisions made 10 iterations ago are forgotten
- **Duplicate implementation** - the AI rewrites code that already exists
- **Placeholder code** - incomplete implementations with TODO comments
- **Cascading failures** - one error triggers increasingly desperate "fixes" that break more things
- **Test amnesia** - solutions that passed tests before are forgotten and reimplemented poorly

DIAL solves these by externalizing memory to SQLite, detecting failure patterns automatically, building a trust-scored solution database, and enforcing one-task-at-a-time discipline.

## Architecture

DIAL v3.0.0 is structured as a Rust workspace with three crates:

```
dial/
├── dial-core/       # Library crate — Engine, events, providers, persistence
├── dial-cli/        # Binary crate — CLI interface
└── dial-providers/  # Provider implementations (Claude, Codex, etc.)
```

The core library (`dial-core`) is embeddable — you can build custom tools, dashboards, or CI integrations on top of it. The CLI is one consumer of the library.

### The Loop

```
                    dial iterate
                         |
                    Get next task
                         |
               Gather context from DB
          (specs, solutions, learnings)
                         |
              AI implements the task
                         |
                   dial validate
                    /         \
              Pass              Fail
               |                  |
          Git commit         Record failure
          Next task          Detect pattern
                             Find solutions
                             Retry (max 3)
```

## Quick Start

### Install

```bash
# Via Cargo
cargo install dial-cli

# From source
git clone https://github.com/victorysightsound/dial.git
cd dial
cargo build --release
cp target/release/dial /usr/local/bin/
```

**Requirements:** Rust 1.70+. No runtime dependencies. The binary is fully self-contained.

### Start a Project

```bash
cd your-project
git init
dial init --phase mvp
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
```

### Add Tasks

```bash
dial task add "Set up project structure" -p 1
dial task add "Implement core data types" -p 2
dial task add "Add CLI argument parsing" -p 3
dial task add "Write unit tests" -p 4
```

### Run the Loop

**Manual mode** (you control the AI):

```bash
dial iterate          # Get next task + context
# ... implement the task with your AI tool ...
dial validate         # Build, test, commit on success
```

**Automated mode** (DIAL drives the AI):

```bash
dial auto-run --cli claude --max 10
```

This spawns a fresh AI subprocess per task, parses completion signals, runs validation, and loops until done.

## Library Usage

Add `dial-core` to your `Cargo.toml`:

```toml
[dependencies]
dial-core = "3.0"
tokio = { version = "1", features = ["full"] }
```

### Basic Example

```rust
use dial_core::{Engine, Event, EventHandler};
use std::sync::Arc;

struct Logger;
impl EventHandler for Logger {
    fn handle(&self, event: &Event) {
        println!("{:?}", event);
    }
}

#[tokio::main]
async fn main() -> dial_core::Result<()> {
    let mut engine = Engine::init("mvp", None, false).await?;
    engine.on_event(Arc::new(Logger));

    // Add tasks with priorities and dependencies
    let t1 = engine.task_add("Build core module", 8, None).await?;
    let t2 = engine.task_add("Write tests", 5, None).await?;
    engine.task_depends(t2, t1).await?;

    // Configure validation
    engine.config_set("build_cmd", "cargo build").await?;
    engine.config_set("test_cmd", "cargo test").await?;

    // Or use the configurable pipeline
    engine.pipeline_add("lint", "cargo clippy", 0, false, Some(60)).await?;
    engine.pipeline_add("build", "cargo build", 1, true, Some(300)).await?;
    engine.pipeline_add("test", "cargo test", 2, true, Some(600)).await?;

    // Get structured metrics
    let report = engine.stats().await?;
    println!("Success rate: {:.1}%", report.success_rate * 100.0);

    Ok(())
}
```

See [`dial-core/examples/`](dial-core/examples/) for more: custom providers, event handlers, and validation pipelines.

## Features

### Task Management
```bash
dial task add "description" -p 1    # Add with priority
dial task list                      # Show active tasks
dial task next                      # Preview next task
dial task done 5                    # Mark complete
dial task block 3 "waiting on API"  # Block with reason
dial task depends 5 3               # Task 5 depends on task 3
```

### Configurable Validation Pipeline
```bash
dial pipeline add "lint" "cargo clippy" --sort 0 --optional --timeout 60
dial pipeline add "build" "cargo build" --sort 1 --required --timeout 300
dial pipeline add "test" "cargo test" --sort 2 --required --timeout 600
dial pipeline show
```

Steps run in order. Required steps abort on failure. Optional steps log and continue.

### Approval Gates
```bash
dial config set approval_mode review   # auto | review | manual
dial approve                           # Accept a paused iteration
dial reject "needs error handling"     # Reject with reason
```

### Failure Pattern Detection

DIAL categorizes build/test errors into 21 patterns across 5 categories (import, syntax, runtime, test, build). When a failure recurs, DIAL surfaces previously successful solutions.

```bash
dial patterns list                     # Show all patterns
dial patterns add "MyError" "desc" "cat" "(?i)myerror" suggested
dial patterns promote 42               # suggested -> confirmed -> trusted
dial patterns suggest                  # Cluster unknown errors
```

### Trust-Scored Solutions

Solutions start at 0.3 confidence. Success adds +0.15; failure subtracts -0.20. Solutions at 0.6+ are "trusted" and automatically included in context.

```bash
dial solutions list                    # Show all solutions
dial solutions refresh 5               # Reset decay timer
dial solutions history 5               # View confidence changes
dial solutions decay                   # Apply confidence decay
```

### Metrics & Trends
```bash
dial stats                             # Summary dashboard
dial stats --format json               # Machine-readable output
dial stats --format csv                # Export for spreadsheets
dial stats --trend 30                  # Daily trends over 30 days
```

### Crash Recovery
```bash
dial recover                           # Reset dangling iterations
```

### Project Learnings
```bash
dial learn "Always run migrations before tests" -c setup
dial learnings list -c gotcha
dial learnings search "database"
```

### Specification Search
```bash
dial index                     # Index specs/ directory
dial spec search "auth"        # Full-text search
dial spec show 5               # Show full section
```

## Project Structure

```
your-project/
├── .dial/
│   ├── mvp.db              # SQLite database for this phase
│   ├── current_phase        # Active phase name
│   ├── current_context.md   # Latest context (auto-generated)
│   └── subagent_prompt.md   # Latest sub-agent prompt (auto-generated)
├── specs/                   # Optional — specification files
│   └── PRD.md
└── ... your project files
```

## Performance

| Metric | Value |
|--------|-------|
| Startup time | ~14ms |
| Binary size | ~4MB |
| Dependencies | None (static binary) |
| Database | SQLite with WAL mode + FTS5 |

## License

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
