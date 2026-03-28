# AI Integration Guide

DIAL works with any AI coding tool. This guide covers setup and usage patterns for each supported tool.

## Overview

DIAL interacts with AI tools in three ways:

| Mode | How it works | Control level |
|------|-------------|---------------|
| **Manual** | You run the AI yourself, use `dial iterate` / `dial validate` for tracking | Full control |
| **Orchestrated** | DIAL generates a prompt, you paste it into the AI | Semi-automated |
| **Auto-run** | DIAL spawns the AI as a subprocess per task | Fully automated |

## Wizard Backends

The project wizard uses its own backend selection path for `dial new` and `dial spec wizard`.

- Supported wizard backends: `codex`, `claude`, `copilot`, `gemini`, `openai-compatible`
- Pass `--wizard-backend` to choose explicitly
- Set `wizard_backend` in project config to make the choice sticky
- If DIAL can detect the active session backend, it uses that automatically
- If multiple supported CLIs are installed and no clear hint exists, DIAL stops and asks you to choose instead of guessing

## DIAL Signals

When DIAL generates prompts for AI tools (via `dial orchestrate` or `dial auto-run`), it instructs the AI to signal completion via a JSON file.

### Preferred: Structured Signal File

The AI writes `.dial/signal.json` before exiting:

```json
{
  "signals": [
    {"type": "complete", "summary": "Implemented user login with bcrypt hashing"},
    {"type": "learning", "category": "build", "description": "bcrypt requires Node 18+"}
  ],
  "timestamp": "2026-03-12T15:30:00Z"
}
```

Signal types: `complete` (with summary), `blocked` (with reason), `learning` (with category and description).

DIAL reads the file after the subprocess exits, then deletes it to prevent stale signals.

### Fallback: Regex Parsing

If `.dial/signal.json` doesn't exist, DIAL falls back to parsing stdout for signal lines:

```
DIAL_COMPLETE: <summary of what was done>
DIAL_BLOCKED: <reason the task can't be completed>
DIAL_LEARNING: <category>: <what was learned>
```

The parser handles formatting variations (markdown bold, backticks, spaces vs underscores, case differences) and filters out template placeholders and instruction lines.

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

For wizard usage, DIAL runs Codex in a lower-friction noninteractive mode with reduced reasoning/verbosity and web search disabled so short structured prompts stay responsive.

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

## GitHub Copilot CLI

### Setup

Install GitHub Copilot CLI and ensure the `copilot` command is available.

### Auto-Run Mode

```bash
dial auto-run --cli copilot --max 10
```

### Wizard Usage

```bash
dial new --template mvp --wizard-backend copilot
```

DIAL runs Copilot in noninteractive silent mode for both auto-run and wizard phases. The current Windows path has been validated in native CLI runs, not just through WSL.

## GitHub Copilot in VS Code

If you prefer Copilot inside VS Code instead of the standalone CLI, use DIAL in manual or orchestrated mode alongside Copilot Chat in VS Code. This works on all platforms including Windows.

### Setup

1. Install DIAL ([installation instructions](../README.md#install))
2. Open your project in VS Code with the [GitHub Copilot](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot) extension installed
3. Open an integrated terminal in VS Code (`` Ctrl+` `` or **Terminal > New Terminal**)

All `dial` commands run in the integrated terminal. Copilot Chat handles the implementation.

### Workflow: Manual Mode

The core loop — you use Copilot Chat to implement each task, then validate with DIAL in the terminal:

```
# 1. Start the next task
dial iterate
```

This prints the task description and relevant context (linked specs, trusted solutions, learnings from previous tasks). It also writes the full context to `.dial/current_context.md`.

```
# 2. Open the context file so Copilot can reference it
# In VS Code, open .dial/current_context.md (or just read the terminal output)
```

Now work with Copilot Chat to implement the task. You can:
- Paste the task description into Copilot Chat and ask it to implement
- Use `@workspace` to give Copilot project-wide context
- Reference `.dial/current_context.md` directly: "Read .dial/current_context.md and implement the task described there"

```
# 3. When implementation is done, validate
dial validate
```

If build and tests pass, DIAL auto-commits and moves to the next task. If they fail, DIAL records the failure pattern and resets the task for retry — the next `dial iterate` will include the failure context so you can adjust your approach.

### Workflow: Orchestrated Mode

For a richer prompt that includes all context, solutions, and behavioral guardrails:

```
# Generate a self-contained prompt
dial orchestrate
```

This writes a detailed prompt to `.dial/subagent_prompt.md`. Open that file and paste its contents into Copilot Chat. The prompt includes everything Copilot needs: the task, relevant specs, known solutions, and instructions.

After Copilot implements the changes:

```
dial validate
```

### Workflow: Copilot Edits (Agent Mode)

If you're using Copilot Edits (multi-file editing), the orchestrated prompt works well:

1. Run `dial orchestrate` in the terminal
2. Open `.dial/subagent_prompt.md`
3. Select all content and paste it into the Copilot Edits panel
4. Copilot applies changes across multiple files
5. Review the changes, then run `dial validate` in the terminal

### Windows-Specific Notes

DIAL runs natively on Windows — the binary is a self-contained `.exe` with no dependencies.

**PowerShell:**
```powershell
# All commands work the same in PowerShell
dial init --phase mvp
dial config set build_cmd "cargo build"
dial config set test_cmd "cargo test"
dial task add "Set up project structure" -p 1
dial iterate
dial validate
```

**Command Prompt (cmd.exe):**
```cmd
dial iterate
dial validate
```

**Windows Terminal:** Works with PowerShell, cmd.exe, or WSL. If your build tools are in WSL but you want to use VS Code on Windows, run DIAL inside WSL and open the folder in VS Code with the Remote - WSL extension.

**Path setup:** After downloading the binary from the [releases page](https://github.com/victorysightsound/dial/releases/latest), add the directory containing `dial.exe` to your system PATH, or place it in a directory already in your PATH (e.g., `C:\Users\<you>\.local\bin\`).

### Tips for Copilot

- **Give Copilot the context file.** The most effective approach is to reference `.dial/current_context.md` directly in your Copilot Chat prompt. It contains the task, specs, and learnings that DIAL assembled.
- **One task at a time.** Don't ask Copilot to implement multiple DIAL tasks in one conversation. Complete one, validate, then move to the next.
- **Record what you learn.** After completing a tricky task, capture the insight so future tasks benefit:
  ```
  dial learn "Windows paths need forward slashes in the config file" -c gotcha
  ```
- **Use `dial context` to refresh.** If you've been working on a task for a while and want updated context (e.g., after recording a learning or fixing a related issue), regenerate it:
  ```
  dial context
  ```
  Then re-read `.dial/current_context.md` in Copilot Chat.

## Cursor / Windsurf / Other AI Editors

AI-powered editors like Cursor and Windsurf work the same way as the Copilot workflow above. Use DIAL in the integrated terminal, and paste the orchestrated prompt into the editor's AI chat:

1. Run `dial orchestrate` in the terminal
2. Paste `.dial/subagent_prompt.md` contents into the AI chat panel
3. Let the editor implement the changes
4. Run `dial validate` in the terminal

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

## Iteration Modes

Control how `auto-run` pauses for review:

```bash
# Run everything without stopping (default)
dial config set iteration_mode autonomous

# Pause for review after every 5 completed tasks
dial config set iteration_mode review_every:5

# Pause after every single task
dial config set iteration_mode review_each
```

When paused, the iteration enters `awaiting_approval` status. Resume with `dial approve` or stop with `dial reject "reason"`.

The `dial new` wizard (phase 8) helps you choose the right mode based on project complexity.

## Auto-Run Best Practices

### Task Sizing

Each task runs in a single AI subprocess with a timeout. Tasks that are too large risk timeout, incomplete output, or context overload. Tasks that are too small create unnecessary overhead.

| Task size | Example | Guidance |
|-----------|---------|----------|
| Too large | "Build the entire authentication system" | Break into 3-5 tasks |
| Right size | "Implement password hashing with bcrypt" | Single focused feature |
| Too small | "Add a comment to line 42" | Combine with related work |

Rule of thumb: a task should be completable in 10-15 minutes of focused AI work.

### Timeout Configuration

Three timeouts control auto-run behavior:

| Config key | Default | What it controls |
|------------|---------|------------------|
| `subagent_timeout` | 1800s (30 min) | How long the AI subprocess gets to implement the task |
| `build_timeout` | 600s (10 min) | How long the build command gets during validation |
| `test_timeout` | 600s (10 min) | How long the test command gets during validation |

Total wall time per task can be up to `subagent_timeout + build_timeout + test_timeout`.

```bash
# For simple tasks (small scripts, config changes)
dial config set subagent_timeout 900    # 15 min

# For complex tasks (large features, refactoring)
dial config set subagent_timeout 3600   # 1 hour
```

### What Happens Without a Completion Signal

When the AI subprocess finishes without writing `.dial/signal.json` or outputting `DIAL_COMPLETE:`:

1. DIAL treats it as a failed attempt
2. The task resets to pending
3. One of the 3 max attempts is consumed
4. On the next attempt, the AI gets a fresh subprocess with updated failure context

Common causes:
- The task was too large and the AI lost focus
- The AI hit a permission prompt or error
- Output was truncated by the timeout
- The AI completed the work but forgot to output the signal

If the work was actually done, run `dial validate` manually to verify and commit.

### Stopping Auto-Run

```bash
# Graceful stop (finishes current task first)
dial stop

# Or create the stop file directly
touch .dial/stop
```

## Writing Effective Specs for AI

The quality of DIAL's output depends heavily on your specification. Specs are optional — you can use DIAL with just tasks — but they unlock context-aware task linking and richer AI prompts.

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

The AI finished but didn't write `.dial/signal.json` or output `DIAL_COMPLETE:`. Possible causes:
- The task is too large — break it into smaller pieces
- The AI hit a tool permission prompt (Claude Code) that blocked non-interactive mode
- The AI completed but forgot to output the signal
- Output was truncated by the subagent timeout

Fix: Check the streamed output (printed in real-time during auto-run). If the work was done, run `dial validate` manually. If the task is too large, cancel it and break it into smaller tasks.

### "Subagent timed out"

The AI took longer than the configured timeout.

Fix: Either increase the timeout or break the task into smaller pieces:
```bash
# Increase timeout
dial config set subagent_timeout 3600  # 1 hour

# Or break the task up
dial task cancel 5
dial task add "Part 1: Set up auth data model" -p 5
dial task add "Part 2: Implement auth endpoints" -p 6
dial task add "Part 3: Add auth middleware" -p 7
```

### Task Burns All 3 Attempts

The AI keeps failing on the same task. Possible causes:
- The task description is too vague — rewrite it with more detail
- The AI is missing critical context — add a learning or link to a spec section
- There's a real blocker (missing dependency, wrong API, etc.)

Fix:
```bash
# Check what's failing
dial failures

# Add context for the next attempt
dial learn "The auth library requires Node 18+" -c gotcha

# Or cancel and rewrite the task
dial task cancel 5
dial task add "Implement auth using passport.js (requires Node 18+)" -p 5 --spec 2
```

### AI Makes Same Mistake Repeatedly

DIAL records failure patterns and surfaces solutions, but the AI needs to see them. In auto-run mode, solutions are included in the prompt automatically. In manual mode, check:

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

# Add a new task for the same work (with better context)
dial task add "Implement auth with CORS headers" -p 1
```
