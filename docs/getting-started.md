# Getting Started with DIAL

This guide walks through installing DIAL, setting up your first project, and running the development loop.

## Prerequisites

- **Git** (DIAL auto-commits on successful validation)
- An AI coding tool (optional for manual mode, required for `dial new` and `dial auto-run`):
  - [Claude Code](https://claude.ai/download) (`claude` CLI) — supports auto-run
  - [Codex CLI](https://github.com/openai/codex) (`codex`) — supports auto-run
  - [GitHub Copilot CLI](https://docs.github.com/copilot/how-tos/use-copilot-agents/coding-agent/customizing-the-development-environment-for-copilot-coding-agent) (`copilot`) — supports auto-run
  - [Gemini CLI](https://github.com/google-gemini/gemini-cli) (`gemini`) — supports auto-run
  - [GitHub Copilot](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot) in VS Code — manual/orchestrated mode
  - [Cursor](https://cursor.sh), [Windsurf](https://codeium.com/windsurf), or any AI editor — manual/orchestrated mode
- For wizard and auto-run flows, make sure the CLI you plan to use is installed, available on your PATH, and already authenticated before you begin.

## Installation

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/victorysightsound/dial/main/install.sh | sh
```

Downloads the correct prebuilt binary for your platform and installs it to `~/.local/bin/`. To upgrade, run the same command again.

### Windows Install

Download `dial-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/victorysightsound/dial/releases/latest), extract `dial.exe`, and add the containing directory to your PATH.

You can verify the install from PowerShell or `cmd.exe` with:

```powershell
dial --version
```

### Via Cargo

If you have the Rust toolchain installed:

```bash
cargo install dial-cli
```

The crate is published as `dial-cli` but the binary is `dial`. To upgrade: `cargo install dial-cli --force`.

### From Source

```bash
git clone https://github.com/victorysightsound/dial.git
cd dial
cargo build --release
```

The binary is at `target/release/dial`. Add it to your PATH:

```bash
# Option 1: Copy to a standard location
sudo cp target/release/dial /usr/local/bin/

# Option 2: Symlink to your personal bin
mkdir -p ~/bin
ln -sf "$(pwd)/target/release/dial" ~/bin/dial
# Make sure ~/bin is in your PATH
```

On Windows, the compiled binary is `target\\release\\dial.exe`.

### Verify Installation

```bash
dial --version
```

## Your First Project

There are two ways to use DIAL: with just tasks (fastest start) or with the full project wizard (`dial new`) which guides you through spec creation, task generation, and build/test configuration. The wizard enforces spec specificity (Phase 4 rewrites vague requirements), right-sizes tasks (Phase 6 splits oversized tasks), and generates test tasks (Phase 7 pairs tests with features). You can always start with just tasks and add a spec later.

If you have multiple supported AI CLIs installed, pass `--wizard-backend` on your first wizard run unless DIAL can clearly detect the active session backend:

```bash
dial new --template mvp --wizard-backend copilot
```

Current verification covers fresh wizard runs on macOS and native Windows CLI environments, including native Windows Copilot, native Windows existing-repo end-to-end runs with Codex, and agent-file mode validation on both macOS and Windows.

The guided wizard is meant to reduce prompt burden, not increase it:
- DIAL explains each phase in plain English
- DIAL sends structured prompts to the backend for each phase
- the wizard does not edit source code or start `dial auto-run`
- you can stop and resume later with `dial new --resume`

Common ways to start:

```bash
# New project from scratch
mkdir my-project && cd my-project
git init
dial new --template mvp --wizard-backend codex

# Existing repo with a design document
cd existing-repo
dial new --template spec --from docs/existing-prd.md --wizard-backend codex
```

### 1. Initialize

Create or navigate to your project directory and initialize DIAL:

```bash
mkdir my-project && cd my-project
git init
dial init --phase mvp
```

**Important:** Set up your `.gitignore` before running the loop. DIAL uses `git add -A` when committing successful tasks, which stages everything not excluded by `.gitignore`. DIAL automatically detects and unstages common secret files (`.env`, `.pem`, `.key`, etc.) before committing, but you should still make sure temp files, build artifacts, and editor configs are covered:

```bash
echo -e ".dial/\nnode_modules/\ntarget/\n.env\n*.tmp" >> .gitignore
git add .gitignore && git commit -m "Add gitignore"
```

This creates:
- `.dial/` directory
- `.dial/mvp.db` SQLite database
- `.dial/current_phase` file set to "mvp"

Agent file handling defaults to `local`:
- `local`: create `AGENTS.md` and hide top-level agent files from `git status` using `.git/info/exclude`
- `shared`: create `AGENTS.md` and leave it visible so your team can commit it intentionally
- `off`: skip agent instruction files entirely

Examples:

```bash
dial init --agents shared
dial init --agents off
```

The `--phase` flag names this development phase. You can create multiple phases (e.g., `mvp`, `beta`, `v2`) with separate databases, and even import trusted solutions from previous phases:

```bash
dial init --phase beta --import-solutions mvp
```

### 2. Configure Build and Test Commands

Tell DIAL how to build and test your project:

```bash
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
```

These commands run during validation. Some examples for different stacks:

| Stack | build_cmd | test_cmd |
|-------|-----------|----------|
| Rust | `cargo build` | `cargo test` |
| Node.js | `npm run build` | `npm test` |
| Python | `python -m py_compile main.py` | `pytest` |
| Go | `go build ./...` | `go test ./...` |
| Make | `make build` | `make test` |

You can also configure timeouts (default is 600 seconds each):

```bash
dial config set build_timeout 300
dial config set test_timeout 300
```

### 3. Create Tasks

Add tasks describing what needs to be built:

```bash
dial task add "Set up project structure with Cargo.toml and dependencies" -p 1
dial task add "Implement core data types and SQLite schema" -p 2
dial task add "Add CLI argument parsing" -p 3
dial task add "Implement CRUD commands" -p 4
dial task add "Add error handling and input validation" -p 5
dial task add "Write integration tests" -p 6
```

The `-p` flag sets priority (1 = highest, 10 = lowest). Tasks execute in priority order.

View your task queue:

```bash
dial task list
```

That's all you need to start. Skip to [Step 5: Run the Loop](#5-run-the-loop) if you want to get going immediately.

### 4. Add a Specification (Optional)

For richer context, write a spec and let DIAL link tasks to relevant sections. Create a `specs/` directory and add markdown files:

```bash
mkdir -p specs
```

Write your specification in `specs/PRD.md`:

```markdown
# Task Manager CLI

## 1. Core Data Model

The application stores tasks with the following fields:
- id: auto-incrementing integer
- title: string, required
- status: enum (pending, in_progress, done)
- priority: integer 1-5
- created_at: timestamp

## 2. CLI Commands

### 2.1 Add Task
`task add "title" --priority 3`

### 2.2 List Tasks
`task list [--all]` shows active tasks by default.

### 2.3 Complete Task
`task done <id>` marks a task as done.

## 3. Storage
Use SQLite for persistence. Database file at `~/.tasks.db`.
```

Index the spec so DIAL can search it:

```bash
dial index
```

Now you can link tasks to spec sections for automatic context retrieval:

```bash
dial task add "Implement Task data model" -p 2 --spec 1
dial task add "Implement 'add' command" -p 3 --spec 2
dial task add "Implement 'list' command" -p 4 --spec 3
```

DIAL parses markdown headers into sections and creates FTS5 full-text search indexes. When you work on a task, DIAL automatically surfaces relevant spec sections — even without explicit `--spec` links, it searches by task description.

If you are working inside an existing repository, you can skip the manual spec import path and let the wizard refine your existing PRD or architecture document instead:

```bash
dial new --template spec --from docs/existing-prd.md --wizard-backend codex
```

### 5. Run the Loop

You have three ways to run DIAL:

#### Option A: Manual Mode

You drive the AI yourself and use DIAL for task tracking and context:

```bash
# 1. Start the next task
dial iterate
# This outputs the task description + relevant context

# 2. Implement the task (in your AI tool or editor)

# 3. Validate and commit
dial validate
# Runs build_cmd and test_cmd
# On success: auto-commits and moves to next task
# On failure: records failure pattern, resets task for retry
```

#### Option B: Semi-Automated with Context

Use `dial context` or `dial orchestrate` to generate prompts for your AI tool:

```bash
# Generate a self-contained prompt for a sub-agent
dial orchestrate

# The prompt is saved to .dial/subagent_prompt.md
# Feed it to your AI tool:
claude -p "$(cat .dial/subagent_prompt.md)"
```

#### Option C: Fully Automated

Let DIAL drive the entire loop:

```bash
dial auto-run --cli claude --max 10
```

This:
1. Picks the next pending task
2. Generates a context-rich prompt
3. Spawns a fresh AI subprocess
4. Parses DIAL signals from the output
5. Runs validation (build + test)
6. Commits on success, retries on failure (max 3 attempts)
7. Moves to the next task
8. Repeats until all tasks are done or the limit is reached

To stop gracefully: create a `.dial/stop` file or press Ctrl+C.

`dial auto-run` is always a separate explicit command. The wizard never starts autonomous execution on its own.

**Task sizing tip:** Each task runs in a single AI subprocess with a timeout (default 30 min). If a task is too large, the AI may time out or lose focus. Rule of thumb: a task should touch 1-3 files and do one focused thing. If you use `dial new`, Phase 6 automatically analyzes task sizing and splits oversized tasks for you.

### 6. Monitor Progress

```bash
# Current status
dial status

# Full statistics
dial stats

# Iteration history
dial history

# View failures
dial failures

# View trusted solutions
dial solutions -t
```

## What Happens When Things Fail

When a build or test fails during validation:

1. DIAL captures the error output
2. Matches it against 21 built-in failure patterns (import errors, syntax errors, build errors, etc.)
3. Records the failure with its pattern category
4. Checks for existing trusted solutions for that pattern
5. Resets the task to pending for retry
6. Includes the failure context and solutions in the next iteration

After 3 failed attempts, DIAL blocks the task and (if in a git repo) reverts to the last successful commit.

If a solution works, its confidence score increases (+0.15). If it fails, the score decreases (-0.20). Solutions with confidence >= 0.6 are "trusted" and automatically surface in future context.

## Recording Learnings

After completing a task, record what you learned:

```bash
dial learn "SQLite WAL mode is required for concurrent access" -c build
dial learn "Must run migrations before seeding test data" -c setup
dial learn "The config parser silently ignores unknown keys" -c gotcha
```

These persist in the database and appear in context for future tasks, sorted by how frequently they've been useful.

## Multi-Phase Projects

DIAL supports multiple development phases with separate databases:

```bash
# Start with MVP
dial init --phase mvp
# ... build MVP ...

# Move to beta, importing trusted solutions
dial init --phase beta --import-solutions mvp
# ... build beta features ...
```

Each phase gets its own database (`.dial/mvp.db`, `.dial/beta.db`), but you can carry forward solutions that proved reliable.

## Next Steps

- [CLI Reference](cli-reference.md) - Every command and flag
- [Methodology](methodology.md) - The theory behind DIAL
- [AI Integration](ai-integration.md) - Detailed setup for each AI tool, auto-run guidance, and troubleshooting
- [Configuration](configuration.md) - All config options and database schema
