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
| DIAL CLI | `~/projects/dial/dial.py` (symlinked to `~/bin/dial`) |
| Methodology guide | `./dial_guide.db` |
| Upgrade plan | `./DIAL_UPGRADE_PLAN.md` |
| Memory proposal | `./DIAL_Memory_System_Proposal.md` |
| AGENTS.md template | `./templates/AGENTS_DIAL_TEMPLATE.md` |

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
