# CLI Reference

Complete reference for all DIAL commands, subcommands, and flags.

## Global

```
dial [COMMAND]
dial --version
dial --help
```

## Initialization

### `dial init`

Initialize DIAL in the current directory. Creates `.dial/` directory and database.

```bash
dial init [--phase NAME] [--import-solutions PHASE] [--no-agents]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--phase` | `default` | Name for this development phase |
| `--import-solutions` | (none) | Copy trusted solutions from another phase's database |
| `--no-agents` | false | Skip adding DIAL instructions to AGENTS.md |

**Examples:**

```bash
dial init --phase mvp
dial init --phase beta --import-solutions mvp
dial init --no-agents
```

**What it creates:**
- `.dial/` directory
- `.dial/<phase>.db` SQLite database with full schema
- `.dial/current_phase` file containing the phase name
- Appends DIAL instructions to `AGENTS.md` (unless `--no-agents`)

## Specification

### `dial index`

Index markdown specification files into the database for full-text search.

```bash
dial index [--dir PATH]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--dir` | `specs` | Directory containing markdown spec files |

Recursively finds all `.md` files in the directory, parses markdown headers (# ## ### etc.) into sections, and creates FTS5 search indexes. Re-running `dial index` replaces the existing index.

### `dial spec`

Query indexed specifications.

```bash
dial spec search QUERY     # Full-text search
dial spec show ID          # Display a specific section
dial spec list             # List all sections
```

**Examples:**

```bash
dial spec search "authentication"
dial spec show 3
dial spec list
```

## Task Management

### `dial task add`

Add a task to the queue.

```bash
dial task add DESCRIPTION [-p PRIORITY] [--spec SECTION_ID]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-p`, `--priority` | `5` | Priority from 1 (highest) to 10 (lowest) |
| `--spec` | (none) | Link to a spec section ID for automatic context |

**Examples:**

```bash
dial task add "Implement user login"
dial task add "Add rate limiting" -p 1
dial task add "Build dashboard" -p 3 --spec 5
```

### `dial task list`

List tasks.

```bash
dial task list [--all]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-a`, `--all` | false | Show all tasks including completed and cancelled |

Without `--all`, shows only pending, in-progress, and blocked tasks. Tasks are sorted by priority (ascending), then by ID.

### `dial task next`

Show the next task that will be picked up by `dial iterate`. Does not modify any state.

```bash
dial task next
```

### `dial task done`

Mark a task as completed.

```bash
dial task done ID
```

### `dial task block`

Block a task with a reason.

```bash
dial task block ID REASON
```

**Example:**

```bash
dial task block 5 "Waiting on API credentials"
```

### `dial task cancel`

Cancel a task (removes it from the active queue without completing).

```bash
dial task cancel ID
```

### `dial task search`

Full-text search across task descriptions.

```bash
dial task search QUERY
```

**Example:**

```bash
dial task search "database migration"
```

## The Loop

### `dial iterate`

Start the next pending task. Gathers context from the database (relevant specs, trusted solutions, unresolved failures, learnings) and writes it to `.dial/current_context.md`.

```bash
dial iterate
```

This command:
1. Picks the highest-priority pending task
2. Creates an iteration record
3. Marks the task as in-progress
4. Gathers context (specs, solutions, failures, learnings)
5. Includes behavioral guardrails ("signs")
6. Writes context to `.dial/current_context.md`
7. Pauses for you to implement the task

### `dial validate`

Validate the current in-progress task by running build and test commands.

```bash
dial validate
```

**On success:**
- Records passing outcomes
- Auto-commits all changes (if in a git repo with changes)
- Marks the iteration and task as completed
- Prompts for learning capture

**On failure:**
- Captures error output
- Detects failure pattern (e.g., RustCompileError, TestFailure)
- Records the failure with pattern
- Checks for trusted solutions and displays them
- Resets the task to pending for retry
- After 3 failures: blocks the task and reverts to last good commit

### `dial run`

Run the iterate/validate loop continuously.

```bash
dial run [--max N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--max` | (none) | Maximum number of iterations before stopping |

Stops when: task queue is empty, max iterations reached, or `.dial/stop` file is detected.

### `dial stop`

Create a stop flag to halt `dial run` or `dial auto-run` after the current iteration.

```bash
dial stop
```

Creates `.dial/stop` file which is checked between iterations.

### `dial context`

Regenerate fresh context for the current or next task without creating a new iteration.

```bash
dial context
```

Useful when you want to see what context DIAL would provide, or to refresh context mid-task. Writes to `.dial/current_context.md`.

### `dial orchestrate`

Generate a self-contained prompt for spawning a fresh AI sub-agent.

```bash
dial orchestrate
```

Outputs the prompt to stdout and saves it to `.dial/subagent_prompt.md`. The prompt includes the task, behavioral guardrails, relevant specs, trusted solutions, and signal instructions.

**Usage with AI tools:**

```bash
# Claude Code
claude -p "$(cat .dial/subagent_prompt.md)"

# Codex CLI
cat .dial/subagent_prompt.md | codex exec

# Gemini CLI
cat .dial/subagent_prompt.md | gemini -p -
```

### `dial auto-run`

Fully automated orchestration: spawns a fresh AI subprocess per task, parses signals, validates, and loops.

```bash
dial auto-run [--max N] [--cli NAME]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--max` | (none) | Maximum number of tasks to process |
| `--cli` | `claude` | AI CLI to use: `claude`, `codex`, or `gemini` |

**How it works:**
1. Picks next pending task
2. Generates sub-agent prompt with full context
3. Spawns AI subprocess (streams output in real-time)
4. Parses DIAL signals from output:
   - `DIAL_COMPLETE: <message>` - Task is done
   - `DIAL_BLOCKED: <reason>` - Task is stuck
   - `DIAL_LEARNING: <category>: <description>` - Insight captured
5. On completion: runs validation (build + test)
6. On validation pass: commits and moves to next task
7. On validation fail: records failure, resets for retry
8. Repeats until done, max reached, or `.dial/stop` detected

**Timeout:** Default 1800 seconds (30 minutes) per task. Configure with:

```bash
dial config set subagent_timeout 900  # 15 minutes
```

## Status and History

### `dial status`

Show current project status: phase, in-progress task, task counts, and recent iterations.

```bash
dial status
```

### `dial stats`

Comprehensive statistics dashboard.

```bash
dial stats
```

Shows:
- Iteration counts and success rate
- Task counts by status
- Total runtime, average/max iteration time
- Top 5 failure patterns
- Solution count and hit rate
- Learning count by category

### `dial history`

Show iteration history with timing and commit hashes.

```bash
dial history [-n LIMIT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-n`, `--limit` | `20` | Number of entries to show |

## Failures and Solutions

### `dial failures`

Show recorded failures.

```bash
dial failures [-a]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-a`, `--all` | false | Show all failures (default: unresolved only) |

### `dial solutions`

Show recorded solutions with their confidence scores.

```bash
dial solutions [-t]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-t`, `--trusted` | false | Show only trusted solutions (confidence >= 0.6) |

## Learnings

### `dial learn`

Record a project learning.

```bash
dial learn DESCRIPTION [-c CATEGORY]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c`, `--category` | (none) | Category: `build`, `test`, `setup`, `gotcha`, `pattern`, `tool`, `other` |

**Examples:**

```bash
dial learn "Always set WAL mode before concurrent writes" -c build
dial learn "Config parser ignores unknown keys silently" -c gotcha
```

### `dial learnings`

Query recorded learnings.

```bash
dial learnings list [-c CATEGORY]    # Browse learnings
dial learnings search QUERY          # Full-text search
dial learnings delete ID             # Remove a learning
```

## Recovery

### `dial revert`

Revert to the last successfully committed iteration. Runs `git reset --hard` to that commit.

```bash
dial revert
```

Only works in git repositories with at least one successful DIAL commit.

### `dial reset`

Reset the current in-progress iteration without committing. Marks the iteration as reverted and returns the task to pending.

```bash
dial reset
```

## Configuration

### `dial config set`

Set a configuration value.

```bash
dial config set KEY VALUE
```

### `dial config show`

Display all configuration values.

```bash
dial config show
```

See [Configuration](configuration.md) for all available keys.
