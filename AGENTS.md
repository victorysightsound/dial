# Project: DIAL

## What is DIAL?

**DIAL** = **Deterministic Iterative Agent Loop**

A CLI tool and methodology for autonomous AI-assisted software development. DIAL gives AI coding agents persistent memory, failure pattern detection, and structured task execution.

- **Problem:** Markdown-based memory causes context bloat as iterations grow
- **Solution:** SQLite + FTS5 for selective recall, trust-based solution learning

### Core Components

| Component | Location |
|-----------|----------|
| DIAL CLI (Rust) | `./dial/` |
| Methodology guide | `./dial_guide.db` |
| AGENTS.md template | `./templates/AGENTS_DIAL_TEMPLATE.md` |
| Documentation | `./docs/` |

### Build Commands

```bash
cd dial && cargo build --release    # Build Rust binary
cd dial && cargo test               # Run tests
```

---

## Using DIAL in Other Projects

1. Copy `templates/AGENTS_DIAL_TEMPLATE.md` to your project as `AGENTS.md`
2. Initialize: `dial init --phase mvp`
3. Configure build/test commands
4. Put your PRD in `specs/`
5. Run `dial index`
6. Tell the AI: "Use DIAL to build this project from the PRD"

---

## The 10 Failure Modes

DIAL addresses these predictable agent failures:

1. Duplicate Implementation
2. Placeholder/Minimal Implementation
3. Context Window Exhaustion
4. Reasoning Loss Between Loops
5. Cascading Failures
6. Overnight Breakage
7. Validation Backpressure Chaos
8. Specification Drift
9. Test Amnesia
10. Learning Loss

Query `dial_guide.db` section 1.4 for full details and countermeasures.

---

## Quick Reference

```bash
dial status           # Current state
dial task list        # Show pending tasks
dial task next        # Show next task
dial iterate          # Start next task, get context
dial validate         # Run tests, commit on success
dial learn "text" -c category  # Record a learning
dial stats            # Statistics dashboard
dial context          # Fresh context regeneration
dial orchestrate      # Sub-agent prompt generation
dial auto-run         # Automated orchestration
```

### The DIAL Loop

1. `dial iterate` - Get task + context
2. Implement (one task only, no placeholders, search before creating)
3. `dial validate` - Test and commit
4. On success: next task. On failure: retry (max 3).

### Automated Orchestration

```bash
dial auto-run --cli claude --max 10
```

Spawns a fresh AI subprocess for each task, parses DIAL signals, runs validation, and loops.

### Configuration

```bash
dial config set build_cmd "your build command"
dial config set test_cmd "your test command"
```

---

## External-Facing Writing

- Keep README files, changelogs, commit messages, PR text, and code comments in normal developer voice.
- Do not describe implementation work in terms of model names, agent runs, or internal workflow prompts.
- DIAL may be documented freely as a product, methodology, or runtime feature because it is the actual subject of this repository.
