# AI Integration Guide

DIAL works with any AI coding tool. This guide covers setup and usage patterns for each supported tool.

## Overview

DIAL interacts with AI tools in three ways:

| Mode | How it works | Control level |
|------|-------------|---------------|
| **Manual** | You run the AI yourself, use `dial iterate` / `dial validate` for tracking | Full control |
| **Orchestrated** | DIAL generates a prompt, you paste it into the AI | Semi-automated |
| **Auto-run** | DIAL spawns the AI as a subprocess per task | Fully automated |

## DIAL Signals

When DIAL generates prompts for AI tools (via `dial orchestrate` or `dial auto-run`), it instructs the AI to output signals when done:

```
DIAL_COMPLETE: <summary of what was done>
DIAL_BLOCKED: <reason the task can't be completed>
DIAL_LEARNING: <category>: <what was learned>
```

DIAL parses these from the AI's output to determine next steps. The parser handles variations in formatting (markdown bold, backticks, spaces vs underscores, case differences).

## Claude Code

### Setup

Install [Claude Code](https://claude.ai/download) and ensure the `claude` CLI is in your PATH.

### Manual Mode

```bash
# Start a task
dial iterate

# Claude Code sees .dial/current_context.md in the project
# Work with Claude normally to implement the task

# Validate when done
dial validate
```

### Auto-Run Mode

```bash
dial auto-run --cli claude --max 10
```

DIAL uses `claude -p "$(cat .dial/subagent_prompt.md)"` to spawn non-interactive Claude sessions. Each task gets a fresh subprocess with clean context.

### Configuration

```bash
dial config set ai_cli claude
dial config set subagent_timeout 1800  # 30 min per task
```

### Tips for Claude Code

- Claude Code can read `.dial/current_context.md` during normal interactive sessions
- In auto-run, each task gets a fresh session with no conversation history
- For large tasks, increase the timeout: `dial config set subagent_timeout 3600`

## Codex CLI

### Setup

Install [Codex CLI](https://github.com/openai/codex) and ensure the `codex` command is available.

### Auto-Run Mode

```bash
dial auto-run --cli codex --max 10
```

DIAL uses `cat .dial/subagent_prompt.md | codex exec --skip-git-repo-check` to run tasks.

### Manual Orchestration

```bash
dial orchestrate
# Copy the output prompt to Codex
codex exec "$(cat .dial/subagent_prompt.md)"
```

## Gemini CLI

### Setup

Install [Gemini CLI](https://github.com/google-gemini/gemini-cli) and ensure the `gemini` command is available.

### Auto-Run Mode

```bash
dial auto-run --cli gemini --max 10
```

DIAL uses `cat .dial/subagent_prompt.md | gemini -p -` to run tasks.

## Other AI Tools

DIAL works with any AI tool that can accept a text prompt. The workflow:

1. Run `dial orchestrate` to generate a self-contained prompt
2. The prompt is saved to `.dial/subagent_prompt.md`
3. Feed the prompt to your AI tool however it accepts input
4. After the AI implements the task, run `dial validate`

### Example with ChatGPT / API

```bash
dial orchestrate
# Copy the content of .dial/subagent_prompt.md
# Paste into ChatGPT, Cursor, Windsurf, or your API call
# Apply the AI's output to your codebase
dial validate
```

### Example with a Custom Script

```bash
#!/bin/bash
# custom-ai-runner.sh

PROMPT=$(cat .dial/subagent_prompt.md)

# Call your AI API
curl -s https://api.example.com/v1/completions \
  -H "Authorization: Bearer $API_KEY" \
  -d "{\"prompt\": \"$PROMPT\"}" \
  | jq -r '.completion' \
  > /tmp/ai-output.txt

# Apply changes...
# Then validate
dial validate
```

## Writing Effective Specs for AI

The quality of DIAL's output depends heavily on your specification. Tips:

### Be Specific About Behavior

```markdown
# Good
## User Login
Accept email and password. Hash password with bcrypt (cost 12).
Return JWT with 24-hour expiry. Store refresh token in httpOnly cookie.
On invalid credentials, return 401 with message "Invalid email or password".

# Bad
## User Login
Users should be able to log in.
```

### Number Your Sections

DIAL indexes spec sections by their markdown headers. Numbered sections (`## 1.`, `## 2.1`) make it easy to link tasks to specs.

### Include Technical Constraints

```markdown
## 3. Data Storage
- SQLite 3.40+ with WAL mode
- Tables: users, sessions, tasks
- All timestamps stored as ISO 8601 UTC
- Indexes on users.email (unique) and tasks.user_id
```

### Keep Sections Focused

Each section should describe one feature or component. DIAL's FTS search works best when sections are cohesive - a search for "authentication" should return the auth section, not a section that mentions auth once in passing.

## Troubleshooting

### "No completion signal received"

The AI finished but didn't output `DIAL_COMPLETE:`. Possible causes:
- The AI hit an error and stopped
- The AI completed but forgot to output the signal
- The output was truncated

Fix: Check the output (printed in real-time during auto-run). If the task was actually completed, run `dial validate` manually.

### "Subagent timed out"

The AI took longer than the configured timeout.

Fix: Increase the timeout:
```bash
dial config set subagent_timeout 3600  # 1 hour
```

### AI Makes Same Mistake Repeatedly

DIAL records failure patterns and surfaces solutions, but the AI needs to read them. In auto-run mode, solutions are included in the prompt automatically. In manual mode, check:

```bash
dial failures          # See what's failing
dial solutions -t      # See trusted solutions
dial context           # Regenerate context with latest info
```

### Tasks Keep Getting Blocked

After 3 failures, tasks are blocked. To retry:

```bash
# Check why it failed
dial failures

# Record what you learned
dial learn "The auth endpoint needs CORS headers" -c gotcha

# Unblock by adding a new task for the same work
dial task add "Implement auth with CORS headers" -p 1
```
