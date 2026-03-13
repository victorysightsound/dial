# DIAL Loop Accuracy Improvements

Version: 4.1.0
Status: Specification
Phase: loop-accuracy

---

## 1. Overview

Four targeted improvements to increase loop accuracy and first-attempt success rate:

1. **Failed attempt diff capture** — preserve what the AI tried before checkpoint restore so retries don't repeat the same approach
2. **Spec specificity enforcement** — strengthen Phase 4 (Gap Analysis) to reject vague requirements and demand concrete acceptance criteria
3. **Task sizing analysis** — strengthen Phase 6 (Task Review) to split oversized tasks and flag vague descriptions
4. **Test coverage generation** — strengthen Phase 7 (Build & Test) to generate test tasks alongside feature tasks and require testable acceptance criteria

### Non-Goals
- New wizard phases (strengthen existing ones)
- Automated test generation (DIAL doesn't write code, it directs the AI)
- Static analysis integration

---

## 2. Failed Attempt Diff Capture

**Problem:** When a task fails and checkpoint_restore() reverts the working tree, the AI's failed changes are erased. On retry, the AI gets the error message but not the code it attempted. It may repeat the same approach.

**Solution:** Capture `git diff` before restoring the checkpoint and include it in retry context.

**Implementation:**
- In iteration/mod.rs, before calling checkpoint_restore() on validation failure:
  1. Run `git diff` to capture the full diff of attempted changes
  2. Run `git diff --stat` to capture the summary
  3. Store both in the iteration record's `notes` field as structured text:
     ```
     FAILED_DIFF_STAT:
     src/main.rs | 15 +++++++++------
     2 files changed, 9 insertions(+), 6 deletions(-)

     FAILED_DIFF:
     (full diff content, truncated to 2000 chars to fit token budget)
     ```
  4. Then proceed with checkpoint_restore() as normal
- In iteration/context.rs, when assembling context for a retry attempt (attempt > 1):
  1. Query the previous failed iteration for this task
  2. Extract the FAILED_DIFF_STAT and FAILED_DIFF from notes
  3. Include at priority 12 (between task_spec and suggested solutions):
     ```
     PREVIOUS ATTEMPT (failed):
     Error: <error message>
     Changes attempted:
     <diff stat>
     <truncated diff>
     DO NOT repeat this approach. Try a different solution.
     ```
- Add FAILED_DIFF_PRIORITY = 12 constant to budget.rs
- Truncate diff to 2000 chars to prevent blowing the token budget on large diffs

**Files modified:**
- `dial-core/src/iteration/mod.rs` — capture diff before checkpoint restore
- `dial-core/src/iteration/context.rs` — include failed diff in retry context
- `dial-core/src/budget.rs` — add FAILED_DIFF_PRIORITY constant
- `dial-core/src/git.rs` — add git_diff() and git_diff_stat() helper functions

**Testing:**
- Unit test: git_diff captures working tree changes
- Unit test: diff truncation at 2000 chars
- Unit test: retry context includes previous attempt diff
- Integration test: fail → diff captured → retry gets diff in context

---

## 3. Spec Specificity Enforcement (Phase 4 Enhancement)

**Problem:** Phase 4 (Gap Analysis) checks for gaps and contradictions but accepts vague requirements like "users can log in" without demanding concrete details.

**Solution:** Add a specificity analysis pass to Phase 4's prompt that identifies vague sections and requires concrete acceptance criteria.

**Implementation:**
- In prd/wizard.rs, update the Phase 4 (GapAnalysis) prompt builder to include a specificity check section:
  ```
  SPECIFICITY CHECK:
  Review each section for vague or incomplete requirements. Flag any section that:
  - Uses words like "should", "might", "could", "etc.", "various" without concrete details
  - Describes a feature without specifying exact behavior, inputs, outputs, or error cases
  - Lacks acceptance criteria (how do you know when it's done?)
  - References external systems without specifying the integration contract

  For each vague section, provide:
  1. The section ID and title
  2. What is vague
  3. Suggested concrete replacement text with specific acceptance criteria

  Rate each section: SPECIFIC (ready for tasks), NEEDS_DETAIL (usable but could be better), VAGUE (must be rewritten before task generation)

  Do not proceed to Phase 5 (Generate) with any VAGUE sections. Rewrite them now.
  ```
- The AI's response in Phase 4 should include the rewritten sections
- Parse the Phase 4 response to extract rewritten sections and update prd.db
- If the AI identifies VAGUE sections and provides rewrites, apply them to prd.db before moving to Phase 5
- Add a specificity_score field to the wizard state for tracking

**Files modified:**
- `dial-core/src/prd/wizard.rs` — update Phase 4 prompt, parse specificity results, update sections

**Testing:**
- Unit test: Phase 4 prompt includes specificity check instructions
- Unit test: parse specificity ratings from AI response
- Integration test: vague section gets rewritten before Phase 5

---

## 4. Task Sizing Analysis (Phase 6 Enhancement)

**Problem:** Phase 6 (Task Review) reorders and deduplicates tasks but doesn't analyze whether individual tasks are appropriately sized for a single AI iteration.

**Solution:** Add task sizing analysis to Phase 6's prompt that splits oversized tasks and flags vague descriptions.

**Implementation:**
- In prd/wizard.rs, update the Phase 6 (TaskReview) prompt builder to include sizing analysis:
  ```
  TASK SIZING ANALYSIS:
  Each task must be completable by an AI in a single focused session (10-15 minutes of work).

  For each task, evaluate:
  - SCOPE: Does this task do ONE thing? A task should touch 1-3 files maximum.
  - SPECIFICITY: Is the description concrete enough for an AI to implement without guessing?
    Bad: "Build the auth system"
    Good: "Add bcrypt password hashing to User model with cost factor 12"
  - TESTABILITY: Can success be verified by running build + test commands?

  Actions:
  - SPLIT any task that would require touching more than 3 files or implementing more than one feature
  - REWRITE any task description that is vague or ambiguous
  - MERGE any tasks that are too small to justify a separate iteration (single-line changes, comment additions)

  For each split, provide the new task descriptions and their dependency relationships.
  For each rewrite, provide the improved description.

  Output the final task list with sizing annotations: [S] small, [M] medium, [L] large-but-focused.
  Flag any [XL] tasks that could not be reduced — these need human review.
  ```
- Parse the Phase 6 response to extract split/rewrite/merge actions
- Apply task changes to the database (delete merged tasks, create split tasks, update descriptions)
- Emit TaskSplit event when a task is split
- Report sizing summary: "12 tasks: 3 small, 7 medium, 2 large, 0 XL"

**Files modified:**
- `dial-core/src/prd/wizard.rs` — update Phase 6 prompt, parse sizing results
- `dial-core/src/event.rs` — add TaskSplit event variant

**Testing:**
- Unit test: Phase 6 prompt includes sizing analysis instructions
- Unit test: parse split/rewrite/merge actions from AI response
- Integration test: oversized task gets split into multiple tasks with dependencies

---

## 5. Test Coverage Generation (Phase 7 Enhancement)

**Problem:** Phase 7 (Build & Test) suggests build/test commands but doesn't ensure the project has adequate test coverage planned. Feature tasks are created without corresponding test tasks.

**Solution:** Add test strategy generation to Phase 7 that creates test tasks paired with feature tasks and ensures the spec has testable acceptance criteria.

**Implementation:**
- In prd/wizard.rs, update the Phase 7 (BuildTestConfig) prompt builder to include test strategy:
  ```
  TEST STRATEGY:
  Review the task list and ensure adequate test coverage:

  1. For each feature task, determine if a separate test task is needed:
     - If the feature is complex enough to warrant dedicated tests: create a test task that depends on the feature task
     - If the feature is simple: note that tests should be included in the implementation task itself
     - Test task descriptions should be specific: "Write integration tests for POST /users endpoint: valid input returns 201, duplicate email returns 409, missing fields return 422"

  2. Suggest the test command and framework based on the technical stack:
     - Rust: cargo test
     - Node.js: npm test (jest/vitest/mocha)
     - Python: pytest
     - Go: go test ./...

  3. Suggest validation pipeline steps in order:
     - Lint (optional): catches style issues without blocking
     - Build (required): compilation must pass
     - Test (required): all tests must pass

  Output:
  - New test tasks (with descriptions and dependencies)
  - Recommended test_cmd
  - Recommended pipeline steps with sort_order, required flag, and timeout
  ```
- Parse the Phase 7 response to extract test tasks, test_cmd, and pipeline steps
- Create test tasks in the database with dependencies on their feature tasks
- Configure the validation pipeline with the suggested steps
- Report: "Added 5 test tasks, configured 3 pipeline steps"

**Files modified:**
- `dial-core/src/prd/wizard.rs` — update Phase 7 prompt, parse test strategy, create test tasks

**Testing:**
- Unit test: Phase 7 prompt includes test strategy instructions
- Unit test: parse test task extraction from AI response
- Integration test: feature tasks get paired test tasks with correct dependencies

---

## 6. Version

These changes constitute DIAL v4.1.0 — a minor version bump. No breaking changes, no schema migrations needed. The failed attempt diff uses the existing iteration notes field.

## 7. Testing Requirements

Minimum 15 new tests:
- 4 for failed attempt diff capture
- 3 for spec specificity enforcement
- 3 for task sizing analysis
- 3 for test coverage generation
- 2 integration tests for full wizard flow with enhanced phases
- All existing 308 tests must continue to pass
