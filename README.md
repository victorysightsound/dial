<h1><img src="assets/dial-icon.svg" width="36" alt="DIAL" style="vertical-align: middle;">&ensp;DIAL</h1>

**Deterministic Iterative Agent Loop**

A Rust library and CLI for autonomous AI-assisted software development.
Persistent memory. Failure pattern detection. Structured task execution.
Build entire projects iteratively without losing context or repeating mistakes.

[Quick Start](#quick-start)&ensp;&middot;&ensp;[Getting Started](docs/getting-started.md)&ensp;&middot;&ensp;[Project Wizard](#project-wizard)&ensp;&middot;&ensp;[Why Specs Matter](#why-specs-matter)&ensp;&middot;&ensp;[Templates](#templates)&ensp;&middot;&ensp;[Manual Workflow](#manual-workflow)&ensp;&middot;&ensp;[Library](#library-usage)&ensp;&middot;&ensp;[CLI Reference](docs/cli-reference.md)&ensp;&middot;&ensp;[AI Integration](docs/ai-integration.md)

---

## The Problem

When you use AI coding assistants (Claude Code, Codex, Gemini, GitHub Copilot, Cursor) to build software iteratively, they hit predictable failure modes:

- **Context window exhaustion** - conversation history grows until the AI loses track of what it already built
- **Reasoning loss between loops** - decisions made 10 iterations ago are forgotten
- **Duplicate implementation** - the AI rewrites code that already exists
- **Placeholder code** - incomplete implementations with TODO comments
- **Cascading failures** - one error triggers increasingly desperate "fixes" that break more things
- **Specification drift** - the implementation diverges from requirements as the AI stops referencing them

DIAL solves these by externalizing memory to SQLite, linking tasks to specifications, detecting failure patterns automatically, building a trust-scored solution database, and enforcing one-task-at-a-time discipline.

## Architecture

DIAL is structured as a Rust workspace with three crates:

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
          (linked spec, related specs,
           trusted solutions, learnings)
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

Choose one install method. DIAL is a global CLI: install it once for your user or machine, then use it in any project directory. The `.dial/` folder that appears later is per-project working data, not the program install location.

**Pre-built binaries** (no Rust required):

```bash
# macOS (Apple Silicon)
curl -L https://github.com/victorysightsound/dial/releases/latest/download/dial-aarch64-apple-darwin.tar.gz | tar xz
sudo mv dial /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/victorysightsound/dial/releases/latest/download/dial-x86_64-apple-darwin.tar.gz | tar xz
sudo mv dial /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/victorysightsound/dial/releases/latest/download/dial-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv dial /usr/local/bin/

# Linux (ARM64)
curl -L https://github.com/victorysightsound/dial/releases/latest/download/dial-aarch64-unknown-linux-gnu.tar.gz | tar xz
sudo mv dial /usr/local/bin/
```

**npm** (requires Node.js 18+):

```bash
npm install -g @victorysightsound/dial-cli
```

This installs the global `dial` command and downloads the matching prebuilt binary for your platform from the GitHub release for the current npm package version.

**Windows:** Download `dial-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/victorysightsound/dial/releases/latest), extract `dial.exe` into a permanent folder, and add that folder to your user PATH. See [Getting Started](docs/getting-started.md) for step-by-step Windows instructions and a plain-language explanation of PATH.

**Via Cargo** (requires Rust 1.70+):

```bash
cargo install dial-cli
```

**From source:**

```bash
git clone https://github.com/victorysightsound/dial.git
cd dial
cargo build --release
sudo cp target/release/dial /usr/local/bin/
```

On Windows, the compiled binary is `target\\release\\dial.exe`. The binary is fully self-contained with no runtime dependencies.

## Project Wizard

The fastest way to start a project. One command walks you from zero to autonomous iteration through 9 AI-guided phases:

```bash
cd your-project
git init
dial new --template mvp
```

The wizard can run through `codex`, `claude`, `copilot`, `gemini`, or an OpenAI-compatible API. If DIAL can detect the current agent session, it uses that backend automatically. Otherwise, pass one explicitly:

```bash
dial new --template mvp --wizard-backend copilot
```

The wizard is guided by default. You do not need to craft a special AI prompt for each phase; DIAL sends structured prompts internally and explains what it is doing as it moves through the flow.

Current release verification covers fresh macOS wizard runs with Codex and Copilot, native Windows wizard runs with Codex and Copilot, native Windows existing-repo `dial new` plus `dial auto-run --cli codex` end-to-end validation against the `mini-note-formatter` fixture scenario, and agent-file mode validation on both macOS and Windows.

For wizard and auto-run use, make sure the selected backend CLI is installed, on your PATH, and already authenticated before you start.

The wizard works in both of these common setups:
- brand-new repo: start in an empty repo and let DIAL generate the PRD, tasks, build/test commands, and iteration mode
- existing repo: run from the repo root and pass `--from` with an existing PRD, spec, or architecture doc so the wizard can refine what you already know about the codebase

| Phase | Name | What Happens |
|-------|------|-------------|
| 1 | **Vision** | AI identifies the problem, target users, success criteria |
| 2 | **Functionality** | AI defines MVP features, deferred features, user workflows |
| 3 | **Technical** | AI covers architecture, data model, integrations, constraints |
| 4 | **Gap Analysis** | AI reviews everything for gaps, contradictions, missing details |
| 5 | **Generate** | AI produces structured PRD sections, terminology, and initial tasks |
| 6 | **Task Review** | AI reorders tasks by implementation sequence, adds dependencies, removes redundancy |
| 7 | **Build & Test** | AI suggests build/test commands and validation pipeline based on tech stack |
| 8 | **Iteration Mode** | AI recommends how to run: autonomous, review every N tasks, or review each |
| 9 | **Launch** | Prints summary of everything configured, ready for `dial auto-run` |

Some later phases can be quiet for a bit, especially with CLI-backed providers on Windows. That is normal. The wizard will keep reporting where it is in the flow, and you can safely stop and resume later.

Agent file handling defaults to `local`: DIAL creates `AGENTS.md` for local AI tooling and hides it from `git status` with `.git/info/exclude`. Use `--agents shared` if you want to commit it intentionally, or `--agents off` to skip it.

After the wizard completes, start building:

```bash
dial auto-run --cli claude
```

`dial auto-run` is always a separate explicit step. The wizard never starts autonomous implementation on its own.

Supported auto-run CLIs are `claude`, `codex`, `copilot`, and `gemini`.

### Pause & Resume

Close the terminal at any phase. Pick up where you left off:

```bash
dial new --resume
```

State is persisted in `prd.db` after every phase — nothing is lost.

### Refine an Existing Document

Have an existing PRD, spec, or architecture doc? Feed it through the wizard:

```bash
dial new --template spec --from docs/existing-prd.md
```

The AI extracts information from your document alongside each phase's questions.

If you are planning inside an existing code repository, run the wizard from that repository's root. The best results come from pairing the repo with a document that describes the intended behavior instead of expecting the wizard to infer the whole architecture from source alone.

### Iteration Modes

Phase 8 configures how `auto-run` behaves:

| Mode | Config Value | Behavior |
|------|-------------|----------|
| **Autonomous** | `autonomous` | Run all tasks, commit on pass, no stops |
| **Review every N** | `review_every:N` | Pause for review after every N completed tasks |
| **Review each** | `review_each` | Pause after every task for approval |

When paused, resume with `dial approve` or stop with `dial reject`.

## Templates

Templates define the section structure the AI follows when generating your PRD. Pick the one that matches what you're building:

### `spec` — General Product Requirements
Best for: products with clear functional requirements and acceptance criteria.
```
Problem Statement → Requirements (Functional, Non-Functional) → Features → Data Model → Constraints → Acceptance Criteria
```

### `architecture` — System Architecture
Best for: system design documents, multi-service architectures, infrastructure planning.
```
Overview → Components (Interactions) → Data Model → Integrations → Deployment → Security
```

### `api` — API Specification
Best for: REST/GraphQL APIs, microservices, developer-facing products.
```
Overview → Authentication → Endpoints (Resources, Actions) → Data Types → Error Handling
```

### `mvp` — Minimum Viable Product
Best for: quick prototypes, hackathons, getting something working fast.
```
Problem → MVP Features → Technical Stack → Data Model
```

The template determines what sections the AI generates in phase 5 and what structure tasks are linked to. You can always add more sections manually after the wizard completes.

## Why Specs Matter

Without a spec, DIAL assembles context from learnings, solutions, and failures — useful, but the AI has no requirements to validate against. With a spec:

- Each task carries **what to build** alongside **how previous attempts went**
- FTS search surfaces relevant requirements even for unlinked tasks
- Specification drift is prevented because the AI re-reads requirements every iteration

This is the key difference between DIAL and running an AI assistant in a loop. The spec is externalized memory — it ensures the AI re-reads requirements at every iteration instead of relying on conversation history that degrades over time.

## Manual Workflow

If you prefer to control each step yourself instead of using `dial new`:

```bash
# 1. Initialize
dial init --phase mvp
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"

# 2. Import or create a spec
dial spec import --dir specs           # From markdown files
dial spec wizard --template mvp --wizard-backend codex

# 3. Add tasks
dial task add "Create user model" -p 1 --spec 1
dial task add "Add API endpoints" -p 2 --spec 2
dial task depends 2 1

# 4. Run
dial iterate          # One task at a time
dial validate         # Build, test, commit
# or
dial auto-run --cli copilot --max 10   # Fully autonomous
```

`dial spec wizard` runs phases 1-5 only (PRD generation). Use `dial new` for the full 9-phase flow.

### Querying the PRD

```bash
dial spec list                    # List all PRD sections with hierarchy
dial spec prd 1.2                 # Show a specific section by dotted ID
dial spec prd-search "auth"       # Full-text search across sections
dial spec check                   # PRD health check (section count, word count, terms)
dial spec term add "API" "Application Programming Interface" -c technical
dial spec term list               # All terms
dial spec term search "auth"      # Search terms
```

## Library Usage

Add `dial-core` to your `Cargo.toml`:

```toml
[dependencies]
dial-core = "4.2"
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

### Specification & PRD
```bash
dial spec import --dir specs       # Import markdown into prd.db
dial spec wizard --template mvp    # AI-guided spec creation
dial spec list                     # List PRD sections (hierarchical)
dial spec prd 1.2                  # Show section by dotted ID
dial spec prd-search "auth"        # Full-text search PRD
dial spec check                    # PRD health summary
dial spec term add "API" "def" -c technical  # Add terminology
dial spec term list                # List all terms
dial spec search "auth"            # Legacy spec search (fallback)
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

### Project Health Score
```bash
dial health                            # Color-coded score (green/yellow/red)
dial health --format json              # Machine-readable output
```

Six weighted factors: success rate, success trend, solution confidence, blocked task ratio, learning utilization, and pattern resolution rate. Trend detection compares current score vs 7 days ago.

### Dry Run / Preview Mode
```bash
dial iterate --dry-run                 # See what would happen without side effects
dial iterate --dry-run --format json   # Machine-readable preview
dial auto-run --dry-run                # Preview next auto-run iteration
```

Shows context items included/excluded with token sizes, prompt preview, suggested solutions, and dependency status — without creating iteration records or spawning subagents.

### Cross-Iteration Failure Tracking
```bash
dial task chronic                      # Tasks exceeding default failure threshold
dial task chronic --threshold 5        # Custom threshold
```

Tasks track `total_attempts` and `total_failures` across all iterations. Auto-run auto-blocks tasks exceeding `max_total_failures` (default: 10).

### Per-Pattern Metrics
```bash
dial patterns metrics                  # Table of per-pattern cost/time/occurrences
dial patterns metrics --format json    # Machine-readable output
dial patterns metrics --sort cost      # Sort by cost, time, or occurrences
```

### Checkpoint System

Automatic git stash-based checkpoints before task execution. On validation failure, the working tree is restored to pre-task state before retry. Controlled via `enable_checkpoints` config (default: true).

### Structured Subagent Signals

Subagents write `.dial/signal.json` with typed signals (`Complete`, `Blocked`, `Learning`) instead of printing `DIAL_` lines to stdout. Falls back to regex parsing for backward compatibility.

### Transaction Safety

All multi-step database mutations (`record_failure`, `task_done`, `iterate`, `prd_import`) are wrapped in `BEGIN IMMEDIATE` transactions with automatic rollback on error.

### Crash Recovery
```bash
dial recover                           # Reset dangling iterations
```

### Project Learnings
```bash
dial learn "Always run migrations before tests" -c setup
dial learnings list -c gotcha
dial learnings list --pattern 5        # Learnings linked to pattern #5
dial learnings search "database"
```

Learnings are auto-linked to failure patterns when recorded during an iteration with failures.

## Project Structure

```
your-project/
├── .dial/
│   ├── mvp.db              # Engine state (tasks, iterations, failures, solutions)
│   ├── prd.db              # PRD database (sections, terminology, wizard state)
│   ├── current_phase        # Active phase name
│   ├── current_context.md   # Latest context (auto-generated)
│   └── subagent_prompt.md   # Latest sub-agent prompt (auto-generated)
├── specs/                   # Original source documents (stay intact)
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

## Copyright and Branding

Copyright (c) 2026 Victory AV, LLC.

DIAL is authored and maintained by John Deaton / Victory AV, LLC. The
repository's open-source licenses grant rights to the code under their stated
terms, but they do not grant trademark or branding rights beyond what
applicable law otherwise allows.

If you redistribute DIAL or publish modified versions, preserve the copyright
and license notices and do not use the DIAL name, logos, or branding in a way
that suggests endorsement, affiliation, or official upstream status without
written permission.

See [NOTICE](NOTICE).
