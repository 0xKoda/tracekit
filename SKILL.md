# tracekit skill

Use tracekit to capture, list, and analyze coding-agent session traces for token/cost inefficiencies.

## When to use this skill

- A user asks to review their session costs, token usage, or agent efficiency
- A user wants to know which sessions were most expensive
- A user wants to find retry loops, redundant reads, or context bloat in past sessions
- A user is ending a session and wants to capture and analyze it

## Binary location

After building: `./target/release/tracekit` or `tracekit` if installed globally.

## Workflow

### 1. List sessions

```bash
# All agents, newest first
tracekit list sessions

# Filter by agent
tracekit list sessions --agent claude
tracekit list sessions --agent opencode
tracekit list sessions --agent codex

# Filter by time window
tracekit list sessions --since 2026-01-01

# JSON output for programmatic use
tracekit list sessions --format json
```

### 2. Analyze a session

```bash
# By session ID (prefix match works)
tracekit analyze session --session-id <id>

# Most recent N sessions
tracekit analyze recent --limit 10

# Most expensive sessions
tracekit analyze expensive --top 20

# JSON output
tracekit analyze session --session-id <id> --format json
```

### 3. Generate a report

```bash
# Terminal table (default)
tracekit report session --session-id <id>

# HTML report (opens as file)
tracekit report session --session-id <id> --format html --out report.html

# Full aggregate HTML report
tracekit report aggregate --format html --out report.html

# JSON report for automation
tracekit report session --session-id <id> --format json --out report.json
```

### 4. Capture traces at session end

```bash
# Discover all sessions
tracekit capture all

# Recent sessions only
tracekit capture recent --limit 5

# Specific session
tracekit capture session --session-id <id>
```

## Common patterns

**Analyze the current session (Claude Code):**
```bash
# Get your current session ID from Claude Code, then:
tracekit analyze session --session-id <current-session-id> --agent claude
```

**Find sessions with the most wasted tokens:**
```bash
tracekit analyze expensive --top 20 --format json | jq '.sessions[] | {id: .session.session_id, cost: .session.total_cost_usd, findings: (.findings | length)}'
```

**Check for specific inefficiency types:**
```bash
tracekit analyze recent --limit 20 --format json | jq '[.sessions[].findings[] | select(.kind == "retry_loop")]'
```

**Generate a weekly cost report:**
```bash
tracekit report aggregate --since $(date -u -v-7d +%Y-%m-%d) --format html --out weekly-report.html
```

## Interpreting findings

| Finding | Fix |
|---|---|
| `RETRY_LOOP` | Agent retried a failing tool repeatedly. Fix the underlying tool error or give the agent better error-handling instructions. |
| `EDIT_CASCADE` | Edit tool kept failing on same file. Check file permissions or patch format. |
| `TOOL_FANOUT` | Many calls to same tool in one turn. Use batch tool calls or parallel execution instructions. |
| `REDUNDANT_REREAD` | Same file read many times. Cache file content in context instead of re-reading. |
| `CONTEXT_BLOAT` | Input token spike. Compress tool outputs before feeding back, or use summarization. |
| `ERROR_REPROMPT_CHURN` | Repeating the same error without recovery. Add explicit error-handling paths to system prompt. |
| `SUBAGENT_OVERHEAD` | Many sidechain agents. Evaluate if tasks need subagents or can be done inline. |

## Notes

- Claude Code and OpenCode provide real cost data where available
- Codex rollout files don't include per-call token counts; only structural analysis is available
- Session IDs support prefix matching (first 8 chars usually sufficient)
- Use `--agent all` (default) to search across all installed agents
