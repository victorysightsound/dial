# Project Context & Rules
This file is the single source of truth for Gemini, Claude, and Codex agents.


## Project Tracking

This project uses `proj` for session and decision tracking. Follow these instructions to maintain project continuity.

### Session Management

**At session start:**
```bash
proj status
```
This shows current project state and automatically starts a session if needed. Review the output to understand:
- Where the last session left off
- Current active tasks
- Recent decisions
- Any open blockers or questions

**During the session:**
- Log significant decisions: `proj log decision "topic" "what was decided" "why"`
- Update task status: `proj task update <id> --status in_progress`
- Note blockers: `proj log blocker "description"`
- Add context notes: `proj log note "category" "title" "content"`

**At session end:**
```bash
proj session end "Brief summary of what was accomplished"
```

### Quick Reference

| Command | Purpose |
|---------|---------|
| `proj status` | Current state + auto-start session |
| `proj resume` | Detailed "where we left off" context |
| `proj context <topic>` | Query decisions/notes about a topic |
| `proj tasks` | List current tasks |
| `proj log decision "topic" "decision" "rationale"` | Record a decision |
| `proj session end "summary"` | Close session with summary |

### Database Queries (for AI agents)

For direct database access when more efficient:

```sql
-- Get last session summary
SELECT summary FROM sessions WHERE status = 'completed' ORDER BY ended_at DESC LIMIT 1;

-- Get active tasks
SELECT task_id, description, status, priority FROM tasks WHERE status NOT IN ('completed', 'cancelled') ORDER BY priority, created_at;

-- Get recent decisions on a topic
SELECT decision, rationale, created_at FROM decisions WHERE topic LIKE '%auth%' AND status = 'active';

-- Search all tracked content
SELECT * FROM tracking_fts WHERE tracking_fts MATCH 'search term';
```

Tracking database: `.tracking/tracking.db`
Project database: `./{project_name}_{type}.db` (in project root)

### Principles

1. **Start with `proj status`** - never guess project state
2. **Log decisions when made** - not later when you might forget the rationale
3. **Keep task status current** - update as you work, not in batches
4. **End sessions with summaries** - future you (or another agent) will thank you
5. **Query before re-reading** - a SQL query uses fewer tokens than re-reading files
