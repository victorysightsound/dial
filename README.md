<h1><img src="assets/dial-icon.svg" width="36" alt="DIAL" style="vertical-align: middle;">&ensp;DIAL</h1>

**Deterministic Iterative Agent Loop**

A Rust library and CLI for autonomous AI-assisted software development.
Persistent memory. Failure pattern detection. Structured task execution.
Build entire projects iteratively without losing context or repeating mistakes.

[Quick Start](#quick-start)&ensp;&middot;&ensp;[Recommended Workflow](#recommended-workflow)&ensp;&middot;&ensp;[PRD Wizard](#prd-wizard)&ensp;&middot;&ensp;[Library](#library-usage)&ensp;&middot;&ensp;[CLI Reference](docs/cli-reference.md)&ensp;&middot;&ensp;[AI Integration](docs/ai-integration.md)

---

## The Problem

When you use AI coding assistants (Claude Code, Codex, Gemini) to build software iteratively, they hit predictable failure modes:

- **Context window exhaustion** - conversation history grows until the AI loses track of what it already built
- **Reasoning loss between loops** - decisions made 10 iterations ago are forgotten
- **Duplicate implementation** - the AI rewrites code that already exists
- **Placeholder code** - incomplete implementations with TODO comments
- **Cascading failures** - one error triggers increasingly desperate "fixes" that break more things
- **Specification drift** - the implementation diverges from requirements as the AI stops referencing them

DIAL solves these by externalizing memory to SQLite, linking tasks to specifications, detecting failure patterns automatically, building a trust-scored solution database, and enforcing one-task-at-a-time discipline.

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

## Recommended Workflow

The most effective way to use DIAL is spec-driven: write a specification first, import it into a structured PRD database, then link tasks to spec sections. This gives the AI grounded requirements at every iteration, preventing drift and duplicate work.

### 1. Initialize

```bash
cd your-project
git init
dial init --phase mvp
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
```

### 2. Create Your Specification

You have three options for getting a spec into DIAL:

**Option A: Write markdown and import** — Create a `specs/` directory with markdown files:

```bash
mkdir -p specs
# Write your spec in specs/PRD.md
dial spec import --dir specs
```

**Option B: Use the PRD Wizard** — Let AI help you create a structured spec interactively:

```bash
dial spec wizard --template mvp
```

**Option C: Refine an existing document** — Feed an existing PRD/spec/architecture doc through the wizard:

```bash
dial spec wizard --template spec --from existing-prd.md
```

All options produce the same result: a `prd.db` database with hierarchical sections, FTS5 search, and terminology tracking.

### 3. Add Tasks Linked to Spec Sections

```bash
dial task add "Create User model with email uniqueness" -p 1 --spec 1
dial task add "Implement POST /users endpoint" -p 2 --spec 2
dial task add "Add JWT authentication middleware" -p 3 --spec 3
dial task add "Write integration tests" -p 4
```

The `--spec` flag links a task to a spec section ID. When DIAL gathers context for that task, it includes the linked section automatically. Even without explicit links, DIAL searches the PRD by task description and surfaces relevant sections.

### 4. Set Up Dependencies (Optional)

```bash
dial task depends 3 1    # Auth depends on User model
dial task depends 4 2    # Tests depend on endpoints
```

### 5. Run the Loop

**Manual mode** (you control the AI):

```bash
dial iterate          # Get next task + spec context
# ... implement the task with your AI tool ...
dial validate         # Build, test, commit on success
```

**Automated mode** (DIAL drives the AI):

```bash
dial auto-run --cli claude --max 10
```

This spawns a fresh AI subprocess per task, parses completion signals, runs validation, and loops until done.

### Why Specs Matter

Without a spec, DIAL assembles context from learnings, solutions, and failures — useful, but the AI has no requirements to validate against. With a spec:

- Each task carries **what to build** alongside **how previous attempts went**
- FTS search surfaces relevant requirements even for unlinked tasks
- Specification drift is prevented because the AI re-reads requirements every iteration
- The token budget is spent on high-value context instead of generic history

For quick prototypes or small utilities, you can skip the spec and just use tasks. But for any project with more than a handful of tasks, the spec-driven workflow is what makes DIAL effective at scale.

## PRD Wizard

The PRD Wizard is an AI-assisted interactive tool that helps you create a structured specification from scratch, or refine an existing document into a DIAL-ready PRD.

### How It Works

The wizard walks through 5 phases, using your configured AI provider to guide the conversation:

| Phase | Name | What It Does |
|-------|------|-------------|
| 1 | **Vision** | Identifies the problem, target users, and core value proposition |
| 2 | **Functionality** | Defines features, user stories, and requirements |
| 3 | **Technical** | Covers architecture, data model, integrations, and constraints |
| 4 | **Gap Analysis** | Reviews everything gathered so far for gaps, contradictions, and missing details |
| 5 | **Generate** | Produces structured PRD sections, extracts terminology, and creates linked DIAL tasks |

### Templates

Four templates are available, each with a different section structure:

| Template | Use For |
|----------|---------|
| `spec` | General product requirements (Problem, Requirements, Features, Data Model, Constraints, Acceptance Criteria) |
| `architecture` | System architecture (Overview, Components, Data Model, Integrations, Deployment, Security) |
| `api` | API design (Overview, Authentication, Endpoints, Data Types, Error Handling) |
| `mvp` | Minimum viable product (Problem, MVP Features, Technical Stack, Data Model) |

### Usage

```bash
# Start a new wizard session with a template
dial spec wizard --template mvp

# Refine an existing document through the wizard
dial spec wizard --template spec --from docs/existing-prd.md

# Resume a paused wizard session
dial spec wizard --resume
```

The wizard can be paused at any phase and resumed later — state is persisted in `prd.db`.

### What Gets Created

After the wizard completes:

- **PRD sections** in `prd.db` with hierarchical dotted IDs (1, 1.1, 1.2.1), full-text search, and word counts
- **Terminology entries** extracted from your spec (canonical names, variants, definitions, categories)
- **Linked DIAL tasks** ready to iterate on, each tied to its relevant PRD section

### Import Without the Wizard

If you already have markdown specs and don't need AI refinement:

```bash
# Import all markdown files from a directory
dial spec import --dir specs

# Migrate existing spec_sections from a legacy DIAL database
dial spec migrate
```

### Querying the PRD

```bash
dial spec list                    # List all PRD sections with hierarchy
dial spec prd 1.2                 # Show a specific section by dotted ID
dial spec prd-search "auth"       # Full-text search across sections
dial spec check                   # PRD health check (section count, word count, terms)
```

### Terminology Management

```bash
dial spec term add "API" "Application Programming Interface" -c technical --variants "api,Rest API"
dial spec term list               # All terms
dial spec term list -c domain     # Filter by category
dial spec term search "auth"      # Search terms
```

### Project Structure with PRD

```
your-project/
├── .dial/
│   ├── mvp.db              # Engine state (tasks, iterations, failures, solutions)
│   ├── prd.db              # PRD database (sections, terminology, sources, wizard state)
│   ├── current_phase
│   └── current_context.md
├── specs/                   # Original source documents (stay intact)
│   └── PRD.md
└── ... your project files
```

`prd.db` is a separate SQLite database alongside the main phase database. The engine reads from it during context assembly — when a task is linked to a PRD section, that section's content is automatically included in the iteration context.

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
