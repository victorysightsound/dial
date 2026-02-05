# Project: DIAL

## On Entry (MANDATORY)

```bash
session-context
```

---

## Project Documentation

**DIAL Guide Database:** `./dial_guide.db`

This database contains the complete DIAL methodology and AI agent instructions.

### Query Instructions

```sql
-- List all sections
SELECT section_id, title FROM sections ORDER BY sort_order;

-- Read AI agent workflow
SELECT content FROM sections WHERE section_id LIKE '2.%' ORDER BY sort_order;

-- Search for topic
SELECT heading_path, content FROM sections_fts WHERE sections_fts MATCH 'your topic';

-- Get PRD format specification
SELECT content FROM sections WHERE section_id = '2.4';

-- Get task extraction guide
SELECT content FROM sections WHERE section_id = '2.5';
```

---

## What is DIAL?

**DIAL** = **Deterministic Iterative Agent Loop**

A methodology and toolset for autonomous AI development:

- **Problem:** Markdown-based memory causes context bloat as iterations grow
- **Solution:** SQLite + FTS5 for selective recall, trust-based solution learning

### Core Components

| Component | Location |
|-----------|----------|
| DIAL CLI (Rust) | `./dial/` → symlinked to `~/bin/dial` |
| Legacy CLI (Python) | `./dial_legacy.py` |
| Methodology guide | `./dial_guide.db` |
| Implementation docs | `./specs/DIAL_RUST_PRD.md` |
| AGENTS.md template | `./templates/AGENTS_DIAL_TEMPLATE.md` |

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

## Memory Commands

```bash
memory-log decision "topic" "what was decided and why"
memory-log note "topic" "content"
memory-log blocker "topic" "what is blocking"
task add "description" [priority]
```

---

## DIAL — Autonomous Development Loop

This project uses **DIAL** (Deterministic Iterative Agent Loop) for autonomous development.

### Get Full Instructions

```bash
sqlite3 ~/projects/dial/dial_guide.db "SELECT content FROM sections WHERE section_id LIKE '2.%' ORDER BY sort_order;"
```

### Quick Reference

```bash
dial status           # Current state
dial task list        # Show pending tasks
dial task next        # Show next task
dial iterate          # Start next task, get context
dial validate         # Run tests, commit on success
dial learn "text" -c category  # Record a learning
dial stats            # Statistics dashboard
dial context          # Fresh context (Ralph-style)
dial orchestrate      # Sub-agent prompt (Ralph-style)
dial auto-run         # Automated orchestration (Ralph-style)
```

### The DIAL Loop

1. `dial iterate` → Get task + context
2. Implement (one task only, no placeholders, search before creating)
3. `dial validate` → Test and commit
4. On success: next task. On failure: retry (max 3).

### Ralph-Style Context Rot Prevention (v2.1+)

DIAL includes features from the Ralph Loop methodology to combat context rot:

1. **Signs (Behavioral Guardrails):** Context now includes critical rules:
   - ONE TASK ONLY - No scope creep
   - SEARCH BEFORE CREATE - Don't duplicate
   - NO PLACEHOLDERS - Complete implementations only
   - VALIDATE BEFORE DONE - Always test
   - RECORD LEARNINGS - Capture insights
   - FAIL FAST - Ask don't guess

2. **Fresh Context:** Run `dial context` anytime to regenerate clean context

3. **Manual Orchestrator Mode:** Run `dial orchestrate` to get a prompt for spawning fresh sub-agents:
   ```bash
   # Claude Code
   claude -p "$(cat .dial/subagent_prompt.md)"

   # Codex CLI
   codex --task "$(cat .dial/subagent_prompt.md)"
   ```

4. **Automated Orchestration (v2.2):** Run `dial auto-run` for fully automated task execution:
   ```bash
   dial auto-run --cli claude --max 10
   ```
   This spawns a fresh AI subprocess for each task, parses DIAL signals, runs validation, and loops.

5. **Learning Prompts:** After successful validation, DIAL reminds you to capture learnings

### Configuration

```bash
dial config set build_cmd "your build command"
dial config set test_cmd "your test command"
```
