# DIAL Engine Hardening — Core Architecture Improvements

Version: 4.0.0
Status: Specification
Phase: engine-hardening

---

## 1. Overview

Ten improvements to the DIAL core engine organized in three tiers: reliability (Tier 1), intelligence (Tier 2), and usability (Tier 3). All changes are to the `dial-core` library and `dial-cli` binary. No external application dependencies.

### Goals
- Make the autonomous loop atomically safe (checkpoint, transactions, structured signals)
- Close the feedback loop between failures and solutions
- Give the engine memory across iterations, not just within them
- Surface actionable intelligence about loop health

### Non-Goals
- Full application UI or dashboard
- Project management features (owners, estimates, sprints)
- Async database migration (rusqlite sync is fine for SQLite)
- Parallel validation step execution

---

## 2. Tier 1 — Reliability

### 2.1 Checkpoint/Rollback System

**Problem:** When validation fails, file changes remain on disk. The next retry attempt starts from a broken state rather than a known-good baseline.

**Solution:** Before each iteration begins, create a git stash checkpoint. On validation failure, restore to the checkpoint. On success, drop the checkpoint.

**Implementation:**
- Add `checkpoint_create()` and `checkpoint_restore()` to `git.rs`
- `checkpoint_create()`: runs `git stash push -u -m "dial-checkpoint-{iteration_id}"` to capture all tracked and untracked changes
- `checkpoint_restore()`: runs `git stash pop` to restore the stashed state, then `git checkout -- .` to clean the working tree back to the checkpoint
- If no changes exist when creating checkpoint, skip (nothing to checkpoint)
- Add `checkpoint_drop()` for successful iterations: `git stash drop` the named stash
- Wire into `iterate()`: create checkpoint before task execution begins
- Wire into `validate()`: on failure, call `checkpoint_restore()`; on success, call `checkpoint_drop()`
- Add config key `enable_checkpoints` (default: true) to allow disabling
- Emit events: `CheckpointCreated`, `CheckpointRestored`, `CheckpointDropped`

**Files modified:**
- `dial-core/src/git.rs` — new checkpoint functions
- `dial-core/src/iteration/mod.rs` — wire checkpoint into iterate/validate
- `dial-core/src/engine.rs` — expose checkpoint config
- `dial-core/src/event.rs` — new event variants

**Testing:**
- Unit test: checkpoint_create with dirty working tree
- Unit test: checkpoint_create with clean working tree (no-op)
- Unit test: checkpoint_restore reverts changes
- Integration test: full iterate→fail→restore→retry cycle

---

### 2.2 Structured Subagent Signals

**Problem:** DIAL parses `DIAL_COMPLETE`, `DIAL_BLOCKED`, `DIAL_LEARNING` via regex from stdout. AI output could contain these strings in code blocks or explanations, causing false matches.

**Solution:** Replace stdout regex parsing with a signal file. The subagent writes a JSON file to `.dial/signal.json` and the orchestrator reads it after the subprocess exits.

**Implementation:**
- Define `SubagentSignal` enum:
  ```
  Complete { summary: String }
  Blocked { reason: String }
  Learning { category: String, description: String }
  ```
- Define `SignalFile` struct: `{ signals: Vec<SubagentSignal>, timestamp: String }`
- Subagent prompt instructs AI to write `.dial/signal.json` instead of printing DIAL_ prefixed lines
- Orchestrator: after subprocess exits, read and parse `.dial/signal.json`
- Fallback: if signal file doesn't exist, fall back to current regex parsing for backward compatibility
- Delete signal file after reading to prevent stale signals
- Update `subagent_prompt.md` template generation to use new signal format

**Files modified:**
- `dial-core/src/iteration/orchestrator.rs` — signal file reading, fallback logic
- `dial-core/src/iteration/context.rs` — update prompt template with signal file instructions
- New file: `dial-core/src/iteration/signal.rs` — SignalFile and SubagentSignal types, read/write/parse

**Testing:**
- Unit test: parse valid signal file with all three signal types
- Unit test: parse signal file with missing fields (graceful error)
- Unit test: fallback to regex when no signal file exists
- Unit test: signal file cleanup after reading
- Integration test: orchestrator reads signal file from mock subagent

---

### 2.3 Transaction Safety

**Problem:** Multi-step database operations (record failure → update pattern → check solutions → update metrics) aren't wrapped in transactions. A crash mid-operation could leave orphaned or inconsistent records.

**Solution:** Wrap all multi-step Engine methods in explicit SQLite transactions.

**Implementation:**
- Identify all multi-step DB operations:
  - `record_failure()` — insert failure + update pattern occurrence_count + check solutions
  - `validate()` — multiple step results + metrics recording
  - `iterate()` complete flow — create iteration + update task status + record actions
  - `task_done()` — update task + auto-unblock dependents
  - `auto_run()` inner loop — iteration + validation + commit + metrics
  - `prd_import()` — multiple section inserts + source tracking
- Add helper method `Engine::with_transaction<F, R>(f: F) -> Result<R>` that:
  - Calls `conn.execute("BEGIN IMMEDIATE")`
  - Runs the closure
  - On Ok: `conn.execute("COMMIT")`
  - On Err: `conn.execute("ROLLBACK")`
- Wrap each identified operation in `with_transaction()`
- For operations spanning both phase DB and prd.db, use SAVEPOINT for nested transactions within each DB

**Files modified:**
- `dial-core/src/db.rs` — add `with_transaction()` helper
- `dial-core/src/engine.rs` — wrap multi-step methods
- `dial-core/src/failure.rs` — wrap record_failure
- `dial-core/src/iteration/mod.rs` — wrap iterate/validate
- `dial-core/src/task/mod.rs` — wrap task_done + auto_unblock
- `dial-core/src/prd/import.rs` — wrap prd_import

**Testing:**
- Unit test: transaction commits on success
- Unit test: transaction rolls back on error
- Unit test: nested savepoints work correctly
- Integration test: simulate crash during record_failure, verify no orphaned records

---

### 2.4 Solution Auto-Suggestion

**Problem:** Solutions exist with trust scores but are only passively included as context text. The engine doesn't actively match failures to solutions.

**Solution:** When a failure is recorded and matches a known pattern, immediately query for trusted solutions for that pattern and emit an event with the suggestion.

**Implementation:**
- Add `find_solutions_for_pattern(pattern_id: i64) -> Vec<Solution>` that queries solutions with confidence >= TRUST_THRESHOLD for the given pattern
- In `record_failure()`, after pattern matching:
  1. Query trusted solutions for the matched pattern
  2. If solutions found, emit `SolutionSuggested { failure_id, solutions: Vec<(solution_id, description, confidence)> }`
  3. Include solution descriptions in the failure context for the next retry attempt
- Add `Solution` struct to public API if not already exposed
- In context assembly (`gather_context_budgeted`), when there are recent failures with matched solutions, include them at priority 15 (between task_spec and fts_specs) with clear formatting: "KNOWN FIX (confidence: 0.85): description"
- Track whether a suggested solution was applied via `solution_applications` table
- After successful validation, if a solution was suggested and the task passed, auto-increment the solution's confidence

**Files modified:**
- `dial-core/src/failure.rs` — add find_solutions_for_pattern, update record_failure
- `dial-core/src/event.rs` — add SolutionSuggested event variant
- `dial-core/src/budget.rs` — include suggested solutions in context with appropriate priority
- `dial-core/src/iteration/context.rs` — format solution suggestions in context
- `dial-core/src/engine.rs` — wire solution feedback after validation

**Testing:**
- Unit test: find_solutions_for_pattern returns only trusted solutions
- Unit test: find_solutions_for_pattern returns empty for unknown pattern
- Integration test: record failure → solution suggested → retry succeeds → confidence incremented

---

## 3. Tier 2 — Intelligence

### 3.1 Cross-Iteration Failure Tracking

**Problem:** The max 3 attempt limit is per-task within a single auto-run session. If a task fails 3 times, gets blocked, is unblocked later, the history resets. Chronically failing tasks aren't identified.

**Solution:** Track cumulative attempt history across all iterations for each task. Surface tasks that fail repeatedly across sessions.

**Implementation:**
- Add migration 11: `ALTER TABLE tasks ADD COLUMN total_attempts INTEGER DEFAULT 0`
- Add migration 11: `ALTER TABLE tasks ADD COLUMN total_failures INTEGER DEFAULT 0`
- Add migration 11: `ALTER TABLE tasks ADD COLUMN last_failure_at TEXT`
- When an iteration fails for a task, increment `total_failures` and update `last_failure_at`
- When an iteration starts for a task, increment `total_attempts`
- Add `Engine::chronic_failures(threshold: u32) -> Vec<Task>` — returns tasks where total_failures >= threshold
- Add `dial task chronic [--threshold N]` CLI command (default threshold: 5)
- In auto_run, before attempting a task: check total_failures. If >= configurable `max_total_failures` (default: 10), auto-block the task with reason "Chronic failure: {N} total failures across {M} sessions"
- Emit `ChronicFailureDetected { task_id, total_failures, total_attempts }` event

**Files modified:**
- `dial-core/src/db.rs` — migration 11
- `dial-core/src/task/mod.rs` — update task creation, failure tracking
- `dial-core/src/engine.rs` — chronic_failures method, auto_run check
- `dial-core/src/event.rs` — ChronicFailureDetected event
- `dial-cli/src/main.rs` — `task chronic` subcommand

**Testing:**
- Unit test: total_attempts increments across iterations
- Unit test: total_failures increments only on failure
- Unit test: chronic_failures query returns correct tasks
- Integration test: task fails across multiple auto_run sessions, gets auto-blocked

---

### 3.2 Similar Completed Task Context

**Problem:** When starting a new task, the AI gets specs, solutions, and learnings — but not examples of how similar tasks were completed successfully.

**Solution:** Use FTS to find completed tasks with similar descriptions and include their iteration notes in the context.

**Implementation:**
- Add `find_similar_completed_tasks(description: &str, limit: usize) -> Vec<(Task, String)>` that:
  1. FTS query against tasks_fts for completed tasks matching keywords from the description
  2. For each match, get the most recent successful iteration's notes and commit_hash
  3. Return task + notes pairs, ordered by FTS rank
- In `gather_context_budgeted()`, add similar completed tasks at priority 25 (between trusted solutions and failures):
  - Format: "SIMILAR COMPLETED TASK: {description}\nApproach: {iteration_notes}\nCommit: {commit_hash}"
  - Limit to 3 similar tasks
- Strip common words from search query to improve FTS relevance (the, a, an, is, for, etc.)

**Files modified:**
- `dial-core/src/task/mod.rs` — add find_similar_completed_tasks
- `dial-core/src/budget.rs` — add similar tasks priority constant
- `dial-core/src/iteration/context.rs` — include similar tasks in context assembly

**Testing:**
- Unit test: FTS returns relevant completed tasks
- Unit test: no results when no similar tasks exist
- Unit test: limit is respected
- Integration test: task context includes similar completed task notes

---

### 3.3 Per-Pattern Metrics

**Problem:** Metrics are per-iteration only. There's no way to see which failure patterns cost the most time/tokens or which get resolved fastest.

**Solution:** Aggregate metrics by failure pattern, providing per-pattern cost and resolution statistics.

**Implementation:**
- Add `PatternMetrics` struct:
  ```
  pattern_key: String
  category: String
  total_occurrences: i64
  total_resolution_time_secs: f64
  avg_resolution_time_secs: f64
  total_tokens_consumed: i64
  total_cost_usd: f64
  auto_resolved_count: i64      // resolved by trusted solution
  manual_resolved_count: i64    // resolved by retry without solution
  unresolved_count: i64
  first_seen: String
  last_seen: String
  ```
- Add `Engine::pattern_metrics() -> Vec<PatternMetrics>` that joins failures, iterations, provider_usage, and solutions to compute aggregates per pattern
- Add `dial patterns metrics` CLI command with table output
- Add `dial patterns metrics --format json` for machine-readable output
- Add `dial patterns metrics --sort cost` to sort by most expensive pattern

**Files modified:**
- `dial-core/src/failure.rs` — PatternMetrics struct, compute_pattern_metrics query
- `dial-core/src/engine.rs` — pattern_metrics method
- `dial-cli/src/main.rs` — patterns metrics subcommand

**Testing:**
- Unit test: pattern_metrics correctly aggregates across multiple failures
- Unit test: pattern_metrics handles patterns with no failures
- Unit test: sorting by cost/time/occurrences works
- Integration test: full cycle from failure → resolution → metrics aggregation

---

### 3.4 Learning-to-Pattern Linking

**Problem:** Learnings are recorded alongside failures but there's no link between them. A learning discovered because of a specific failure pattern isn't automatically surfaced when that pattern recurs.

**Solution:** Add an optional pattern_id foreign key to learnings. When a learning is recorded during a failed iteration, auto-link it to the detected pattern. Surface linked learnings when the pattern recurs.

**Implementation:**
- Add migration 11 (same migration as 3.1): `ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id)`
- Add migration 11: `ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id)`
- When `learn()` is called during an active iteration that has a recorded failure, auto-set pattern_id from the most recent failure's pattern
- Add `Engine::learnings_for_pattern(pattern_id: i64) -> Vec<Learning>` query
- In context assembly: when a failure matches a pattern, also include learnings linked to that pattern at priority 35 (just above general learnings at 40)
- Format: "LEARNING (from pattern: {pattern_key}): {description}"
- CLI: `dial learnings list --pattern <pattern_key>` to filter by pattern
- When recording a DIAL_LEARNING signal in auto_run, check if current iteration has failures and link

**Files modified:**
- `dial-core/src/db.rs` — migration 11 (combined with cross-iteration columns)
- `dial-core/src/learning.rs` — add pattern_id/iteration_id to learn(), learnings_for_pattern query
- `dial-core/src/iteration/context.rs` — include pattern-linked learnings
- `dial-core/src/iteration/orchestrator.rs` — link learnings to patterns during auto_run
- `dial-core/src/engine.rs` — learnings_for_pattern method
- `dial-cli/src/main.rs` — --pattern flag on learnings list

**Testing:**
- Unit test: learning auto-linked to pattern during failed iteration
- Unit test: learnings_for_pattern returns only linked learnings
- Unit test: unlinked learnings still work (backward compat)
- Integration test: failure → learning recorded → pattern recurs → learning surfaced in context

---

## 4. Tier 3 — Usability

### 4.1 Dry Run / Preview Mode

**Problem:** No way to inspect what DIAL would do without actually doing it. Users can't verify context assembly or task selection logic.

**Solution:** Add `--dry-run` flag to `dial iterate` and `dial auto-run` that shows what would happen without executing.

**Implementation:**
- Add `Engine::iterate_dry_run() -> DryRunResult` that:
  1. Selects next task (same logic as iterate)
  2. Assembles context with budget (same logic)
  3. Generates subagent prompt (same logic)
  4. Returns everything without creating iteration records or spawning subagents
- `DryRunResult` struct:
  ```
  task: Task
  context_items_included: Vec<(String, usize)>   // (label, tokens)
  context_items_excluded: Vec<(String, usize)>
  total_context_tokens: usize
  token_budget: usize
  prompt_preview: String                           // first 500 chars of generated prompt
  suggested_solutions: Vec<String>                 // if task has prior failures with solutions
  dependencies_satisfied: bool
  ```
- CLI: `dial iterate --dry-run` — pretty-print the DryRunResult
- CLI: `dial auto-run --dry-run` — show first N tasks that would be attempted in order
- JSON output: `dial iterate --dry-run --format json`

**Files modified:**
- `dial-core/src/engine.rs` — iterate_dry_run method, DryRunResult struct
- `dial-core/src/iteration/context.rs` — refactor context assembly to be reusable without side effects
- `dial-cli/src/main.rs` — --dry-run flag on iterate and auto-run commands

**Testing:**
- Unit test: dry run doesn't create iteration records
- Unit test: dry run returns correct task selection
- Unit test: dry run shows excluded context items
- Integration test: dry run matches what actual iterate would do

---

### 4.2 Project Health Score

**Problem:** `dial stats` shows raw numbers but doesn't tell users whether their autonomous loop is improving or declining.

**Solution:** Compute a single 0-100 health score from weighted factors, with trend indication.

**Implementation:**
- Add `HealthScore` struct:
  ```
  score: u32                    // 0-100
  trend: Trend                  // Improving, Stable, Declining
  factors: Vec<HealthFactor>
  ```
- `HealthFactor` struct:
  ```
  name: String
  score: u32                    // 0-100 for this factor
  weight: f64                   // contribution weight
  detail: String                // human-readable explanation
  ```
- Factors and weights:
  1. **Success rate** (weight 0.30) — recent 20 iterations success rate, scaled 0-100
  2. **Success trend** (weight 0.15) — comparing last 10 vs previous 10 success rates
  3. **Solution confidence** (weight 0.15) — average confidence of all solutions, scaled
  4. **Blocked task ratio** (weight 0.15) — (total - blocked) / total * 100
  5. **Learning utilization** (weight 0.10) — learnings with times_referenced > 0 / total learnings
  6. **Pattern resolution rate** (weight 0.15) — resolved failures / total failures
- Trend calculation: compare current score vs score from 7 days ago (if data exists)
- Add `Engine::health() -> HealthScore`
- CLI: `dial health` — colored output (green >= 70, yellow >= 40, red < 40)
- CLI: `dial health --format json`

**Files modified:**
- New file: `dial-core/src/health.rs` — HealthScore, HealthFactor, compute_health
- `dial-core/src/lib.rs` — pub mod health
- `dial-core/src/engine.rs` — health() method
- `dial-cli/src/main.rs` — health command

**Testing:**
- Unit test: perfect project scores 100
- Unit test: empty project scores 50 (neutral baseline)
- Unit test: all-failing project scores < 20
- Unit test: trend calculation with sufficient/insufficient data
- Integration test: health score changes appropriately after successes and failures

---

## 5. Migration Strategy

All database changes are consolidated into migration 11:
- `ALTER TABLE tasks ADD COLUMN total_attempts INTEGER DEFAULT 0`
- `ALTER TABLE tasks ADD COLUMN total_failures INTEGER DEFAULT 0`
- `ALTER TABLE tasks ADD COLUMN last_failure_at TEXT`
- `ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id)`
- `ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id)`

No breaking changes to existing data. All new columns have defaults or are nullable.

## 6. Version

These changes constitute DIAL v4.0.0 — a major version bump reflecting significant architectural improvements to the core engine. The library API surface grows but nothing existing breaks.

## 7. Testing Requirements

Minimum 50 new tests across all tiers:
- Each feature: 3-4 unit tests + 1 integration test
- All existing 201 tests must continue to pass
- `cargo test` from workspace root must pass with zero failures
- `cargo clippy` must pass with no warnings
