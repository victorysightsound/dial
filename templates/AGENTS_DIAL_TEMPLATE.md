# Project: PROJECT_NAME

## DIAL-Enabled Project

This project uses [DIAL](https://github.com/victorysightsound/dial) for autonomous iterative development.

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
| `dial context` | Regenerate fresh context |
| `dial auto-run --cli claude` | Fully automated orchestration |
| `dial status` | Current state |
| `dial stats` | Statistics dashboard |

### Configuration

```bash
dial config set build_cmd "YOUR_BUILD_COMMAND"
dial config set test_cmd "YOUR_TEST_COMMAND"
```

### Specifications (Optional)

If your project has specs in `specs/*.md`, index them for richer context:

```bash
dial index
dial task add "Implement feature X" -p 2 --spec 1
```

Specs are optional. DIAL works with just tasks — specs add automatic context retrieval for related work.

### The DIAL Loop

1. `dial iterate` - Get task + context from database
2. Implement the task (one task only, no placeholders, search before creating)
3. `dial validate` - Run build/test, commit on success
4. On success: next task. On failure: retry (max 3 attempts).

### DIAL Signals (for automated mode)

When running under `dial auto-run`, output these signals:

```
DIAL_COMPLETE: <summary of what was done>
DIAL_BLOCKED: <reason the task can't be completed>
DIAL_LEARNING: <category>: <what was learned>
```

---

## Project-Specific Notes

<!-- Add project-specific instructions, architecture notes, conventions here -->
