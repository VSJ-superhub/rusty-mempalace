# yourmemory — Architecture

## Overview

A local, offline-first AI memory system in Rust. One binary, one shared palace, any AI client reads from the same store.

Designed for public distribution (`cargo install yourmemory`). Works with any project, any stack, any team. No cloud dependency, no account, no telemetry.

---

## System Diagram

```
Claude CLI  ──┐
Gemini CLI  ──┤──→  MCP Server  ──→  Rust Core  ──→  Local Storage (.yourmemory/)
LangGraph   ──┤                          ↑
Kiro        ──┘                     Gemma 1B (Ollama)
                                    write-time only
```

### Palace Discovery (automatic, works in any project)

```
walk up from cwd → look for .yourmemory/ → fallback to ~/.yourmemory/global
```

Drop `.yourmemory/` in any project root and it just works. Global palace at `~/.yourmemory/global` used when no project palace is found.

---

## First-Run Experience

```bash
cargo install yourmemory          # install binary

cd my-project
yourmemory init                   # creates .yourmemory/ with config.toml + schema
yourmemory mine ./                # index project files into palace
yourmemory health                 # verify palace state

# Pick your client integration:
yourmemory setup claude           # writes .claude/hooks.json
yourmemory setup gemini           # writes gemini MCP config
yourmemory setup kiro             # writes .kiro/hooks.json
```

Zero config required. `config.toml` is optional and generated with sensible defaults.

---

## Repository Structure

```
yourmemory/
├── core/          # Rust — storage engine, knowledge graph, forgetting, WAL
├── gemma/         # Ollama client — write-time classification & compression
├── mcp/           # MCP server — 20 tools exposed to all MCP-compatible clients
├── server/        # HTTP server + REST API + embedded web dashboard (Axum, opt-in)
├── cli/           # CLI commands: init, setup, health, mine, search, wakeup, persist, serve, token
├── hooks/         # Hook config templates per client (Claude, Gemini, Kiro)
├── schemas/       # Domain taxonomy definitions (general, incident, education, custom)
├── langgraph/     # Python package — MemoryReadNode, MemoryWriteNode, MemoryState
├── examples/      # Working examples: FastAPI agent, LangGraph incident bot, etc.
├── docs/          # User-facing documentation
└── benches/       # LongMemEval + BEAM evaluation harness
```

---

## Memory Structure

```
Palace
├── Wing          (project, person, topic, or any logical grouping)
│   ├── Room      (sub-topic: e.g. auth, billing, incidents, students)
│   │   └── Drawer  (individual stored fact or memory fragment)
│   └── ...
├── Hall          (memory type shared across wings)
└── Knowledge Graph  (temporal facts with validity windows)
```

Wings, rooms, and drawers are user-defined. The `general` schema creates them automatically from conversation context. Domain schemas (incident, education) provide opinionated starting taxonomies — users can ignore or extend them.

### Token Loading Layers

| Layer | Content | Tokens (approx) | When loaded |
|-------|---------|-----------------|-------------|
| L0 | Wing/room metadata | ~170 | Always |
| L1 | Room summaries for relevant wings | variable | On wing match |
| L2 | Specific drawers | variable | On demand |
| L3 | Full verbatim content | variable | Exact recall only |

---

## Two-Tier Model Strategy

| Role | Model | When |
|------|-------|------|
| Classification, compression, room detection | Gemma 1B (local/Ollama) | Write time only |
| Retrieval scoring | Deterministic (vector + keyword) | Read time |
| Actual reasoning | User's model (Claude, Gemini, GPT, etc.) | Receives compressed context |

**Rationale:** Compression on write is low-risk and amortized. Live model calls on read add latency and failure modes. Read path stays deterministic. User keeps control of their expensive model.

Ollama is **optional** — if not running, write-time compression is skipped and raw content is stored. The system degrades gracefully.

---

## Reuse from rust_scratch (already built)

These modules transfer directly into `yourmemory/core/`:

| Module | What it provides | yourmemory use |
|--------|-----------------|----------------|
| `provenance-mcp/main.rs` + `protocol.rs` | MCP stdio JSON-RPC server loop | Scaffold for yourmemory MCP server |
| `provenance-mcp/db.rs` | rusqlite open/WAL/query pattern | Extend for Palace/Wing/Room/Drawer schema |
| `entropy.rs` | Shannon entropy scoring + ranking | Write-time compression priority decisions |
| `knapsack.rs` | 0/1 DP + greedy token budget selection | `MemoryReadNode(token_budget=N)` enforcement — L0–L3 loading |
| `lsh.rs` | MinHash + band LSH similarity search | Sub-linear drawer retrieval on read path |
| `mcp_agent.rs` | `ollama_chat()` blocking client | Gemma 1B write-time compression calls |
| `ollama_stream.rs` | Streaming Ollama client | Streaming compression for large fragments |
| `walker.rs` | Parallel file walker (rayon + walkdir) | `yourmemory mine ./` project indexing |
| `ast_extractor.rs` | tree-sitter AST (Python/Rust/TS) | Typed drawer creation from code during mine |
| `tiktoken-rs` (dep) | Token counting | Budget enforcement in L0–L3 loading |

Cargo deps that transfer zero-cost: `serde_json`, `reqwest`, `rusqlite`, `chrono`, `walkdir`, `rayon`, `regex`, `ollama-rs`, `tokio`, `tokio-stream`, `tree-sitter` + grammars, `tiktoken-rs`, `git2`, `diffy`.

---

## Core Design Decisions

### Forgetting is first-class
- Low-access drawers compress further over time (tiered decay)
- Contradicted facts are invalidated, not deleted (audit trail preserved)
- Storage stays bounded — no unbounded growth

### Confidence scores on every fact
- Source type tagged at write time: `conversation | config | system | user`
- Confidence level: `high | medium | low | inferred`
- Low-confidence facts flagged before context injection

### Write-ahead log
- Operations logged before storage write
- No palace corruption on crash or power loss

### Pluggable compression backends

Write-time compression uses a `Compressor` trait — users swap backends via `config.toml`, no code changes:

```rust
trait Compressor: Send + Sync {
    fn compress(&self, content: &str, hint: &str) -> Result<CompressedMemory>;
}
```

| Backend | Config value | Covers |
|---------|-------------|--------|
| Ollama | `"ollama"` (default) | Local Gemma 1B, no API key |
| OpenAI-compatible | `"openai"` + `base_url` | OpenAI, Groq, LM Studio, llamafile, Together.ai, Anyscale, Ollama REST |
| Anthropic | `"anthropic"` | Claude Haiku — different API format |
| None | `"none"` | Skip compression, store raw content |

`"openai"` with a custom `base_url` covers ~80% of alternatives. Anthropic is a separate implementation only because its auth and API format differ.

`NoopCompressor` is the automatic fallback when the configured backend is unreachable — system stays functional, compression is skipped.

```toml
[compression]
backend     = "ollama"        # ollama | openai | anthropic | none
model       = "gemma:1b"

# OpenAI-compatible example (Groq):
# backend      = "openai"
# base_url     = "https://api.groq.com/openai/v1"
# api_key_env  = "GROQ_API_KEY"
# model        = "gemma2-9b-it"

# Anthropic example:
# backend      = "anthropic"
# api_key_env  = "ANTHROPIC_API_KEY"
# model        = "claude-haiku-4-5-20251001"
```

**Constraint:** pluggability applies to write-time only. Read path stays deterministic regardless of backend choice.

### Pluggable embedding backends
| Backend | Default | Notes |
|---------|---------|-------|
| `nomic-embed-text` via Ollama | Yes | Fully local, no account |
| `text-embedding-3-small` | Optional | OpenAI cloud |
| Custom | Optional | Implement the `Embedder` trait |

### No PyO3
MCP is the interface for all clients. Single binary, broad compatibility, no Python version pinning.

### Ollama is optional
If Ollama isn't running, compression is skipped — raw content stored. Full functionality requires Ollama but the system is usable without it.

---

## MCP Server (20 tools)

Exposed over stdio to any MCP-compatible client:

| Group | Tools |
|-------|-------|
| Read | `wakeup`, `search`, `recall`, `get_drawer` |
| Write | `persist`, `store_fact`, `update_fact`, `invalidate_fact` |
| Structure | `create_wing`, `create_room`, `list_wings`, `list_rooms` |
| Maintenance | `health`, `compact`, `forget`, `export` |
| KG | `add_relation`, `query_relations`, `set_validity` |
| Meta | `get_schema`, `list_schemas` |

All tools return structured JSON. Error responses include actionable messages.

---

## Network Server & Web Dashboard (`yourmemory serve`)

An **opt-in** HTTP layer in the `server/` crate (Axum + tokio), separate from the stdio MCP
server. It serves a single-page web dashboard (`/ui`) and a REST API (`/api`) for browsing and
editing the palace: Graph Explorer, Room Navigator, Audit Log, Confidence Heatmap, Admin.

The dashboard is the first feature that opens a network port, so it is **secure by default**:

```
yourmemory serve                       # binds 127.0.0.1:7700 only (loopback)
yourmemory serve --listen 0.0.0.0:7700 # refuses to start unless a token exists
```

- **Loopback by default.** No flags ⇒ loopback-only, safe to run on a laptop with zero config.
- **No anonymous network exposure.** Binding to a non-loopback address requires at least one
  configured access token, or `serve` exits with instructions.
- **One write path.** The REST API wraps existing `core` Palace + WAL functions — it never opens
  its own connection for hand-written write SQL. UI edits and CLI/MCP writes share one audit trail.
- **Read path stays deterministic.** Dashboard read endpoints never invoke Gemma/Ollama.
- **Static assets embedded** in the binary via `rust-embed` (target: <2MB size increase). No
  separate frontend server, no build step.

Full UI/endpoint spec lives in `.claude/frontend.md`.

---

## Access Control (token / grant model)

The dashboard's compliance value (scoped audit log, CSV export) requires a **real** access
boundary, not a stub. The token/grant model lives in `core` so scoping is enforced inside query
functions — no endpoint can leak an out-of-scope wing.

- A **token** has: id, label, hashed secret (never stored plaintext), created_at, last_used_at,
  revoked_at (soft revoke — audit-preserving).
- A token holds **per-wing grants**: `(token_id, wing, level)` with `level ∈ read | write | admin`.
- No grant for a wing ⇒ that wing is invisible in graph, search, audit, heatmap, and stats.
- A global admin grant (`*` wing, `admin`) permits token management.

CLI surface:

```bash
yourmemory token create --label <name> --grant <wing>:<read|write|admin> [...]  # prints secret once
yourmemory token list                                                           # masked
yourmemory token revoke <id>                                                    # soft
yourmemory token grant <id> <wing>:<level>
```

The HTTP auth middleware resolves `Authorization: Bearer <token>` → grant set, then every handler
scopes its `core` calls to that set (401 missing/invalid, 403 insufficient grant). Auth failures
are rate-limited and written to the WAL. This access model applies only to the network server; the
local stdio MCP/CLI path is unchanged (single-user, filesystem-permission trust).

---

## Domain Schemas

Schemas are **optional taxonomies** that give the palace opinionated structure. Users can:
- Use `general` (default) — auto-creates wings and rooms from context
- Use a built-in schema — gets a pre-built wing/room hierarchy
- Define a custom schema in `config.toml`
- Mix schemas across wings

### Built-in schemas (ship as examples, not required)

**`general`** — default for all users
- Freeform wing/room creation from conversation context
- Works for any project without configuration

**`incident_response`** — reference for SRE/on-call teams
- Wings: services, teams, runbooks
- Rooms: outage history, ownership, postmortems, dependencies
- KG: ownership validity windows, causal incident chains

**`education_tutoring`** — reference for tutoring platforms
- Wings: students, programs, locations
- Rooms: assessment history, parent comms, learning goals

Custom schemas defined in `schemas/*.toml` — documented format so users can contribute their own.

---

## LangGraph Integration (Python package: `yourmemory-langgraph`)

Published separately to PyPI. Three entry levels:

```python
# Level 1 — drop-in nodes (10 min setup)
from yourmemory.langgraph import MemoryReadNode, MemoryWriteNode

# Level 2 — typed state mixin
from yourmemory.langgraph import MemoryState
class MyAgentState(MemoryState): ...

# Level 3 — graph factory
from yourmemory.langgraph import MemoryGraphFactory
graph = MemoryGraphFactory(domain="general", llm=your_llm)
```

`MemoryReadNode` accepts a `token_budget` parameter — auto-fits retrieved context via layered loading → compression → truncation.

Communicates with the Rust core over the MCP server (localhost socket or stdio).

---

## Client Integrations

### Claude CLI (`yourmemory setup claude`)

Writes to `.claude/hooks.json`:

```json
{
  "pre_tool": "yourmemory wakeup --budget 500",
  "post_tool": "yourmemory persist --session $SESSION_ID"
}
```

### Gemini CLI (`yourmemory setup gemini`)

Writes Gemini MCP config:

```json
{
  "mcpServers": {
    "yourmemory": {
      "command": "yourmemory",
      "args": ["mcp-server"],
      "env": { "MEMORY_DOMAIN": "general", "TOKEN_BUDGET": "600" }
    }
  }
}
```

### Kiro (`yourmemory setup kiro`)

Writes `.kiro/hooks.json` in the same pattern as Claude CLI.

---

## CLI Reference

```bash
yourmemory init                    # initialize palace in current project
yourmemory setup <claude|gemini|kiro>  # write client integration config
yourmemory mine [path]             # index project files into palace
yourmemory health                  # palace stats, integrity check
yourmemory search "<query>"        # search memories, print results
yourmemory wakeup --budget <N>     # load relevant context within token budget (stdout)
yourmemory persist [--session <id>] # flush session buffer to palace
yourmemory compact                 # run forgetting + compression pass
yourmemory export [--format json]  # export palace contents
yourmemory mcp-server              # start MCP server (used by client configs)
yourmemory serve [--listen <addr>] # start HTTP server + web dashboard (loopback by default)
yourmemory token <create|list|revoke|grant> ...  # manage dashboard access tokens
```

---

## Configuration (`config.toml` — optional)

```toml
[palace]
schema = "general"          # domain schema
embedding = "nomic-embed-text"  # embedding backend
ollama_url = "http://localhost:11434"

[budget]
wakeup_tokens = 500         # default token budget for wakeup
max_palace_mb = 500         # storage cap

[forgetting]
decay_days = 90             # compress after N days without access
invalidate_on_contradiction = true
```

---

## Distribution

- **Rust crate**: `cargo install yourmemory` from crates.io
- **Python package**: `pip install yourmemory-langgraph` from PyPI (LangGraph nodes only)
- **GitHub releases**: pre-built binaries for Linux, macOS, Windows
- **Homebrew tap**: `brew install yourmemory/tap/yourmemory`

---

## Competitive Positioning

| Feature | mempalace (Python) | mempalace-rs | yourmemory |
|---|---|---|---|
| Language | Python | Rust | Rust |
| Claude CLI hooks | ✅ | ✅ | ✅ |
| Gemini CLI | ❌ | ❌ | ✅ |
| Kiro | ❌ | ❌ | ✅ |
| LangGraph nodes | ❌ | ❌ | ✅ |
| Gemma write compression | ❌ | ❌ | ✅ |
| Works without Ollama | ✅ | ✅ | ✅ (degrades gracefully) |
| Domain schemas | Generic | Generic | General + pluggable |
| Multi-client shared palace | ❌ | ❌ | ✅ |
| Confidence scores | ❌ | ❌ | ✅ |
| Principled forgetting | ❌ | ❌ | ✅ |
| Single binary | ❌ | ✅ | ✅ |
| crates.io installable | ❌ | ✅ | ✅ |

---

## Build, Test, Publish

```bash
cargo build --release
cargo test
cargo bench              # LongMemEval + BEAM harness

# Publish
cargo publish -p yourmemory-core
cargo publish -p yourmemory-mcp
cargo publish -p yourmemory       # top-level binary
pip publish yourmemory-langgraph  # Python package
```

---

## Status

Pre-build. Architecture finalized. Core algorithms (entropy, knapsack, LSH, MCP server, SQLite, Ollama client) already implemented in rust_scratch — ready to be ported into the workspace.
