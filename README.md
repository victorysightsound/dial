# DIAL - Deterministic Iterative Agent Loop

A CLI tool and methodology for autonomous AI-assisted software development. DIAL gives AI coding agents persistent memory, failure pattern detection, and structured task execution so they can build entire projects iteratively without losing context or repeating mistakes.

## The Problem

When you use AI coding assistants (Claude Code, Codex, Gemini) to build software iteratively, they hit predictable failure modes:

- **Context window exhaustion** - conversation history grows until the AI loses track of what it already built
- **Reasoning loss between loops** - decisions made 10 iterations ago are forgotten
- **Duplicate implementation** - the AI rewrites code that already exists
- **Placeholder code** - incomplete implementations with TODO comments
- **Cascading failures** - one error triggers increasingly desperate "fixes" that break more things
- **Test amnesia** - solutions that passed tests before are forgotten and reimplemented poorly

DIAL solves these by externalizing memory to SQLite, detecting failure patterns automatically, building a trust-scored solution database, and enforcing one-task-at-a-time discipline.

## How It Works

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

DIAL maintains a per-project SQLite database with FTS5 full-text search. Each project gets:
- **Task queue** with priorities and status tracking
- **Indexed specifications** parsed from your PRD/spec markdown files
- **Failure pattern catalog** that auto-categorizes errors (21 built-in patterns)
- **Trust-scored solutions** that earn confidence through repeated success
- **Project learnings** that persist across sessions

## Quick Start

### Install

```bash
# Clone and build
git clone https://github.com/victorysightsound/dial.git
cd dial/dial
cargo build --release

# Add to PATH (pick one)
cp target/release/dial /usr/local/bin/
# or
ln -sf "$(pwd)/target/release/dial" ~/bin/dial
```

**Requirements:** Rust 1.70+ and Cargo. No other runtime dependencies - the binary is fully self-contained (~4MB).

### Initialize a Project

```bash
cd your-project
dial init --phase mvp
```

This creates a `.dial/` directory with a SQLite database for this phase.

### Configure Build and Test Commands

```bash
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
```

These commands run during `dial validate` to verify each task.

### Add Your Specification

Put your PRD or spec files in a `specs/` directory as markdown, then index them:

```bash
mkdir -p specs
# Create or copy your PRD.md into specs/
dial index
```

DIAL parses markdown headers into searchable sections and surfaces relevant ones when working on related tasks.

### Create Tasks

```bash
dial task add "Set up project structure with Cargo.toml" -p 1
dial task add "Implement core data types" -p 2
dial task add "Add CLI argument parsing" -p 3
dial task add "Write unit tests" -p 4
```

Priority is 1 (highest) to 10 (lowest). Tasks execute in priority order.

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

This spawns a fresh AI subprocess per task, parses completion signals, runs validation, and loops until done or the limit is reached. Supports Claude Code, Codex CLI, and Gemini CLI.

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](docs/getting-started.md) | Detailed setup and first project walkthrough |
| [CLI Reference](docs/cli-reference.md) | Complete command reference with all flags and options |
| [Methodology](docs/methodology.md) | The DIAL methodology: failure modes, countermeasures, the loop |
| [AI Integration](docs/ai-integration.md) | Using DIAL with Claude Code, Codex, Gemini, and other AI tools |
| [Configuration](docs/configuration.md) | All configuration keys, database schema, and project structure |
| [Architecture](docs/architecture.md) | Source code architecture for contributors |

## Features

### Task Management
```bash
dial task add "description" -p 1    # Add with priority
dial task list                      # Show active tasks
dial task next                      # Preview next task
dial task done 5                    # Mark complete
dial task block 3 "waiting on API"  # Block with reason
dial task search "auth"             # Full-text search
```

### Failure Pattern Detection

DIAL automatically categorizes build/test errors into 21 patterns across 5 categories (import, syntax, runtime, test, build). When a failure recurs, DIAL surfaces previously successful solutions.

### Trust-Scored Solutions

Solutions start at 0.3 confidence. Each successful application adds +0.15; each failure subtracts -0.20. Solutions reaching 0.6 confidence become "trusted" and are automatically included in context for future tasks.

### Specification Search

```bash
dial index                     # Index specs/ directory
dial spec search "auth"        # Full-text search
dial spec show 5               # Show full section
```

### Project Learnings

```bash
dial learn "Always run migrations before tests" -c setup
dial learnings list -c gotcha
dial learnings search "database"
```

Categories: `build`, `test`, `setup`, `gotcha`, `pattern`, `tool`, `other`

### Statistics Dashboard

```bash
dial stats
```

Shows iteration counts, success rates, task progress, time spent, top failure patterns, solution hit rates, and learning counts.

## Example Workflow

```bash
# 1. Start a new project
mkdir my-app && cd my-app
git init
dial init --phase mvp

# 2. Configure
dial config set build_cmd "npm run build"
dial config set test_cmd "npm test"

# 3. Write a spec
mkdir specs
cat > specs/PRD.md << 'EOF'
# My App PRD
## 1. Authentication
Users can sign up and log in with email/password.
## 2. Dashboard
Display user stats after login.
EOF
dial index

# 4. Create tasks from the spec
dial task add "Set up Next.js project with TypeScript" -p 1
dial task add "Implement user auth with email/password" -p 2 --spec 1
dial task add "Build dashboard page with stats" -p 3 --spec 2
dial task add "Add E2E tests for auth flow" -p 4

# 5. Run with AI
dial auto-run --cli claude --max 10

# 6. Check progress
dial status
dial stats
```

## Project Structure

```
your-project/
├── .dial/
│   ├── mvp.db              # SQLite database for this phase
│   ├── current_phase        # Active phase name
│   ├── current_context.md   # Latest context (auto-generated)
│   └── subagent_prompt.md   # Latest sub-agent prompt (auto-generated)
├── specs/
│   └── PRD.md              # Your specification files
└── ... your project files
```

## Performance

| Metric | Value |
|--------|-------|
| Startup time | ~14ms |
| Binary size | ~4MB |
| Dependencies | None (static binary) |
| Database | SQLite with WAL mode |

## License

MIT - see [LICENSE](LICENSE).
