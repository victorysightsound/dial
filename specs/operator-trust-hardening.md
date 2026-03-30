# DIAL Operator Trust Hardening Backlog

Version: 4.2.6
Status: Backlog
Phase: operator-trust-hardening

---

## 1. Overview

DIAL's runtime architecture is already strong:

- host-controlled task selection
- explicit validation before commit
- rollback/checkpoint support
- structured retry context
- persistent state for tasks, failures, learnings, and solutions

The next step is not more autonomy. The next step is stronger operator trust.

This backlog focuses on improvements that make DIAL easier to inspect, easier to audit, and safer to use for UI-heavy work without weakening the current host-controlled loop.

---

## 2. Goals

1. Make DIAL's progress more human-readable outside the database.
2. Make reusable project patterns more visible in task context.
3. Add stronger completion requirements for UI-facing work.
4. Make task completion and acceptance criteria easier for humans to audit.

### Non-Goals

- Replace SQLite state with flat files
- Move validation/commit control back into the AI prompt
- Rebuild DIAL around a shell-script-driven loop

---

## 3. Why This Matters

There are four trust gaps worth closing:

1. Iteration history is strong in the database but not visible enough in a readable project log.
2. Reusable project patterns exist in learnings and solutions but are not prominent enough in live context.
3. UI tasks can pass build and test without proving that the visible behavior is correct.
4. Task state is richer than a simple pass/fail bit, but it is not as easy to audit at a glance as it should be.

These additions would improve clarity, reviewability, and operator confidence.

---

## 4. Workstreams

### A. Human-Readable Progress Log

**Problem:** DIAL stores the right information, but much of it lives in SQLite and event output instead of a durable human-readable timeline.

**Proposal:**
- Add `.dial/progress.md` as an append-only log written by the host process.
- Append an entry after each iteration attempt, whether it succeeds, fails, or blocks.
- Include:
  - timestamp
  - task id and description
  - iteration number
  - result: completed / failed / blocked / no-signal
  - files changed summary when available
  - commit hash on success
  - short failure summary on failure
  - reusable learning summary when present

**Why it strengthens DIAL:**
- makes loop history inspectable without opening the database
- gives users a high-trust audit trail
- improves recovery when debugging a bad run

**Likely files:**
- `dial-core/src/iteration/orchestrator.rs`
- `dial-core/src/iteration/mod.rs`
- `dial-core/src/output.rs` or a new progress-log helper module

---

### B. Codebase Patterns Digest

**Problem:** DIAL stores learnings and trusted solutions, but reusable patterns are not surfaced as a first-class, distilled layer.

**Proposal:**
- Add `.dial/patterns.md` or `.dial/patterns.json`.
- Populate it from curated learnings, trusted solutions, and repeated high-confidence patterns.
- Surface the top patterns near the top of every auto-run context.
- Distinguish:
  - reusable pattern
  - gotcha
  - environment requirement
  - testing convention

**Why it strengthens DIAL:**
- reduces repeated mistakes across iterations
- makes stable project conventions easier for both humans and agents to see
- gives DIAL a clearer top-level convention layer without giving up the DB model

**Likely files:**
- `dial-core/src/iteration/context.rs`
- `dial-core/src/learning.rs`
- new pattern digest module in `dial-core/src/`

---

### C. Browser Verification Gate For UI Tasks

**Problem:** DIAL's build/test validation is strong for code correctness but weaker for UI tasks where automated checks may miss regressions or incomplete behavior.

**Proposal:**
- Add task metadata such as `requires_browser_verification`.
- Allow Phase 6 or Phase 7 of the wizard to infer this for UI tasks.
- Require a browser verification artifact before a UI task can be marked complete in auto-run or manual validate flows.
- Store a small verification record in `.dial/`:
  - verified route/page
  - timestamp
  - optional screenshot path
  - optional notes

**Why it strengthens DIAL:**
- closes a real gap in frontend confidence
- prevents "tests pass but UI is wrong" completions
- keeps completion criteria aligned with actual user-visible behavior

**Likely files:**
- `dial-core/src/prd/wizard.rs`
- `dial-core/src/iteration/mod.rs`
- `dial-core/src/iteration/orchestrator.rs`
- `docs/ai-integration.md`
- `docs/cli-reference.md`

---

### D. Acceptance Criteria As First-Class Validation Inputs

**Problem:** DIAL validates with configured commands, but task completion still depends heavily on build/test outcomes rather than an explicit acceptance-criteria ledger.

**Proposal:**
- Extend tasks to store structured acceptance criteria where available.
- Let the wizard generate and preserve clearer per-task acceptance criteria.
- Show those criteria in iterate/auto-run context and in `dial task show`.
- Add a completion summary that records which criteria were satisfied automatically and which still need human review.

**Why it strengthens DIAL:**
- improves explainability of "why this task is done"
- reduces the chance that passing tests are mistaken for complete feature delivery
- makes DIAL's task model easier to audit than a simple pass/fail bit

**Likely files:**
- `dial-core/src/prd/wizard.rs`
- `dial-core/src/task/`
- `dial-core/src/iteration/context.rs`
- `dial-cli/src/main.rs`

---

### E. Human-Facing Task Ledger Export

**Problem:** DIAL has rich task state, but that richness is not as easy to scan at a glance as it should be.

**Proposal:**
- Add `dial progress` or `dial status --human`.
- Optionally export a readable task ledger to `.dial/task-ledger.md`.
- Include:
  - pending / in progress / completed / blocked
  - dependency summary
  - last attempt result
  - acceptance review state

**Why it strengthens DIAL:**
- gives users a simple trust surface
- makes the system easier to review between sessions
- reduces the feeling that DIAL's real state is hidden in SQLite

**Likely files:**
- `dial-cli/src/main.rs`
- `dial-core/src/task/`
- `dial-core/src/engine.rs`

---

## 5. Recommended Implementation Order

### Immediate

1. Human-readable progress log
2. Codebase patterns digest
3. Human-facing task ledger export

These add transparency quickly without changing completion semantics.

### Next

4. Acceptance criteria as first-class task data
5. Browser verification gate for UI tasks

These are higher-value but require more schema, UX, and validation design work.

---

## 6. Suggested Backlog Items

1. Add `.dial/progress.md` writer and append entries after every iteration outcome.
2. Add distilled pattern generation from learnings and trusted solutions.
3. Surface top reusable patterns at the top of generated task context.
4. Add `dial progress` for a readable project timeline and task ledger view.
5. Extend task model to store acceptance criteria explicitly.
6. Show acceptance criteria in `dial iterate`, `dial task show`, and auto-run prompts.
7. Add task metadata for UI/browser verification requirements.
8. Design a verification artifact format for browser-checked tasks.
9. Prevent completion of UI-tagged tasks until browser verification is recorded.

---

## 7. Bottom Line

DIAL should keep its current architecture. The loop itself is not the problem.

The improvements worth adopting are about operator trust and workflow clarity:

- more visible progress
- more visible reusable patterns
- more explicit acceptance tracking
- stronger browser verification for UI work

That combination would make DIAL easier to trust while it runs, easier to review after it runs, and harder to misunderstand during autonomous execution.
