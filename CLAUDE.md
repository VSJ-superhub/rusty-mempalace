# yourmemory

A local, offline-first AI memory system in Rust. Published on GitHub and crates.io for anyone to use.

## What it is

Single binary (`cargo install yourmemory`) that gives any AI client long-term memory across sessions. Works with Claude CLI, Gemini CLI, LangGraph, and Kiro via MCP. Any client running in a project directory automatically shares the same palace.

## Compression backend (pluggable)

Write-time compression uses a `Compressor` trait. Four implementations:
- `OllamaCompressor` — default, local, no key
- `OpenAICompatibleCompressor` — covers OpenAI, Groq, LM Studio, llamafile, Together.ai (configurable `base_url`)
- `AnthropicCompressor` — Haiku, separate because API format differs
- `NoopCompressor` — skip compression; also the automatic fallback when backend unreachable

Configured in `config.toml` under `[compression]`. Read path is always deterministic — no model calls on read regardless of backend.

## Key design constraints

- **Public OSS** — no internal project assumptions. The `general` schema is the primary use case. Domain schemas (incident, education) are optional reference examples, not core features.
- **Zero config to start** — `yourmemory init && yourmemory setup claude` is the entire onboarding.
- **Ollama optional** — system works without it; compression skipped if Ollama not running.
- **No PyO3** — MCP is the interface for all clients. Single binary, no Python version pinning.
- **Read path is deterministic** — no live model calls on read. Gemma only at write time.

## Repo structure

```
yourmemory/
├── core/          # Rust storage, KG, forgetting, WAL
├── gemma/         # Ollama write-time compression client
├── mcp/           # MCP stdio server — 20 tools
├── cli/           # init, setup, health, mine, search, wakeup, persist
├── hooks/         # Client config templates
├── schemas/       # Domain taxonomy definitions
├── langgraph/     # Python package for LangGraph integration
├── examples/      # Working examples for different use cases
└── benches/       # LongMemEval + BEAM evaluation
```

## Core algorithms (already built in rust_scratch)

These modules should be ported/adapted, not rewritten:

- `rust_scratch/src/entropy.rs` → write-time compression scoring
- `rust_scratch/src/knapsack.rs` → L0–L3 layered token budget selection
- `rust_scratch/src/lsh.rs` → MinHash similarity search for drawer retrieval
- `rust_scratch/provenance-mcp/src/db.rs` → rusqlite storage pattern (extend for Palace schema)
- `rust_scratch/provenance-mcp/src/main.rs` + `protocol.rs` → MCP server scaffold
- `rust_scratch/src/mcp_agent.rs` → `ollama_chat()` for Gemma calls
- `rust_scratch/src/walker.rs` → parallel file walking for `mine`

Path to rust_scratch: `C:/Users/alway/Projects/eng_team/rust_scratch/`

## Memory structure

```
Palace → Wing → Room → Drawer
```

- Wings: logical groupings (project, person, topic)
- Rooms: sub-topics within a wing
- Drawers: individual stored facts
- Knowledge Graph: temporal relations with validity windows

Token loading: L0 (metadata ~170 tokens always) → L1 (room summaries on match) → L2 (drawers on demand) → L3 (full verbatim).

## MCP tools (target: ~20)

Read: `wakeup`, `search`, `recall`, `get_drawer`
Write: `persist`, `store_fact`, `update_fact`, `invalidate_fact`
Structure: `create_wing`, `create_room`, `list_wings`, `list_rooms`
Maintenance: `health`, `compact`, `forget`, `export`
KG: `add_relation`, `query_relations`, `set_validity`
Meta: `get_schema`, `list_schemas`

## Distribution targets

- crates.io: `yourmemory` binary crate
- PyPI: `yourmemory-langgraph` package
- GitHub releases: pre-built binaries (Linux, macOS, Windows)

## Architecture reference

See `ARCHITECTURE.md` for full design including competitive analysis, token savings profile, and client integration details.
