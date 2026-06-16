# yourmemory

A local, offline-first AI memory system in Rust. Gives any AI client long-term memory across sessions via the Model Context Protocol (MCP). Works with Claude Code, Gemini CLI, LangGraph, and Kiro — any client running in a project directory automatically shares the same memory store.

## How it works

Memory is organized into a **Palace** hierarchy:

```
Palace → Wing → Room → Drawer
```

- **Wing** — a logical group (project, person, topic)
- **Room** — a sub-topic within a wing (e.g. `auth`, `billing`)
- **Drawer** — a single stored fact or memory fragment

At session start, an AI agent calls `wakeup` to restore relevant context within a token budget. At session end (or at key moments), it calls `persist` to store new facts. The read path is fully deterministic — no LLM calls during retrieval.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| Rust 1.78+ | Install via [rustup.rs](https://rustup.rs) |
| Ollama | **Optional.** Used for write-time compression. System works without it — facts are stored uncompressed. |

To install Ollama and pull the default compression model:

```sh
# macOS / Linux
curl -fsSL https://ollama.com/install.sh | sh
ollama pull gemma:2b
```

---

## Installation

### From crates.io (recommended)

```sh
cargo install yourmemory
```

This installs two binaries: `yourmemory` (CLI) and `yourmemory-mcp` (MCP server).

### Build from source

```sh
git clone https://github.com/VSJ-superhub/rusty-mempalace
cd rusty-mempalace
cargo build --release --workspace
# Binaries are at target/release/yourmemory and target/release/yourmemory-mcp
```

---

## Quick start

```sh
yourmemory init          # Create a palace in the current directory
yourmemory setup claude  # Wire the MCP server into Claude Code
```

That's it. Open a new Claude Code session and memory is active.

---

## Initializing a palace

```sh
cd /your/project
yourmemory init
```

Creates a `.yourmemory/` directory containing:

```
.yourmemory/
├── palace.db      # SQLite store (WAL mode)
└── config.toml    # Optional — compression backend config
```

By default the palace is project-local. Each project directory gets its own palace. Multiple AI clients in the same directory share it automatically.

### config.toml (optional)

Drop a `config.toml` in your project root to configure the compression backend:

```toml
[compression]
backend = "ollama"        # ollama | openai | anthropic | none
model   = "gemma:2b"      # model name for the chosen backend
# base_url = "http://localhost:11434"   # Ollama default — omit unless changed
# api_key  = ""                         # Required for openai / anthropic
```

If the configured backend is unreachable, the system falls back to `none` automatically and logs a warning.

---

## Wiring into Claude Code

### Automatic (recommended)

```sh
yourmemory setup claude
```

This writes the correct MCP entry to `.claude/settings.json` for the current project.

### Manual

Add this to `.claude/settings.json` in your project (or `~/.claude/settings.json` for global use):

```json
{
  "mcpServers": {
    "yourmemory": {
      "type": "stdio",
      "command": "yourmemory-mcp",
      "args": []
    }
  }
}
```

To scope the palace to a specific directory:

```json
{
  "mcpServers": {
    "yourmemory": {
      "type": "stdio",
      "command": "yourmemory-mcp",
      "args": ["--project", "/absolute/path/to/project"]
    }
  }
}
```

Restart Claude Code after editing `settings.json`.

---

## The wakeup workflow

At the start of every session, an AI agent should call `wakeup` to restore relevant memory:

```
wakeup(token_budget=4000)
```

The response contains tiered context:

- **L0** — wing and room names (always included, ~170 tokens)
- **L1** — room summaries (included when budget allows)
- **L2** — drawer content for matching rooms (included on demand)
- **L3** — full verbatim content (explicit recall only)

The agent should then work normally. Before ending the session or at meaningful checkpoints, call `persist` to store new facts:

```
persist(content="Switched auth from JWT to session cookies due to mobile client requirements",
        wing="decisions", room="auth")
```

The memory system handles deduplication, compression, and routing automatically.

---

## Promotion gate

Not every write deserves to become a durable memory. Without a gate, an unattributed, low-confidence claim made mid-conversation becomes a permanent "fact" indistinguishable from a verified one — the classic place memory poisoning starts.

`persist` and `store_fact` therefore run each write through a **promotion gate**. Every write is classified into a `kind`, and a per-kind rule decides whether it is promoted to durable storage or rejected with an actionable reason.

| `kind` | Rule |
|--------|------|
| `observation` | Always promoted. Lowest tier — raw scratch notes. |
| `fact` | Promoted only if `confidence ≥ 0.7` **and** the write is attributable (a `source_run_id` is supplied, or `source` is `user` / `system` / `config` rather than `conversation`). |
| `episode` | Promoted only when `task_complete=true` — a summary of a finished task/session. |
| `policy` | Never auto-promotes. Requires explicit `confirm=true` (human-in-the-loop). |

Confidence levels map to scores as: `high=0.9`, `medium=0.7`, `low=0.4`, `inferred=0.2`. The fact threshold is `0.7`, so `high` and `medium` clear it while `low` and `inferred` do not.

When a write is rejected, the tool returns `{ "status": "rejected", "reason": "..." }` with guidance on how to make it promotable — e.g. raise confidence, supply a `source_run_id`, set `task_complete=true`, or store it as `kind=observation` instead. Nothing is silently dropped or silently written.

```
# Rejected — a bare conversational claim can't become a fact
store_fact(wing="infra", room="db", content="prod runs Postgres 16",
           kind="fact", confidence="high")
# → { "status": "rejected", "reason": "fact rejected: unattributed — supply
#     source_run_id, or set source to user/system/config" }

# Promoted — attributed via source_run_id
store_fact(wing="infra", room="db", content="prod runs Postgres 16",
           kind="fact", confidence="high", source_run_id="run-2026-06-15-a")
```

---

## MCP tools reference

### Read

| Tool | Description |
|------|-------------|
| `wakeup` | Restore memories for a session — returns tiered context (wings, rooms, drawers) within a token budget |
| `search` | Full-text search across all drawers, returns ranked results |
| `recall` | Retrieve recent drawers from a wing (and optionally a specific room) |
| `get_drawer` | Fetch a single drawer by its numeric ID |

### Write

| Tool | Description |
|------|-------------|
| `persist` | Store a memory, auto-routing to `general/notes` if wing/room are omitted; runs compression. Defaults to `kind=observation`. Subject to the [promotion gate](#promotion-gate). |
| `store_fact` | Store a fact with an explicit wing and room (no auto-routing). Defaults to `kind=fact` — facts need `confidence ≥ 0.7` and attribution. Subject to the [promotion gate](#promotion-gate). |
| `update_fact` | Replace the content of an existing drawer by ID |
| `invalidate_fact` | Soft-delete a drawer — excluded from reads but kept for audit |

Both write tools accept these gate-related arguments:

| Argument | Type | Purpose |
|----------|------|---------|
| `kind` | `observation` \| `fact` \| `episode` \| `policy` | Selects which promotion rule applies. Defaults to `observation` for `persist`, `fact` for `store_fact`. |
| `source_run_id` | string | Run/session id that produced the claim — satisfies the attribution requirement for facts. |
| `task_complete` | bool | Must be `true` to promote an `episode`. |
| `confirm` | bool | Must be `true` to promote a `policy` (human-in-the-loop). |

### Structure

| Tool | Description |
|------|-------------|
| `create_wing` | Create a wing in the palace (idempotent) |
| `create_room` | Create a room inside a wing (idempotent) |
| `list_wings` | List all wings in the palace |
| `list_rooms` | List all rooms within a specified wing |

### Maintenance

| Tool | Description |
|------|-------------|
| `health` | Return palace statistics: wing count, room count, drawer count, DB size |
| `compact` | Purge low-access drawers older than a threshold (default: 90 days, zero accesses) |
| `forget` | Permanently delete a wing or room and all its drawers |
| `export` | Dump the entire palace to JSON |

### Knowledge graph

| Tool | Description |
|------|-------------|
| `add_relation` | Add a typed relation between two drawers, with optional validity window |
| `query_relations` | Query all KG relations for a drawer, with optional direction and type filters |
| `set_validity` | Update the `valid_from` / `valid_until` window of an existing relation |

### Meta

| Tool | Description |
|------|-------------|
| `get_schema` | Return the active schema taxonomy (default: `general`) |
| `list_schemas` | List all available schema names from the `schemas/` directory |
| `setup_claude` | Generate the `.claude/settings.json` MCP entry for this project |

---

## CLI commands

```
yourmemory <command> [options]
```

| Command | What it does |
|---------|--------------|
| `init` | Initialize a palace in the current directory (creates `.yourmemory/`) |
| `setup <client>` | Write the MCP config for a client — `claude`, `gemini`, or `kiro` |
| `mine [path]` | Crawl a directory and index existing files into the palace |
| `search <query>` | Search the palace from the terminal |
| `wakeup` | Print restored context to stdout (useful for debugging) |
| `persist <text>` | Store a fact from the command line |

### Examples

```sh
# Initialize and wire Claude Code in one step
yourmemory init && yourmemory setup claude

# Index existing project files
yourmemory mine ./src

# Search from the terminal
yourmemory search "auth token expiry"

# Store a one-off fact
yourmemory persist "Deploy target is fly.io, region iad"

# Check what the MCP server would return on wakeup
yourmemory wakeup
```

---

## Compression backends

Write-time compression condenses stored facts to save tokens on future reads. The read path never calls a model.

| Backend | Config value | Notes |
|---------|-------------|-------|
| Ollama (default) | `backend = "ollama"` | Local, no API key needed; requires Ollama running |
| OpenAI-compatible | `backend = "openai"` | Works with OpenAI, Groq, LM Studio, llamafile, Together.ai — set `base_url` and `api_key` |
| Anthropic | `backend = "anthropic"` | Uses Claude Haiku; set `api_key` |
| None | `backend = "none"` | Stores raw content; also the automatic fallback when a backend is unreachable |

---

## LangGraph integration

Install the Python package:

```sh
pip install yourmemory-langgraph
```

```python
from yourmemory_langgraph import MemoryReadNode, MemoryWriteNode, MemoryState

# Add to your LangGraph workflow
graph.add_node("memory_read", MemoryReadNode())
graph.add_node("memory_write", MemoryWriteNode())
```

The LangGraph nodes communicate with the same `yourmemory-mcp` binary via MCP — no separate process needed if the MCP server is already configured.

---

## Troubleshooting

**`yourmemory-mcp` not found after `cargo install`**

Ensure `~/.cargo/bin` is on your `PATH`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

**MCP server not appearing in Claude Code**

- Confirm `settings.json` is valid JSON (trailing commas will break parsing)
- Restart Claude Code fully after editing settings
- Run `yourmemory health` in the terminal to verify the palace opens correctly

**Compression not working / facts stored uncompressed**

- Check that Ollama is running: `ollama list`
- Verify the model is pulled: `ollama pull gemma:2b`
- Run `yourmemory health` — it will show `status: ok` regardless; compression errors are warnings only
- Set `backend = "none"` in `config.toml` to disable compression entirely

**`wakeup` returns no drawers**

The palace may be empty. Store something first:

```sh
yourmemory persist "This is a test memory"
yourmemory wakeup
```

**Palace is in the wrong directory**

`yourmemory` looks for `.yourmemory/` starting from the current working directory. Run `yourmemory init` in the correct project root, or pass `--project` to `yourmemory-mcp`.

**Drawers accumulating from old sessions**

Run `compact` to remove low-access drawers older than 90 days:

```
compact(max_age_days=90)
```

Or from the CLI:

```sh
yourmemory compact
```

---

## Contributing

Contributions welcome. The repo is at [github.com/VSJ-superhub/rusty-mempalace](https://github.com/VSJ-superhub/rusty-mempalace).

```sh
git clone https://github.com/VSJ-superhub/rusty-mempalace
cd rusty-mempalace
cargo test --workspace
```

Please open an issue before submitting a large PR.

---

## License

MIT
