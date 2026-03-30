# Configuration Reference

## Project Structure

After `dial init`, your project has this structure:

```
your-project/
├── .dial/                       # DIAL data directory
│   ├── <phase>.db               # SQLite database for the active phase
│   ├── prd.db                   # PRD database (sections, terminology, wizard state)
│   ├── current_phase            # Text file with the active phase name
│   ├── current_context.md       # Auto-generated context (latest)
│   ├── subagent_prompt.md       # Auto-generated prompt (latest)
│   ├── progress.md              # Human-readable iteration journal
│   ├── patterns.md              # Codebase pattern digest + trusted solutions
│   ├── task-ledger.md           # Human-readable task queue summary
│   ├── signal.json              # Subagent signal file (transient, auto-deleted)
│   └── stop                     # Created by `dial stop` (temporary)
├── specs/                       # Specification files (you create these)
│   └── *.md                     # Markdown specs, indexed by `dial spec import`
└── AGENTS.md                    # AI agent instructions (optional; created when agents mode is local/shared)
```

Agent-file modes:
- `local` (default): create `AGENTS.md` and hide top-level agent files from normal `git status` via `.git/info/exclude`
- `shared`: create `AGENTS.md` and leave it visible so the repository can commit it intentionally
- `off`: skip agent instruction files entirely

### Generated Artifacts

These files are refreshed automatically as you work:

| File | Purpose |
|------|---------|
| `.dial/current_context.md` | Latest assembled task context |
| `.dial/subagent_prompt.md` | Latest prompt for a fresh subagent |
| `.dial/progress.md` | Iteration journal with outcomes, changed files, and learnings |
| `.dial/patterns.md` | Compact digest of stable codebase patterns and trusted solutions |
| `.dial/task-ledger.md` | Human-readable task queue summary |

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
| `ai_cli` | `claude` | AI CLI for auto-run: `claude`, `codex`, `copilot`, or `gemini` |
| `wizard_backend` | auto-resolved | Wizard backend for `dial new` / `dial spec wizard`: `codex`, `claude`, `copilot`, `gemini`, or `openai-compatible` |
| `wizard_model` | (empty) | Optional model override for the selected wizard backend |
| `wizard_api_base_url` | (empty) | Base URL for the `openai-compatible` wizard backend |
| `subagent_timeout` | `1800` | Per-task timeout for auto-run in seconds |
| `approval_mode` | `auto` | Approval gate: `auto`, `review`, or `manual` |
| `iteration_mode` | `autonomous` | Auto-run behavior: `autonomous`, `review_every:N`, or `review_each` |
| `enable_checkpoints` | `true` | Create git stash checkpoints before each iteration |
| `max_total_failures` | `10` | Auto-block tasks exceeding this many cumulative failures |

### Setting Configuration

```bash
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
dial config set build_timeout 300
dial config set test_timeout 300
dial config set ai_cli claude
dial config set wizard_backend copilot
dial config set wizard_model gpt-5.4-mini
dial config set subagent_timeout 900
```

Wizard backend resolution order is:
1. `--wizard-backend`
2. `wizard_backend`
3. `ai_cli`
4. active session hint (for example Codex)
5. exactly one installed supported CLI

If multiple supported CLIs are installed and no explicit/configured backend is available, DIAL stops and asks you to choose instead of guessing.

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
| `prd_section_id` | TEXT | Dotted PRD section ID (nullable) |
| `total_attempts` | INTEGER | Cumulative attempts across all iterations (default 0) |
| `total_failures` | INTEGER | Cumulative failures across all iterations (default 0) |
| `last_failure_at` | TEXT | Timestamp of most recent failure (nullable) |
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
| `status` | TEXT | `in_progress`, `completed`, `failed`, `reverted`, `awaiting_approval`, `rejected` |
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
| `description` | TEXT | Human-readable pattern description |
| `category` | TEXT | Category: `import`, `syntax`, `runtime`, `test`, `build`, `unknown` |
| `occurrence_count` | INTEGER | How many times detected |
| `first_seen_at` | TEXT | ISO 8601 timestamp |
| `last_seen_at` | TEXT | ISO 8601 timestamp |
| `regex_pattern` | TEXT | Custom regex for DB-driven matching (nullable) |
| `status` | TEXT | `suggested`, `confirmed`, or `trusted` |

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
| `source` | TEXT | How the solution was discovered: `auto-learned` or `manual` |
| `last_validated_at` | TEXT | When the solution was last confirmed to work (nullable) |
| `version` | TEXT | Codebase version when solution was recorded (nullable) |

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
| `pattern_id` | INTEGER | FK to failure_patterns — auto-linked when learned during failure (nullable) |
| `iteration_id` | INTEGER | FK to iterations — which iteration this was learned in (nullable) |

FTS5 virtual table: `learnings_fts` (indexed on `category`, `description`).

#### `task_dependencies`

Task dependency graph for topological ordering.

| Column | Type | Description |
|--------|------|-------------|
| `task_id` | INTEGER (PK) | FK to tasks — the dependent task |
| `depends_on_id` | INTEGER (PK) | FK to tasks — the prerequisite task |
| `created_at` | TEXT | ISO 8601 timestamp |

Composite primary key on (`task_id`, `depends_on_id`). Both columns have ON DELETE CASCADE foreign keys.

#### `validation_steps`

Configurable validation pipeline steps.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `name` | TEXT | Step name (e.g., "lint", "build", "test") |
| `command` | TEXT | Shell command to execute |
| `sort_order` | INTEGER | Execution order (0 = first) |
| `required` | INTEGER | 1 = abort on failure, 0 = log and continue |
| `timeout_secs` | INTEGER | Per-step timeout (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

#### `provider_usage`

Token and cost tracking per iteration.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `iteration_id` | INTEGER | FK to iterations |
| `provider` | TEXT | Provider name (e.g., "anthropic", "cli-passthrough") |
| `model` | TEXT | Model identifier (nullable) |
| `tokens_in` | INTEGER | Input tokens (default 0) |
| `tokens_out` | INTEGER | Output tokens (default 0) |
| `cost_usd` | REAL | Estimated cost in USD (default 0.0) |
| `duration_secs` | REAL | API call duration (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

#### `solution_history`

Confidence change log for solutions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment |
| `solution_id` | INTEGER | FK to solutions |
| `event_type` | TEXT | Event: `applied_success`, `applied_failure`, `decay`, `manual_refresh` |
| `old_confidence` | REAL | Confidence before change |
| `new_confidence` | REAL | Confidence after change |
| `notes` | TEXT | Context for the change (nullable) |
| `created_at` | TEXT | ISO 8601 timestamp |

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

DIAL auto-commits when validation passes. Commit messages are normalized into short developer-style subjects based on the task description:

```
Add API error handling
```

DIAL checks `git status --porcelain` before committing and only commits if there are changes.

DIAL does not push to remote repositories. That's always your decision.

### Secret Detection

Before committing, DIAL scans staged files against 13 dangerous patterns (`.env`, `.pem`, `.key`, `id_rsa`, `credentials.json`, `service-account.json`, `.p12`, `.pfx`, `.secret`, `.secrets`, `.env.local`, `.env.production`, `id_ed25519`). Any matching files are automatically unstaged with a warning printed to stderr. This is a safety net, not a replacement for a proper `.gitignore`.

## `.gitignore` Recommendations

**Important:** DIAL uses `git add -A` when auto-committing successful tasks. This stages all unignored files in the working tree. While DIAL's secret detection will catch common dangerous files, you should still set up a thorough `.gitignore` before running the loop to prevent temp files, build artifacts, and editor configs from being committed.

At minimum, add:

```
.dial/
.env
*.tmp
```

For your specific stack, also exclude build artifacts (`target/`, `node_modules/`, `dist/`, `__pycache__/`, etc.), editor configs (`.vscode/`, `.idea/`), and any generated files that shouldn't be committed.

The `.dial/` directory contains the local database and generated context files. These are machine-specific and should not be committed.
