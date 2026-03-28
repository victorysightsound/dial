# CLI Reference

Complete reference for all DIAL commands, subcommands, and flags.

## Global

```
dial [COMMAND]
dial --version
dial --help
```

## Project Setup

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

### `dial new`

Full 9-phase guided project setup. One command from zero to autonomous iteration.

```bash
dial new [--template NAME] [--from PATH] [--resume] [--phase NAME] [--wizard-backend NAME] [--wizard-model MODEL]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--template` | `spec` | PRD template: `spec`, `architecture`, `api`, or `mvp` |
| `--from` | (none) | Existing document to refine through the wizard |
| `--resume` | false | Resume from where the wizard left off |
| `--phase` | `default` | Name for the `.dial/<phase>.db` project phase |
| `--wizard-backend` | auto-resolved | Wizard backend: `codex`, `claude`, `copilot`, `gemini`, or `openai-compatible` |
| `--wizard-model` | (none) | Optional model override for the selected wizard backend |

**Phases:**

| # | Name | What Happens |
|---|------|-------------|
| 1 | Vision | AI identifies the problem, target users, success criteria |
| 2 | Functionality | AI defines MVP features, deferred features, user workflows |
| 3 | Technical | AI covers architecture, data model, integrations, constraints |
| 4 | Gap Analysis | AI reviews everything for gaps, contradictions, missing details |
| 5 | Generate | AI produces structured PRD sections, terminology, and initial tasks |
| 6 | Task Review | AI reorders tasks, adds dependencies, removes redundancy |
| 7 | Build & Test | AI suggests build/test commands and validation pipeline |
| 8 | Iteration Mode | AI recommends autonomous, review_every:N, or review_each |
| 9 | Launch | Prints summary, ready for `dial auto-run` |

**Examples:**

```bash
dial new --template mvp
dial new --template mvp --wizard-backend copilot
dial new --template spec --from docs/existing-prd.md
dial new --resume
```

State persists in `prd.db` after every phase. Close the terminal at any point and resume later with `--resume`.
If multiple wizard backends are installed and DIAL cannot detect an active session backend, pass `--wizard-backend` explicitly.
When backend selection is ambiguous, `dial new` stops before creating project files instead of guessing.

## Specification

### `dial index`

Index markdown specification files into the database for full-text search. **Deprecated:** Use `dial spec import` instead.

```bash
dial index [--dir PATH]
```

### `dial spec`

Manage specifications and PRD sections.

```bash
dial spec import --dir PATH                                       # Import markdown into prd.db
dial spec wizard --template NAME [--resume] [--wizard-backend B]  # Run phases 1-5 only (PRD generation)
dial spec migrate                    # Migrate legacy spec_sections to prd.db

dial spec list                       # List all PRD sections (hierarchical)
dial spec prd SECTION_ID             # Show section by dotted ID (e.g., 1.2.1)
dial spec prd-search QUERY           # Full-text search PRD sections
dial spec check                      # PRD health summary

dial spec term add TERM DEF [-c CAT] # Add terminology
dial spec term list [-c CATEGORY]    # List terms
dial spec term search QUERY          # Search terms

dial spec search QUERY               # Legacy spec search (fallback)
dial spec show ID                    # Legacy section display
```

**Examples:**

```bash
dial spec import --dir specs
dial spec wizard --template mvp --wizard-backend codex
dial spec wizard --template mvp --from docs/existing-prd.md --resume --wizard-backend copilot
dial spec prd 1.2
dial spec prd-search "authentication"
dial spec term add "API" "Application Programming Interface" -c technical
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

### `dial task list`

List tasks.

```bash
dial task list [--all]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-a`, `--all` | false | Show all tasks including completed and cancelled |

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

### `dial task cancel`

Cancel a task.

```bash
dial task cancel ID
```

### `dial task search`

Full-text search across task descriptions.

```bash
dial task search QUERY
```

### `dial task depend`

Add a dependency relationship between tasks.

```bash
dial task depend TASK_ID DEPENDS_ON_ID
```

Task TASK_ID will not be picked up until DEPENDS_ON_ID is completed. Cycle detection prevents circular dependencies.

### `dial task undepend`

Remove a dependency relationship.

```bash
dial task undepend TASK_ID DEPENDS_ON_ID
```

### `dial task deps`

Show dependency information for a task.

```bash
dial task deps TASK_ID
```

### `dial task chronic`

Show tasks that fail repeatedly across iterations.

```bash
dial task chronic [--threshold N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--threshold` | `10` | Minimum total failures to be considered chronic |

## The Loop

### `dial iterate`

Start the next pending task. Gathers context from the database and writes it to `.dial/current_context.md`.

```bash
dial iterate [--dry-run] [--format FORMAT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--dry-run` | false | Preview what would happen without executing |
| `--format` | `text` | Output format for dry run: `text` or `json` |

**Normal mode:** Picks the highest-priority unblocked task, creates an iteration record, gathers context (specs, solutions, failures, similar tasks, learnings), and writes context to `.dial/current_context.md`.

**Dry run mode:** Shows the task that would be selected, context items included/excluded with token counts, prompt preview, and suggested solutions — without creating any database records.

### `dial validate`

Validate the current in-progress task by running the validation pipeline.

```bash
dial validate
```

**On success:**
- Drops checkpoint (restores clean git state marker)
- Auto-commits all changes
- Marks the iteration and task as completed
- Auto-unblocks dependent tasks
- If a solution was suggested, increments its confidence

**On failure:**
- Restores checkpoint (reverts working tree to pre-iteration state)
- Detects failure pattern
- Records failure and checks for trusted solutions
- Resets the task to pending for retry
- After 3 failures: blocks the task and reverts to last good commit

### `dial run`

Run the iterate/validate loop continuously.

```bash
dial run [--max N]
```

### `dial auto-run`

Fully automated orchestration: spawns a fresh AI subprocess per task, parses signals, validates, and loops.

```bash
dial auto-run [--max N] [--cli NAME] [--dry-run]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--max` | (none) | Maximum number of tasks to process |
| `--cli` | `claude` | AI CLI: `claude`, `codex`, `copilot`, or `gemini` |
| `--dry-run` | false | Show task execution order without running |

**Signal parsing:** After each subprocess exits, DIAL first checks for `.dial/signal.json` (structured JSON signals). If the file doesn't exist, it falls back to regex parsing of stdout for `DIAL_COMPLETE`, `DIAL_BLOCKED`, and `DIAL_LEARNING` signals.

**Iteration modes** (set via `dial config set iteration_mode`):
- `autonomous` — run all tasks without pausing (default)
- `review_every:N` — pause for review after every N completed tasks
- `review_each` — pause after every task for approval

When paused, resume with `dial approve` or stop with `dial reject`.

### `dial stop`

Create a stop flag to halt `dial run` or `dial auto-run` after the current iteration.

```bash
dial stop
```

### `dial context`

Regenerate fresh context for the current or next task without creating a new iteration.

```bash
dial context
```

### `dial orchestrate`

Generate a self-contained prompt for spawning a fresh AI sub-agent.

```bash
dial orchestrate
```

## Approval

### `dial approve`

Accept a paused iteration (when using review iteration modes).

```bash
dial approve
```

### `dial reject`

Reject a paused iteration with a reason. Resets the task to pending.

```bash
dial reject REASON
```

## Status and Monitoring

### `dial status`

Show current project status: phase, in-progress task, task counts, and recent iterations.

```bash
dial status
```

### `dial stats`

Comprehensive statistics dashboard.

```bash
dial stats [--format FORMAT] [--trend DAYS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format` | `text` | Output format: `text`, `json`, or `csv` |
| `--trend` | (none) | Show daily trends over N days |

### `dial health`

Project health score with weighted factors and trend detection.

```bash
dial health [--format FORMAT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format` | `text` | Output format: `text` or `json` |

Computes a 0-100 score from 6 factors: success rate (30%), success trend (15%), solution confidence (15%), blocked task ratio (15%), learning utilization (10%), pattern resolution rate (15%).

Color-coded output: green (70+), yellow (40-69), red (below 40).

### `dial history`

Show iteration history with timing and commit hashes.

```bash
dial history [-n LIMIT]
```

## Failures, Patterns, and Solutions

### `dial failures`

Show recorded failures.

```bash
dial failures [-a]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-a`, `--all` | false | Show all failures (default: unresolved only) |

### `dial patterns`

Manage failure patterns.

```bash
dial patterns list                                              # Show all patterns
dial patterns add KEY DESC CATEGORY REGEX STATUS                # Add custom pattern
dial patterns promote ID                                        # suggested → confirmed → trusted
dial patterns suggest                                           # Cluster unknown errors into suggestions
dial patterns metrics [--format FORMAT] [--sort FIELD]          # Per-pattern cost/time analytics
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format` | `text` | Output format: `text` or `json` |
| `--sort` | `occurrences` | Sort by: `cost`, `time`, or `occurrences` |

### `dial solutions`

Show recorded solutions with their confidence scores.

```bash
dial solutions [-t]
dial solutions refresh ID        # Reset decay timer
dial solutions history ID        # View confidence changes
dial solutions decay             # Apply confidence decay
```

## Learnings

### `dial learn`

Record a project learning.

```bash
dial learn DESCRIPTION [-c CATEGORY]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c`, `--category` | (none) | Category: `build`, `test`, `setup`, `gotcha`, `pattern`, `tool`, `other` |

### `dial learnings`

Query recorded learnings.

```bash
dial learnings list [-c CATEGORY] [--pattern KEY]    # Browse learnings
dial learnings search QUERY                          # Full-text search
dial learnings delete ID                             # Remove a learning
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c`, `--category` | (none) | Filter by category |
| `--pattern` | (none) | Filter by linked failure pattern key |

## Validation Pipeline

### `dial pipeline`

Manage the configurable validation pipeline.

```bash
dial pipeline show                                                    # List all steps
dial pipeline add NAME CMD --sort N --required|--optional [--timeout S]  # Add step
dial pipeline remove ID                                               # Remove step
```

Steps run in sort order. Required steps abort on failure. Optional steps log and continue.

## Recovery

### `dial revert`

Revert to the last successfully committed iteration.

```bash
dial revert
```

### `dial reset`

Reset the current in-progress iteration without committing.

```bash
dial reset
```

### `dial recover`

Reset dangling in-progress iterations (crash recovery).

```bash
dial recover
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
