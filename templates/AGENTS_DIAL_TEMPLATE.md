# Project: PROJECT_NAME

## On Entry (MANDATORY)

```bash
session-context
```

---

## DIAL-Enabled Project

This project uses DIAL for autonomous iterative development.

### Get Full Instructions

DIAL guide database: `~/.dial/dial_guide.db`

```bash
# Quick reference (start here)
sqlite3 ~/.dial/dial_guide.db "SELECT content FROM sections WHERE section_id = '2.1';"

# Full AI workflow
sqlite3 ~/.dial/dial_guide.db "SELECT content FROM sections WHERE section_id LIKE '2.%' ORDER BY sort_order;"

# PRD format specification
sqlite3 ~/.dial/dial_guide.db "SELECT content FROM sections WHERE section_id = '2.4';"

# Task extraction guide
sqlite3 ~/.dial/dial_guide.db "SELECT content FROM sections WHERE section_id = '2.5';"

# Search for any topic
sqlite3 ~/.dial/dial_guide.db "SELECT s.section_id, s.title, s.content FROM sections s INNER JOIN sections_fts fts ON s.id = fts.rowid WHERE sections_fts MATCH 'your topic' LIMIT 5;"
```

### Quick Start

```bash
# Check status
dial status

# See next task
dial task next

# Start working on task
dial iterate

# After implementing, validate and commit
dial validate

# View progress
dial stats
```

### Key Commands

| Command | Purpose |
|---------|---------|
| `dial init --phase NAME` | Initialize DIAL for this project |
| `dial index` | Index specs/ into searchable database |
| `dial task add "desc" -p N` | Add task with priority (1=high, 10=low) |
| `dial task list` | Show pending tasks |
| `dial iterate` | Start next task, get context |
| `dial validate` | Run build/test, commit on success |
| `dial status` | Current state |
| `dial stats` | Statistics dashboard |

### Configuration

```bash
dial config set build_cmd "YOUR_BUILD_COMMAND"
dial config set test_cmd "YOUR_TEST_COMMAND"
```

---

## Project-Specific Notes

<!-- Add project-specific instructions here -->

---

## Memory Commands

```bash
memory-log decision "topic" "what was decided and why"
memory-log note "topic" "content"
task add "description" [priority]
```
