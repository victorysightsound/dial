# Configuration Reference

## Project Structure

After `dial init`, your project has this structure:

```
your-project/
├── .dial/                       # DIAL data directory
│   ├── <phase>.db               # SQLite database for the active phase
│   ├── current_phase            # Text file with the active phase name
│   ├── current_context.md       # Auto-generated context (latest)
│   ├── subagent_prompt.md       # Auto-generated prompt (latest)
│   └── stop                     # Created by `dial stop` (temporary)
├── specs/                       # Specification files (you create these)
│   └── *.md                     # Markdown specs, indexed by `dial index`
└── AGENTS.md                    # AI agent instructions (optional)
```

## Configuration Keys

Configuration is stored in the SQLite database as key-value pairs. Manage with `dial config set` and `dial config show`.

| Key | Default | Description |
|-----|---------|-------------|
| `phase` | set during init | Current phase name |
| `project_name` | directory name | Project name (derived automatically) |
| `build_cmd` | (empty) | Shell command to build the project |
| `test_cmd` | (empty) | Shell command to run tests |
| `build_timeout` | `600` | Build command timeout in seconds |
| `test_timeout` | `600` | Test command timeout in seconds |
| `ai_cli` | `claude` | AI CLI for auto-run: `claude`, `codex`, or `gemini` |
| `subagent_timeout` | `1800` | Per-task timeout for auto-run in seconds |

### Setting Configuration

```bash
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
dial config set build_timeout 300
dial config set test_timeout 300
dial config set ai_cli claude
dial config set subagent_timeout 900
```

### Viewing Configuration

```bash
dial config show
```

## Database Schema

DIAL uses SQLite with WAL (Write-Ahead Logging) mode and FTS5 full-text search. The database is created at `.dial/<phase>.db`.

### Tables

#### `config`

Key-value configuration store.

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Configuration key |
| `value` | TEXT | Configuration value |
| `updated_at` | TEXT | ISO 8601 timestamp |

#### `spec_sections`

Indexed specification sections from markdown files.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `file_path` | TEXT | Source markdown file |
| `heading_path` | TEXT | Full heading hierarchy (e.g., "1. Core > 1.1 Auth") |
| `level` | INTEGER | Header level (1 for #, 2 for ##, etc.) |
| `content` | TEXT | Section content |
| `indexed_at` | TEXT | ISO 8601 timestamp |

FTS5 virtual table: `spec_sections_fts` (indexed on `content`).

#### `tasks`

Task queue with status tracking.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `description` | TEXT | Task description |
| `status` | TEXT | `pending`, `in_progress`, `completed`, `blocked`, `cancelled` |
| `priority` | INTEGER | 1 (highest) to 10 (lowest), default 5 |
| `blocked_by` | TEXT | Blockage reason (nullable) |
| `spec_section_id` | INTEGER | FK to spec_sections (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |
| `started_at` | TEXT | When work began (nullable) |
| `completed_at` | TEXT | When finished (nullable) |

FTS5 virtual table: `tasks_fts` (indexed on `description`).

#### `iterations`

Each attempt to complete a task.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `task_id` | INTEGER | FK to tasks |
| `status` | TEXT | `in_progress`, `completed`, `failed`, `reverted` |
| `attempt_number` | INTEGER | Which attempt (1, 2, or 3) |
| `started_at` | TEXT | ISO 8601 timestamp |
| `ended_at` | TEXT | ISO 8601 timestamp (nullable) |
| `duration_seconds` | REAL | Time taken (nullable) |
| `commit_hash` | TEXT | Git commit hash on success (nullable) |
| `notes` | TEXT | Outcome notes (nullable) |

#### `actions`

Actions taken during an iteration (build, test, etc.).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `iteration_id` | INTEGER | FK to iterations |
| `action_type` | TEXT | Type of action (e.g., "build", "test") |
| `description` | TEXT | Action description |
| `file_path` | TEXT | Affected file (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

#### `outcomes`

Results of actions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `action_id` | INTEGER | FK to actions |
| `success` | INTEGER | 1 for success, 0 for failure |
| `output_summary` | TEXT | Stdout summary (nullable) |
| `error_message` | TEXT | Stderr/error text (nullable) |
| `duration_seconds` | REAL | Time taken (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

#### `failure_patterns`

Categorized error types detected across iterations.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `pattern_key` | TEXT (UNIQUE) | Pattern name (e.g., "RustCompileError") |
| `category` | TEXT | Category: `import`, `syntax`, `runtime`, `test`, `build`, `unknown` |
| `occurrence_count` | INTEGER | How many times detected |
| `first_seen_at` | TEXT | ISO 8601 timestamp |
| `last_seen_at` | TEXT | ISO 8601 timestamp |

#### `failures`

Specific failure instances linked to patterns.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `iteration_id` | INTEGER | FK to iterations |
| `pattern_id` | INTEGER | FK to failure_patterns |
| `error_text` | TEXT | Full error output |
| `file_path` | TEXT | File that caused the error (nullable) |
| `line_number` | INTEGER | Line number (nullable) |
| `resolved` | INTEGER | 0 or 1 |
| `resolved_by_solution_id` | INTEGER | FK to solutions (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

FTS5 virtual table: `failures_fts` (indexed on `error_text`).

#### `solutions`

Trusted fixes linked to failure patterns.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `pattern_id` | INTEGER | FK to failure_patterns |
| `description` | TEXT | How to fix the problem |
| `code_example` | TEXT | Code snippet (nullable) |
| `confidence` | REAL | Trust score 0.0 - 1.0 |
| `success_count` | INTEGER | Times applied successfully |
| `failure_count` | INTEGER | Times applied unsuccessfully |
| `created_at` | TEXT | ISO 8601 timestamp |
| `last_used_at` | TEXT | ISO 8601 timestamp (nullable) |

FTS5 virtual table: `solutions_fts` (indexed on `description`, `code_example`).

#### `solution_applications`

Tracks each time a solution is applied.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `solution_id` | INTEGER | FK to solutions |
| `failure_id` | INTEGER | FK to failures |
| `iteration_id` | INTEGER | FK to iterations |
| `success` | INTEGER | 0 or 1 |
| `created_at` | TEXT | ISO 8601 timestamp |

#### `learnings`

Project knowledge captured during development.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `category` | TEXT | One of: `build`, `test`, `setup`, `gotcha`, `pattern`, `tool`, `other` |
| `description` | TEXT | What was learned |
| `discovered_at` | TEXT | ISO 8601 timestamp |
| `times_referenced` | INTEGER | Auto-incremented when included in context |

FTS5 virtual table: `learnings_fts` (indexed on `category`, `description`).

### Database Pragmas

```sql
PRAGMA journal_mode = WAL;     -- Write-ahead logging for concurrent reads
PRAGMA busy_timeout = 5000;    -- 5 second retry on lock contention
```

### FTS5 Triggers

All FTS5 virtual tables are kept in sync with their source tables via INSERT, UPDATE, and DELETE triggers. You don't need to manage them manually.

## Built-in Failure Patterns

DIAL recognizes 21 error patterns across 5 categories:

| Category | Patterns |
|----------|----------|
| **import** | `ImportError`, `ModuleNotFoundError` |
| **syntax** | `SyntaxError`, `IndentationError` |
| **runtime** | `NameError`, `TypeError`, `ValueError`, `AttributeError`, `KeyError`, `IndexError`, `FileNotFoundError`, `PermissionError`, `ConnectionError`, `TimeoutError` |
| **test** | `TestFailure` (FAILED.*test_), `AssertionError` |
| **build** | `RustCompileError` (error[E\d+]), `CargoBuildError`, `NpmError`, `TypeScriptError` |

Unrecognized errors are recorded as `UnknownError` in the `unknown` category.

## Trust Scoring Constants

| Constant | Value | Description |
|----------|-------|-------------|
| Initial confidence | 0.3 | Score for new solutions |
| Trust threshold | 0.6 | Minimum to be "trusted" |
| Success increment | +0.15 | Added on successful application |
| Failure decrement | -0.20 | Subtracted on failed application |
| Maximum confidence | 1.0 | Upper bound |
| Max fix attempts | 3 | Retries before blocking a task |

## Environment Variables

| Variable | Effect |
|----------|--------|
| `NO_COLOR` | Disables ANSI color output |

## Git Integration

DIAL auto-commits when validation passes. Commit messages follow the format:

```
DIAL: <task description>
```

For auto-run mode, the same format is used. DIAL checks `git status --porcelain` before committing and only commits if there are changes.

DIAL does not push to remote repositories. That's always your decision.

## `.gitignore` Recommendations

Add to your project's `.gitignore`:

```
.dial/
```

The `.dial/` directory contains the local database and generated files. These are machine-specific and should not be committed.
