# The DIAL Methodology

DIAL (Deterministic Iterative Agent Loop) is both a tool and a methodology. This document explains the theory: why AI coding agents fail at iterative development and how DIAL's design prevents each failure mode.

## The Core Insight

AI coding assistants are powerful within a single task but break down during iterative multi-task development. The problem isn't intelligence - it's memory architecture. LLMs process conversations as a growing sequence of tokens. As development iterations accumulate, the conversation grows until:

1. Important early decisions scroll out of the context window
2. The AI can't distinguish its own previous output from the spec
3. Error patterns repeat because there's no structured recall
4. Each new iteration starts with less useful context than the last

DIAL externalizes the AI's memory to a structured database, giving it selective recall instead of total recall. Instead of stuffing the entire history into context, DIAL queries for exactly what's relevant to the current task.

## The 10 Failure Modes

DIAL was designed by cataloging the predictable ways AI agents fail during iterative development. Every feature in DIAL maps to preventing one or more of these failures.

### 1. Duplicate Implementation

**The failure:** The AI creates a new file or function that already exists in the codebase.

**Cause:** After many iterations, the agent loses track of what it already built. Earlier file-creation actions have scrolled out of context.

**DIAL countermeasure:** Behavioral sign "SEARCH BEFORE CREATE" is included in every task context. Learnings from previous tasks remind the agent what exists. Spec sections link tasks to requirements, preventing re-implementation.

### 2. Placeholder Code

**The failure:** The AI writes stubs, TODO comments, or partial implementations instead of complete code.

**Cause:** Pressure to show progress. The agent "implements" a function by writing its signature and a `// TODO: implement` comment.

**DIAL countermeasure:** Behavioral sign "NO PLACEHOLDERS" enforces complete implementations. Validation runs actual build/test commands - placeholders fail tests. Auto-run mode requires a `DIAL_COMPLETE` signal, and validation must pass before moving on.

### 3. Context Window Exhaustion

**The failure:** The conversation grows so large that the AI's reasoning quality degrades.

**Cause:** Iterative development naturally accumulates history. Build outputs, error messages, code snippets, and conversation all compete for limited context.

**DIAL countermeasure:** The database replaces conversation-as-memory. Each task gets fresh context assembled from the DB. Auto-run spawns a fresh AI subprocess per task with zero conversation history. Only relevant specs, solutions, and learnings are included.

### 4. Reasoning Loss Between Loops

**The failure:** The AI forgets a decision made 10 iterations ago and makes an incompatible choice.

**Cause:** Decisions made in conversation are ephemeral. They exist only as buried text in a growing transcript.

**DIAL countermeasure:** Learnings persist decisions in the database. Solutions record what works and what doesn't. Each new iteration includes relevant learnings sorted by reference frequency.

### 5. Cascading Failures

**The failure:** One error leads to a "fix" that introduces two new errors. Those fixes introduce more. The codebase spirals.

**Cause:** Without structured error tracking, the AI reacts to each error independently. It doesn't see the pattern of compounding damage.

**DIAL countermeasure:** Failure pattern detection auto-categorizes errors (21 patterns across 5 categories). The max-attempts limit (3) prevents infinite retry spirals. On max failures, DIAL blocks the task and reverts to the last successful commit.

### 6. Overnight Breakage

**The failure:** Changes to one component break tests in an unrelated component.

**Cause:** The AI focuses on the current task and doesn't run the full test suite. Or it runs tests but doesn't notice unrelated failures.

**DIAL countermeasure:** Validation runs the full `test_cmd` (which should include the complete test suite). Build must also pass. Both must succeed before DIAL commits and moves on.

### 7. Validation Backpressure Chaos

**The failure:** Strict validation (full builds, all tests) creates so much pressure that the AI takes shortcuts: disabling tests, commenting out checks, or weakening assertions.

**Cause:** When the AI is measured by "did validation pass?", it optimizes for that metric by any means necessary.

**DIAL countermeasure:** Solutions are trust-scored. A hack that passes once but fails later loses confidence (-0.20). Legitimate fixes that work repeatedly gain confidence (+0.15). Over time, the solution database converges on real solutions.

### 8. Specification Drift

**The failure:** The implementation diverges from the spec. Features get built differently than specified, or entire requirements are missed.

**Cause:** The spec is just a document. As iterations accumulate, the AI stops referencing it because the conversation has moved past it.

**DIAL countermeasure:** Specs are indexed with FTS5 search. Tasks can link to spec sections. Context gathering automatically surfaces relevant spec sections based on the current task description.

### 9. Test Amnesia

**The failure:** A test that passed in iteration 5 starts failing in iteration 15, and the AI can't remember how it made it pass before.

**Cause:** The fix from iteration 5 exists only in conversation history, which may have been truncated or lost.

**DIAL countermeasure:** Successful solutions are stored with code examples and confidence scores. When the same failure pattern recurs, DIAL surfaces the trusted solutions from previous iterations.

### 10. Learning Loss

**The failure:** A hard-won insight from Monday's session is completely forgotten by Tuesday.

**Cause:** Each conversation session starts fresh. Cross-session memory depends entirely on what's in the codebase or documentation.

**DIAL countermeasure:** The learning system explicitly captures insights. Learnings persist across sessions in the database. They're included in task context, sorted by how frequently they've been useful.

## The DIAL Loop

The core loop is simple:

```
1. Pick the next task (by priority)
2. Gather relevant context from the database
3. Implement the task
4. Validate (build + test)
5. On success: commit, move to next task
6. On failure: record pattern, retry (max 3)
```

### Behavioral Signs

Every task context includes six behavioral guardrails:

1. **ONE TASK ONLY** - Complete exactly this task. No scope creep.
2. **SEARCH BEFORE CREATE** - Always search for existing files/functions before creating new ones.
3. **NO PLACEHOLDERS** - Every implementation must be complete. No TODO, FIXME, or stub code.
4. **VALIDATE BEFORE DONE** - Run `dial validate` after implementing. Don't mark complete without testing.
5. **RECORD LEARNINGS** - After success, capture what you learned.
6. **FAIL FAST** - If blocked or confused, stop and ask rather than guessing.

These are included verbatim in every context generation because they address the most common ways agents go off-track.

### Context Assembly

When DIAL starts a task, it assembles context by querying the database for:

1. **Behavioral signs** (always first - most important for preventing drift)
2. **Linked spec section** (if the task has a `--spec` link)
3. **Related spec sections** (FTS search using the task description)
4. **Failed attempt diffs** (on retry attempts, the previous failed diff and error at priority 12 — shows what was tried so the agent doesn't repeat the same approach)
5. **Trusted solutions** (confidence >= 0.6, sorted by occurrence count)
6. **Recent unresolved failures** (so the agent knows what to avoid)
7. **Project learnings** (sorted by reference frequency, top 10)

This gives the agent exactly what it needs without flooding it with irrelevant history.

### The Trust System

Solutions follow a simple trust algorithm:

- **New solution:** starts at 0.3 confidence
- **Applied successfully:** +0.15 confidence
- **Applied and failed:** -0.20 confidence
- **Trusted threshold:** 0.6
- **Maximum:** 1.0

The asymmetric scoring (failing costs more than succeeding) means solutions must prove themselves multiple times before being trusted. A solution needs 2 consecutive successes to cross the 0.6 threshold. A single failure drops it back below.

This naturally filters out hacks and workarounds while preserving legitimate fixes.

### Max Attempts and Reversion

Each task gets 3 attempts maximum. This prevents the cascading failure spiral where an AI keeps trying increasingly desperate fixes.

After 3 failures:
1. The task is blocked with the reason "Failed 3 times"
2. If in a git repo, DIAL reverts to the last successful commit
3. The task stays in the queue as blocked for human review

This ensures the codebase never degrades past the last known-good state.

## When to Use DIAL

### Where DIAL Excels

- **Greenfield projects** with a clear spec
- **Feature implementation** where tasks are well-defined
- **Code generation** from detailed requirements
- **Multi-file refactoring** that needs to stay coordinated

### Where DIAL Struggles

- **Exploratory coding** where the design isn't known upfront
- **Debugging specific issues** (use your AI tool directly)
- **Large existing codebases** where the AI can't grasp the architecture from specs alone
- **UI/visual work** where validation requires human judgment

### The 90% Expectation

Set expectations appropriately: DIAL will get you 80-90% of the way through a project autonomously. The remaining 10-20% requires human judgment for nuanced decisions, edge cases, and integration concerns. This is by design - DIAL makes the AI maximize the boring, repetitive, well-specified work so you can focus on the interesting problems.
