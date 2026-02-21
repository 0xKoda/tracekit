# tracekit

![tracekit hero](./assets/hero.png)

A Rust CLI that analyzes coding-agent session traces to identify token/cost inefficiencies and suggest optimizations.

## Why

Coding-agent sessions waste tokens through avoidable patterns:

- Failed edits that trigger repeated model calls
- Malformed tool payloads requiring retries
- Redundant file reads with no state change
- Tool fan-out that could be batched in one call
- Excessive context injection driving high cache-write costs
- Repeated subagent/trace replay without summarization

tracekit makes these inefficiencies **measurable and fixable**.

## Supported Agents

| Agent | Session Location |
|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` |
| OpenCode | `~/.local/share/opencode/storage/` |
| Codex (ChatGPT) | `~/.codex/sessions/**/*.jsonl` |
| Pi | `~/.pi/agent/sessions/**/*.jsonl` |
| Kodo | `~/.kodo/sessions/**/*.jsonl` |

## Install

```bash
cargo install --path crates/tracekit-cli
```

Or build locally:

```bash
cargo build --release
./target/release/tracekit --help
```

## Quick Start

```bash
# List all sessions across all agents
tracekit list sessions

# List sessions for a specific agent
tracekit list sessions --agent claude
tracekit list sessions --agent opencode --since 2026-01-01

# Analyze a specific session
tracekit analyze session --session-id <id>

# Analyze the 10 most recent sessions
tracekit analyze recent --limit 10

# Find the most expensive sessions
tracekit analyze expensive --top 20

# Generate an HTML report for a session
tracekit report session --session-id <id> --format html

# Generate a JSON report
tracekit report session --session-id <id> --format json

# Aggregate HTML report across all sessions
tracekit report aggregate --format html --out report.html
```

## Commands

### `capture`

Discover and inspect sessions without full analysis.

```bash
tracekit capture all --agent all
tracekit capture recent --agent claude --limit 20
tracekit capture session --session-id <id>
tracekit capture session --session-id <id> --inspect-file
tracekit capture session --session-id <id> --inspect-file --inspect-mode analysis
tracekit capture session --session-id <id> --inspect-file --inspect-mode forensic
tracekit capture session --session-id <id> --inspect-terminal
tracekit capture session --session-id <id> --inspect-file --inspect-terminal
```

### `list sessions`

Display a session table with agent, ID, CWD, start time, message count, and cost.

```bash
tracekit list sessions --agent all
tracekit list sessions --agent codex --since 2026-01-01
tracekit list sessions --model-id gpt-5
```

**Filters:** `--agent`, `--since`, `--until`, `--cwd`, `--model-id`, `--limit`

### `analyze`

Run inefficiency detection and cost analysis.

```bash
tracekit analyze session --session-id <id>
tracekit analyze recent --agent claude --limit 20
tracekit analyze expensive --top 10 --agent all
```

**Options:** `--optimize-for cost|latency|reliability`, `--format table|json`

### `report`

Generate full reports in table, JSON, or HTML format.

```bash
tracekit report session --session-id <id> --format html --out report.html
tracekit report aggregate --agent all --since 2026-01-01 --format html
```

## Inefficiency Detectors

| Pattern | Description |
|---|---|
| `RETRY_LOOP` | Same tool called again after an error, without corrective input |
| `EDIT_CASCADE` | Repeated failed edits on the same file |
| `TOOL_FANOUT` | 4+ calls to the same tool in one turn that could be batched |
| `REDUNDANT_REREAD` | Same file read 3+ times with no writes in between |
| `CONTEXT_BLOAT` | Input token spike >3× session average — likely over-injected context |
| `ERROR_REPROMPT_CHURN` | Same error class repeated 3+ consecutive turns |
| `SUBAGENT_OVERHEAD` | High sidechain/subagent usage — check if tasks could be inlined |

Each finding includes:
- Evidence (turn numbers)
- Estimated wasted tokens
- Estimated wasted cost
- Confidence score

## Cost Normalization

- **OpenCode**: uses the `cost` field recorded directly in session files
- **Claude Code**: estimates from token counts × model pricing catalog
- **Codex**: structural analysis only (no per-call token counts in rollout files)

The pricing catalog covers Claude 3/4 families, GPT-4/4o/5, o3/o4, Gemini, and Kimi models.

## Workspace Layout

```
crates/
  tracekit-core/      canonical schema, pricing catalog, detectors
  tracekit-ingest/    source adapters (claude, opencode, codex)
  tracekit-report/    terminal, JSON, HTML renderers
  tracekit-cli/       CLI commands (capture, list, analyze, report)
```

## As a Coding Agent Skill

See [SKILL.md](./SKILL.md) for instructions on using tracekit as a repeatable skill in Claude Code, Codex, or OpenCode sessions.

## Roadmap

### LLM-Assisted Analysis *(coming soon)*

tracekit currently uses deterministic rule-based detectors. The next layer will pass
finding bundles to an LLM (via OpenRouter) for deeper, contextual analysis:

- **Root-cause diagnosis** — why is the model retrying? Is it a prompt issue, a tool
  output format problem, or a context starvation pattern?
- **Concrete fix suggestions** — rewrite the system prompt section driving the bloat,
  add a batch instruction for fan-out tools, restructure the edit workflow
- **Session comparison** — "this session cost 3× more than your last similar task — here's why"
- **Prompt diff recommendations** — specific before/after prompt edits estimated to reduce cost

The LLM layer will be opt-in (`--with-llm --provider openrouter --model <id>`),
bounded in prompt size, and support `--redact-secrets` for privacy.

## License

MIT
