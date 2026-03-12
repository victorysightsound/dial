# Unified Project Wizard (`dial new`)

## 1. Overview

Rewrite the PRD wizard into a seamless 9-phase guided flow that takes a user from zero to autonomous iteration. One command (`dial new`) replaces the current multi-step manual process of `dial init` + `dial config set` + `dial spec wizard` + `dial task add` + `dial auto-run`.

The wizard state persists after every phase so the user can close the terminal and resume later with `dial new --resume`. The existing `dial spec wizard` command continues to work but only runs phases 1-5 (PRD generation).

## 2. Phase Definitions

### 2.1 Phase 1: Vision (existing)

Gathers project name, elevator pitch, problem statement, target users, success criteria, scope exclusions. AI provider call, JSON response.

No changes to the prompt or response schema.

### 2.2 Phase 2: Functionality (existing)

Defines MVP features, deferred features, user workflows. AI provider call, JSON response.

No changes to the prompt or response schema.

### 2.3 Phase 3: Technical (existing)

Defines data model, integrations, platform stack, constraints, performance requirements. AI provider call, JSON response.

No changes to the prompt or response schema.

### 2.4 Phase 4: Gap Analysis (existing)

Reviews all gathered info and identifies gaps, contradictions, ambiguities. AI provider call, JSON response.

No changes to the prompt or response schema.

### 2.5 Phase 5: Generate (existing)

Generates PRD sections and terminology, inserts into prd.db, creates initial DIAL tasks. AI provider call, JSON response.

No changes to the core generation logic. One addition: the generated tasks should include dependency relationships where the AI identifies them (see section 3.3).

### 2.6 Phase 6: Task Review

After phase 5 generates tasks, send the full task list back to the AI provider for review and refinement. The prompt includes:
- All generated tasks with their descriptions and priorities
- The full PRD context (gathered_info from phases 1-5)
- Instructions to: reorder by logical implementation sequence, add missing tasks, remove redundant ones, set dependency relationships, assign realistic priorities (1 = first, higher = later)

Expected JSON response:
```json
{
  "tasks": [
    {
      "description": "task description",
      "priority": 1,
      "spec_section": "1.2",
      "depends_on": [],
      "rationale": "why this order"
    }
  ],
  "removed": [
    {"original": "task that was removed", "reason": "why"}
  ],
  "added": [
    {"description": "new task", "reason": "why it was missing"}
  ]
}
```

After parsing: clear existing auto-generated tasks from phase 5, insert the reviewed task list with dependencies. Store the review rationale in wizard state for context.

### 2.7 Phase 7: Build & Test Configuration

The AI provider is given the technical details from phase 3 (languages, frameworks, platform) and asked to suggest build and test commands. The prompt also asks for validation pipeline steps if the project has multiple validation concerns (e.g., lint + build + test + integration test).

Expected JSON response:
```json
{
  "build_cmd": "cargo build",
  "test_cmd": "cargo test",
  "pipeline_steps": [
    {"name": "lint", "command": "cargo clippy", "order": 1, "required": true, "timeout": 120},
    {"name": "build", "command": "cargo build", "order": 2, "required": true, "timeout": 300},
    {"name": "test", "command": "cargo test", "order": 3, "required": true, "timeout": 300}
  ],
  "build_timeout": 600,
  "test_timeout": 600,
  "rationale": "why these commands"
}
```

After parsing: write `build_cmd`, `test_cmd`, `build_timeout`, `test_timeout` to the config table. If `pipeline_steps` are provided and non-empty, insert them into `validation_steps`. Store in wizard state.

### 2.8 Phase 8: Iteration Mode

The AI provider is given the full project context (scope, task count, complexity from gap analysis) and asked to recommend an iteration mode. The prompt explains the available modes and asks the AI to recommend one based on project characteristics, then outputs the recommendation.

The iteration modes are:

| Mode | Config Value | Behavior |
|------|-------------|----------|
| Autonomous | `autonomous` | Run all tasks, commit on pass, no stops. Maps to `ApprovalMode::Auto`. |
| Review every N | `review_every:N` | Pause for review after every N completed tasks. Maps to `ApprovalMode::Review` with `review_interval` config. |
| Review each task | `review_each` | Pause after every task for approval. Maps to `ApprovalMode::Manual`. |

Expected JSON response:
```json
{
  "recommended_mode": "autonomous",
  "review_interval": null,
  "ai_cli": "claude",
  "subagent_timeout": 1800,
  "rationale": "why this mode"
}
```

After parsing: write `iteration_mode`, `review_interval` (if applicable), `ai_cli`, `subagent_timeout` to config. Set the engine's `ApprovalMode` accordingly. Store in wizard state.

### 2.9 Phase 9: Launch

This is NOT an AI provider call. This phase:
1. Prints a summary of everything configured (project name, task count, build/test commands, iteration mode, AI CLI)
2. Writes a `launch_ready` flag to wizard state
3. If running interactively, prints: "Project configured. Run `dial auto-run` to start autonomous iteration."
4. The `dial new` command returns successfully

The wizard does NOT start `auto-run` itself. This keeps the wizard a configuration tool and avoids it becoming a long-lived process. The user (or their AI agent) runs `dial auto-run` as a separate step.

## 3. Implementation Details

### 3.1 WizardPhase Enum Extension

Extend `WizardPhase` from 5 to 9 variants:
```rust
pub enum WizardPhase {
    Vision = 1,
    Functionality = 2,
    Technical = 3,
    GapAnalysis = 4,
    Generate = 5,
    TaskReview = 6,
    BuildTestConfig = 7,
    IterationMode = 8,
    Launch = 9,
}
```

Update `from_i32()`, `name()`, and `next()` accordingly.

### 3.2 WizardState Changes

No structural changes needed. The existing `gathered_info: JsonValue` field stores phase data keyed by phase name. New phases store their data under keys: `"task_review"`, `"build_test_config"`, `"iteration_mode"`, `"launch"`.

### 3.3 run_wizard() Refactor

The current `run_wizard()` calls `run_wizard_phases_1_3()` then `run_wizard_phases_4_5()`. Refactor to a single loop that iterates through all 9 phases:

```rust
pub async fn run_wizard(
    provider: &dyn Provider,
    prd_conn: &Connection,
    template: &str,
    from_doc: Option<&str>,
    resume: bool,
    full: bool,  // true for `dial new`, false for `dial spec wizard`
) -> Result<WizardResult>
```

The `full` parameter controls whether to run all 9 phases or stop at phase 5. When `full` is false, behavior is identical to current `dial spec wizard`.

The loop structure:
1. Determine which phases to run based on `full` flag
2. For each phase that hasn't been completed:
   a. Set current phase, save state
   b. Build the phase-specific prompt
   c. For phases 1-8: call provider, parse JSON, process results
   d. For phase 9: print summary (no provider call)
   e. Mark phase complete, save state

### 3.4 Phase 6 Task Replacement

Phase 6 (Task Review) must:
1. Read existing tasks from the phase DB (generated by phase 5)
2. Send them to the provider for review
3. Delete the phase 5 tasks
4. Insert the reviewed tasks with updated priorities and dependencies
5. Set up `task_dependencies` relationships

This requires access to the phase DB connection in addition to prd_conn. The current `run_wizard_phases_4_5` already does this for initial task creation (line 474: `crate::db::get_db(None)?`). Phase 6 follows the same pattern.

### 3.5 Phase 7 Config Writing

Phase 7 writes to the config table via `crate::config::config_set()`. It also optionally inserts validation pipeline steps. This requires the phase DB connection.

### 3.6 Phase 8 Iteration Mode Config

Phase 8 introduces a new config key `iteration_mode` with values: `autonomous`, `review_every:N`, `review_each`. The `auto_run()` function in `orchestrator.rs` must be updated to read this config and set the appropriate `ApprovalMode`.

Changes to `auto_run()`:
- Read `iteration_mode` from config at startup
- If `review_every:N`: track completed count, pause every N completions (set iteration to `awaiting_approval`)
- If `review_each`: set every completion to `awaiting_approval`
- If `autonomous` or not set: current behavior (auto-commit)

### 3.7 New `dial new` CLI Command

Add a `New` variant to the `Commands` enum:
```rust
/// Create a new project with guided setup
New {
    /// Template to use (spec, architecture, api, mvp)
    #[arg(long, default_value = "spec")]
    template: String,
    /// Existing document to refine
    #[arg(long)]
    from: Option<String>,
    /// Resume a paused wizard session
    #[arg(long)]
    resume: bool,
    /// Phase name for the DIAL database
    #[arg(long, default_value = "mvp")]
    phase: String,
}
```

The `dial new` handler:
1. If not `--resume`: run `Engine::init()` first (creates .dial/ and DB)
2. Set up the provider (from `ANTHROPIC_API_KEY` env var or configured CLI)
3. Call `engine.prd_wizard()` with `full: true`
4. Print launch instructions

### 3.8 Engine Method Updates

Add a new engine method or update the existing `prd_wizard()`:
```rust
pub async fn new_project(
    &self,
    template: &str,
    from_doc: Option<&str>,
    resume: bool,
) -> Result<()>
```

This calls `run_wizard()` with `full: true` and handles all event emissions.

### 3.9 Event Additions

Add new event variants for the new phases:
```rust
Event::TaskReviewCompleted { tasks_kept: usize, tasks_added: usize, tasks_removed: usize }
Event::BuildTestConfigured { build_cmd: String, test_cmd: String, pipeline_steps: usize }
Event::IterationModeSet { mode: String }
Event::LaunchReady { project_name: String, task_count: usize }
```

## 4. Migration Path

- `dial spec wizard` continues to work unchanged (runs phases 1-5 only)
- `dial new` is the new recommended entry point
- Existing projects can run `dial new --resume` to pick up from wherever they are, even if they started with the old wizard
- If phases 1-5 are already complete (from `dial spec wizard`), `dial new --resume` skips straight to phase 6

## 5. Testing

### 5.1 Unit Tests

- `WizardPhase` enum: `from_i32()` for all 9 values, `next()` chain, `name()` strings
- Phase prompt builders for phases 6, 7, 8 (verify they include correct prior context)
- JSON response parsing for phases 6, 7, 8 (valid responses, malformed responses, missing fields)
- Task replacement logic in phase 6 (tasks cleared and re-inserted with dependencies)
- Config writing in phase 7 (build_cmd, test_cmd, pipeline steps)
- Iteration mode parsing in phase 8 (all three modes, with and without interval)

### 5.2 Integration Tests

- Full 9-phase wizard with a mock provider that returns canned JSON for each phase
- Resume from each phase (save state at phase N, restart, verify phases 1..N skipped)
- `full: false` mode stops at phase 5 (backward compatibility)
- Phase 6 task replacement (verify old tasks deleted, new tasks with dependencies inserted)
- Phase 7 config writing (verify config table and validation_steps table)
- Phase 8 iteration mode config (verify config value written, auto_run reads it)

### 5.3 CLI Tests

- `dial new --template mvp` runs without error (with mock provider)
- `dial new --resume` loads state and continues
- `dial spec wizard` still works (phases 1-5 only)

## 6. Files Modified

| File | Changes |
|------|---------|
| `dial-core/src/prd/wizard.rs` | Extend WizardPhase (5→9), add phase 6-8 prompt builders, refactor run_wizard() to single loop with `full` flag, add phase 6 task replacement, phase 7 config writing, phase 8 mode config, phase 9 summary |
| `dial-core/src/engine.rs` | Add `new_project()` method, update `prd_wizard()` signature to accept `full` flag |
| `dial-core/src/event.rs` | Add 4 new event variants (TaskReviewCompleted, BuildTestConfigured, IterationModeSet, LaunchReady) |
| `dial-core/src/iteration/orchestrator.rs` | Read `iteration_mode` config, implement review_every:N pause logic |
| `dial-cli/src/main.rs` | Add `New` command variant, add handler that calls init + new_project, update `Wizard` handler to pass `full: false` |
| `dial-cli/src/cli_handler.rs` | Handle new event variants in CliEventHandler |
| `dial-core/src/errors.rs` | No changes expected (existing error variants sufficient) |

## 7. Non-Goals

- The wizard does NOT start `auto-run` itself. It configures everything and tells the user to run it.
- No interactive terminal prompting (stdin reads). All user input comes through `--from`, `--template`, and `--resume` flags. The AI provider generates the content.
- No changes to the PRD database schema. New data fits in existing tables.
- No changes to the task database schema. Dependencies use the existing `task_dependencies` table.
