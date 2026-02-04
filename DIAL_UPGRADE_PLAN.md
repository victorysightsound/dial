# DIAL Upgrade Plan

## Problem Statement

The current DIAL methodology relies on markdown files (`fix_plan.md`, `AGENT.md`, specs) for persistent memory. This causes:

1. **Context bloat** - Each iteration reads entire files, which grow over time
2. **Slower iterations** - More text to process = longer completion times
3. **Linear recall** - No way to query specific information; must read everything
4. **Re-learning** - Same solutions rediscovered repeatedly because learnings aren't structured

## Solution Overview

Replace markdown-based memory with SQLite + FTS5 database for:

- **Selective recall** - Query only what's needed, not entire files
- **Structured learning** - Solutions earn trust through repeated success
- **Constant context size** - Queries return fixed-size results regardless of iteration count
- **Standalone operation** - Self-contained per project, no external dependencies
- **Phase support** - Multiple DIAL runs per project with optional solution inheritance

## Implementation: Single Python File

**Location:** `~/bin/dial` (symlink to `~/.dial/dial.py`)

**Why Python:**
- sqlite3 built-in (no dependencies)
- Clean markdown parsing for spec indexer
- Proper math for trust calculations
- Robust error handling for autonomous operation

**Why single file:**
- Easy to distribute and maintain
- ~800-1000 lines estimated
- Clear section organization within file

---

## Configuration Decisions

| Setting | Value | Rationale |
|---------|-------|-----------|
| Max fix attempts | 3 | Enough to iterate, not enough to spiral |
| Build timeout | 600s (10 min) | Buffer for larger projects, configurable |
| Test timeout | 600s (10 min) | Same as build |
| Empty queue behavior | Stop and report | DIAL executes, human plans |
| Trust threshold | 0.6 | Requires 2+ successes to trust |

---

## Database Schema

**Location:** `.dial/{phase}.db` in each project (default phase: "default")

### Config Table
```sql
CREATE TABLE config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);
-- Stores: build_cmd, test_cmd, project_name, phase, build_timeout, test_timeout
```

### Spec Sections (indexed from markdown)
```sql
CREATE TABLE spec_sections (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,           -- e.g., "specs/auth.md"
    heading_path TEXT NOT NULL,        -- e.g., "Authentication > Login Flow"
    level INTEGER NOT NULL,            -- heading level (1-6)
    content TEXT NOT NULL,
    indexed_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE spec_sections_fts USING fts5(
    heading_path, content,
    content='spec_sections', content_rowid='id',
    tokenize='porter'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER spec_sections_ai AFTER INSERT ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(rowid, heading_path, content)
    VALUES (NEW.id, NEW.heading_path, NEW.content);
END;

CREATE TRIGGER spec_sections_ad AFTER DELETE ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(spec_sections_fts, rowid, heading_path, content)
    VALUES('delete', OLD.id, OLD.heading_path, OLD.content);
END;

CREATE TRIGGER spec_sections_au AFTER UPDATE ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(spec_sections_fts, rowid, heading_path, content)
    VALUES('delete', OLD.id, OLD.heading_path, OLD.content);
    INSERT INTO spec_sections_fts(rowid, heading_path, content)
    VALUES (NEW.id, NEW.heading_path, NEW.content);
END;
```

### Tasks (replaces fix_plan.md)
```sql
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    status TEXT DEFAULT 'pending'
        CHECK(status IN ('pending', 'in_progress', 'completed', 'blocked', 'cancelled')),
    priority INTEGER DEFAULT 5,        -- 1=highest, 10=lowest
    blocked_by TEXT,                   -- description of blocker
    spec_section_id INTEGER,           -- link to relevant spec
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    completed_at TEXT,
    FOREIGN KEY (spec_section_id) REFERENCES spec_sections(id)
);

CREATE VIRTUAL TABLE tasks_fts USING fts5(
    description,
    content='tasks', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER tasks_ai AFTER INSERT ON tasks BEGIN
    INSERT INTO tasks_fts(rowid, description) VALUES (NEW.id, NEW.description);
END;

CREATE TRIGGER tasks_ad AFTER DELETE ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, description)
    VALUES('delete', OLD.id, OLD.description);
END;

CREATE TRIGGER tasks_au AFTER UPDATE ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, description)
    VALUES('delete', OLD.id, OLD.description);
    INSERT INTO tasks_fts(rowid, description) VALUES (NEW.id, NEW.description);
END;
```

### Iterations (each loop cycle)
```sql
CREATE TABLE iterations (
    id INTEGER PRIMARY KEY,
    task_id INTEGER NOT NULL,
    status TEXT DEFAULT 'in_progress'
        CHECK(status IN ('in_progress', 'completed', 'failed', 'reverted')),
    attempt_number INTEGER DEFAULT 1,  -- which attempt at this task (1, 2, 3)
    started_at TEXT DEFAULT CURRENT_TIMESTAMP,
    ended_at TEXT,
    duration_seconds REAL,             -- for statistics
    commit_hash TEXT,                  -- git commit on success
    notes TEXT,
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);
```

### Actions (what was attempted)
```sql
CREATE TABLE actions (
    id INTEGER PRIMARY KEY,
    iteration_id INTEGER NOT NULL,
    action_type TEXT NOT NULL,         -- 'code_change', 'build', 'test', 'fix_attempt'
    description TEXT NOT NULL,
    file_path TEXT,                    -- affected file if applicable
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (iteration_id) REFERENCES iterations(id)
);
```

### Outcomes (what happened)
```sql
CREATE TABLE outcomes (
    id INTEGER PRIMARY KEY,
    action_id INTEGER NOT NULL,
    success INTEGER NOT NULL,          -- 1=success, 0=failure
    output_summary TEXT,               -- brief description
    error_message TEXT,                -- if failed
    duration_seconds REAL,             -- for statistics
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (action_id) REFERENCES actions(id)
);
```

### Failure Patterns (categorized failure types)
```sql
CREATE TABLE failure_patterns (
    id INTEGER PRIMARY KEY,
    pattern_key TEXT UNIQUE NOT NULL,  -- e.g., "ImportError", "TestFailure:auth"
    description TEXT NOT NULL,
    category TEXT,                     -- 'build', 'test', 'runtime', 'syntax'
    occurrence_count INTEGER DEFAULT 0,
    first_seen_at TEXT DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT
);
```

### Failures (specific failure instances)
```sql
CREATE TABLE failures (
    id INTEGER PRIMARY KEY,
    iteration_id INTEGER NOT NULL,
    pattern_id INTEGER,                -- link to pattern if identified
    error_text TEXT NOT NULL,          -- actual error message
    file_path TEXT,
    line_number INTEGER,
    resolved INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT,
    resolved_by_solution_id INTEGER,   -- which solution fixed it
    FOREIGN KEY (iteration_id) REFERENCES iterations(id),
    FOREIGN KEY (pattern_id) REFERENCES failure_patterns(id),
    FOREIGN KEY (resolved_by_solution_id) REFERENCES solutions(id)
);

CREATE VIRTUAL TABLE failures_fts USING fts5(
    error_text,
    content='failures', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER failures_ai AFTER INSERT ON failures BEGIN
    INSERT INTO failures_fts(rowid, error_text) VALUES (NEW.id, NEW.error_text);
END;

CREATE TRIGGER failures_ad AFTER DELETE ON failures BEGIN
    INSERT INTO failures_fts(failures_fts, rowid, error_text)
    VALUES('delete', OLD.id, OLD.error_text);
END;

CREATE TRIGGER failures_au AFTER UPDATE ON failures BEGIN
    INSERT INTO failures_fts(failures_fts, rowid, error_text)
    VALUES('delete', OLD.id, OLD.error_text);
    INSERT INTO failures_fts(rowid, error_text) VALUES (NEW.id, NEW.error_text);
END;
```

### Solutions (fixes with earned trust)
```sql
CREATE TABLE solutions (
    id INTEGER PRIMARY KEY,
    pattern_id INTEGER NOT NULL,       -- which failure pattern this solves
    description TEXT NOT NULL,         -- what to do
    code_example TEXT,                 -- example fix if applicable
    confidence REAL DEFAULT 0.3,       -- starts low, earns trust
    success_count INTEGER DEFAULT 0,
    failure_count INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    last_used_at TEXT,
    FOREIGN KEY (pattern_id) REFERENCES failure_patterns(id)
);

CREATE VIRTUAL TABLE solutions_fts USING fts5(
    description, code_example,
    content='solutions', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER solutions_ai AFTER INSERT ON solutions BEGIN
    INSERT INTO solutions_fts(rowid, description, code_example)
    VALUES (NEW.id, NEW.description, NEW.code_example);
END;

CREATE TRIGGER solutions_ad AFTER DELETE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, description, code_example)
    VALUES('delete', OLD.id, OLD.description, OLD.code_example);
END;

CREATE TRIGGER solutions_au AFTER UPDATE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, description, code_example)
    VALUES('delete', OLD.id, OLD.description, OLD.code_example);
    INSERT INTO solutions_fts(rowid, description, code_example)
    VALUES (NEW.id, NEW.description, NEW.code_example);
END;
```

### Solution Applications (tracking when solutions were used)
```sql
CREATE TABLE solution_applications (
    id INTEGER PRIMARY KEY,
    solution_id INTEGER NOT NULL,
    failure_id INTEGER NOT NULL,
    iteration_id INTEGER NOT NULL,
    success INTEGER,                   -- 1=worked, 0=didn't work, NULL=pending
    applied_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (solution_id) REFERENCES solutions(id),
    FOREIGN KEY (failure_id) REFERENCES failures(id),
    FOREIGN KEY (iteration_id) REFERENCES iterations(id)
);
```

---

## CLI Commands

### Project Setup
```bash
dial init [--phase NAME]              # Create .dial/ and database (default phase: "default")
dial init --phase mvp                 # Create .dial/mvp.db
dial init --phase api --import-solutions mvp  # New phase, import trusted solutions
dial index                            # Parse specs/*.md into FTS5
dial config set KEY VALUE             # Set config (build_cmd, test_cmd, etc.)
dial config show                      # Show current config
```

### Task Management
```bash
dial task add "description" [--priority N] [--spec SECTION_ID]
dial task list [--all]                # Show pending (or all) tasks
dial task next                        # Show highest priority pending task
dial task done ID                     # Mark task complete
dial task block ID "reason"           # Mark task blocked
dial task cancel ID                   # Cancel task
dial task search "query"              # FTS5 search tasks
```

### Iteration Control
```bash
dial iterate                          # Run ONE iteration
dial run [--max N]                    # Run until done or stuck (optional limit)
dial stop                             # Stop after current iteration (creates .dial/stop flag)
```

### Status and History
```bash
dial status                           # Current state summary
dial history [--limit N]              # Iteration history
dial failures [--unresolved]          # Show failures
dial solutions [--trusted]            # Show solutions
dial stats                            # Statistics dashboard
```

### Recovery
```bash
dial revert                           # Revert to last successful commit
dial reset                            # Reset current iteration, keep task pending
```

### Spec Queries
```bash
dial spec search "query"              # FTS5 search specs
dial spec show SECTION_ID             # Show specific section
dial spec list                        # List all indexed sections
```

---

## Solution Trust System

**Initial confidence:** 0.3 (untrusted)

**On successful application:**
- confidence += 0.15
- success_count += 1
- Cap at 1.0

**On failed application:**
- confidence -= 0.20
- failure_count += 1
- Floor at 0.0

**Trusted threshold:** >= 0.6

**Trust progression:**
```
0.30 (new)
  ↓ +0.15 (1st success)
0.45 (untrusted)
  ↓ +0.15 (2nd success)
0.60 (TRUSTED)
  ↓ -0.20 (1 failure)
0.40 (back to untrusted)
```

This prevents:
- Cargo-cult learning (one lucky success doesn't create trust)
- Superstition (solutions must prove themselves repeatedly)

---

## Iteration Flow

```
┌─────────────────────────────────────────────────────────────┐
│ 1. SELECT TASK                                              │
│    Query: SELECT * FROM tasks                               │
│           WHERE status='pending'                            │
│           ORDER BY priority, id LIMIT 1                     │
│    If no tasks → report and exit                            │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ 2. CREATE ITERATION                                         │
│    INSERT INTO iterations (task_id, status)                 │
│    Update task status to 'in_progress'                      │
│    Record started_at timestamp                              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ 3. GATHER CONTEXT (selective queries)                       │
│    - Query spec_sections_fts for task-relevant specs        │
│    - Query trusted solutions for similar past failures      │
│    - Query recent failures for patterns to avoid            │
│    Output: context block for agent                          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ 4. DO WORK                                                  │
│    - Agent implements the task                              │
│    - Record actions in actions table                        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ 5. VALIDATE                                                 │
│    - Run build command (timeout: build_timeout)             │
│    - If build passes: run test command (timeout: test_timeout)
│    - Record outcomes with duration                          │
└─────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    │                   │
                    ▼                   ▼
┌───────────────────────────┐ ┌─────────────────────────────────┐
│ 6a. ON SUCCESS            │ │ 6b. ON FAILURE                  │
│  - git add changed files  │ │  - Parse error, find/create     │
│  - git commit             │ │    failure pattern              │
│  - Record commit hash     │ │  - Record failure               │
│  - Mark iteration done    │ │  - Check for trusted solutions  │
│  - Mark task completed    │ │  - If solution: attempt fix     │
│  - Update solution trust  │ │  - If attempt < 3: retry        │
│    if solution was used   │ │  - If attempt >= 3: revert      │
└───────────────────────────┘ │  - Mark iteration failed        │
                              └─────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ 7. CHECK STOP FLAG                                          │
│    If .dial/stop exists → remove flag, exit                 │
│    Else → return to step 1                                  │
└─────────────────────────────────────────────────────────────┘
```

---

## Statistics Dashboard

Command: `dial stats`

```
DIAL Statistics: myproject (phase: mvp)
═══════════════════════════════════════════════════════════════

Iterations
  Total:          47
  Successful:     42 (89.4%)
  Failed:          5 (10.6%)

Tasks
  Completed:      38
  Pending:         3
  Blocked:         1
  Cancelled:       0

Time
  Total runtime:       1h 47m
  Avg iteration:       2.3 min
  Longest iteration:  12.4 min (task #14: "Implement OAuth flow")

Failure Patterns (top 5)
  ImportError              12 occurrences
  TestFailure:auth          8 occurrences
  SyntaxError               3 occurrences
  TypeError                 2 occurrences
  ConnectionError           1 occurrence

Solutions
  Total:           9
  Trusted (≥0.6):  7
  Hit rate:       73% (22 applications, 16 successful)

═══════════════════════════════════════════════════════════════
```

The schema already supports this - stats are computed from:
- `iterations` table (counts, durations)
- `tasks` table (status counts)
- `failure_patterns` table (occurrence_count)
- `solutions` table (confidence, success_count, failure_count)
- `solution_applications` table (hit rate)

---

## Colored Output

Helper functions (no dependencies, just ANSI codes):

```python
def green(text):  return f"\033[32m{text}\033[0m"
def red(text):    return f"\033[31m{text}\033[0m"
def yellow(text): return f"\033[33m{text}\033[0m"
def bold(text):   return f"\033[1m{text}\033[0m"
def dim(text):    return f"\033[2m{text}\033[0m"
```

Used for:
- Success messages (green)
- Errors and failures (red)
- Warnings (yellow)
- Headers and emphasis (bold)
- Secondary info (dim)

---

## Phase Support

Phases allow multiple DIAL runs in the same project:

```bash
# Phase 1: MVP
dial init --phase mvp
dial task add "User registration"
dial task add "User login"
dial run
# Creates .dial/mvp.db

# Phase 2: API (inherits trusted solutions from MVP)
dial init --phase api --import-solutions mvp
dial task add "REST endpoints"
dial task add "API authentication"
dial run
# Creates .dial/api.db with solutions copied from mvp.db
```

**Database location:** `.dial/{phase}.db`

**Import solutions:** Copies rows from `solutions` table where `confidence >= 0.6`

**Use cases:**
- Distinct project phases (MVP → v2 → optimization)
- Fresh task list but keep learned solutions
- Parallel workstreams

---

## File Structure

```
~/.dial/                      # DIAL installation
└── dial.py                   # Single-file implementation

~/bin/dial                    # Symlink to ~/.dial/dial.py

project/                      # Any project using DIAL
├── .dial/
│   ├── default.db            # Default phase database
│   ├── mvp.db                # MVP phase (if created)
│   ├── api.db                # API phase (if created)
│   └── stop                  # Stop flag (created by `dial stop`)
├── specs/
│   ├── overview.md
│   ├── auth.md
│   └── api.md
└── src/
    └── ...
```

---

## What Gets Removed/Replaced

| Old (markdown) | New (database) |
|----------------|----------------|
| fix_plan.md | tasks table |
| AGENT.md lessons | solutions + failures tables |
| Reading entire spec files | spec_sections_fts queries |
| No failure memory | failure_patterns + failures |
| No solution learning | solutions with trust scores |
| Single monolithic run | Phase support for staged work |

---

## Future Memloft Integration (not in scope now)

Later, memloft could optionally:
- Import trusted solutions from `.dial/*.db` as global lessons
- Sync task completions to memloft's task history
- Share failure patterns across projects

This is additive - DIAL works standalone first.

---

## Implementation Sequence

Build in this order (each step depends on previous):

### 1. Foundation
- [ ] Argument parser (argparse)
- [ ] Database connection and schema creation
- [ ] Config get/set
- [ ] Colored output helpers
- [ ] `dial init` command with phase support

### 2. Task Management
- [ ] `dial task add` with priority and spec link
- [ ] `dial task list` with status filtering
- [ ] `dial task next` (highest priority pending)
- [ ] `dial task done/block/cancel`
- [ ] `dial task search` (FTS5)

### 3. Spec Indexer
- [ ] Markdown parser (split on # headers)
- [ ] `dial index` command
- [ ] `dial spec search` (FTS5)
- [ ] `dial spec show` and `dial spec list`

### 4. Iteration Core
- [ ] `dial iterate` - single iteration
- [ ] Iteration record creation
- [ ] Action/outcome recording
- [ ] Build/test execution with timeout
- [ ] Git commit on success
- [ ] Git revert on failure (after 3 attempts)

### 5. Learning System
- [ ] Failure pattern detection (parse error messages)
- [ ] Failure recording with pattern link
- [ ] Solution recording
- [ ] Trust calculation (+0.15/-0.20)
- [ ] Solution lookup during iterations
- [ ] `dial solutions` command

### 6. Run Loop
- [ ] `dial run` - continuous iteration
- [ ] `dial stop` - graceful stop via flag
- [ ] Empty queue detection and reporting
- [ ] `--max N` limit

### 7. Status and Stats
- [ ] `dial status` - current state summary
- [ ] `dial history` - iteration history
- [ ] `dial failures` - failure list
- [ ] `dial stats` - statistics dashboard

---

## Success Criteria

DIAL upgrade is complete when:

1. `dial init --phase NAME` creates working database
2. `dial task add/list/next/done` manages task queue
3. `dial index` parses specs into FTS5
4. `dial spec search` returns relevant sections
5. `dial iterate` runs one complete loop with validation
6. `dial run` executes multiple iterations autonomously
7. Failures are recorded with patterns
8. Solutions accumulate trust through repeated success
9. `dial stats` shows meaningful statistics
10. Context size stays constant regardless of iteration count
11. Phase support allows fresh starts with solution inheritance
