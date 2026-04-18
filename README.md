# yourmemory

A local, offline-first AI memory system in Rust. Gives any AI client long-term memory across sessions via the Model Context Protocol (MCP).

## Crates

| Crate | Description |
|-------|-------------|
| `core` | Storage engine: SQLite Palace, knowledge graph, WAL, forgetting |
| `gemma` | Ollama write-time compression client |
| `mcp` | MCP stdio server exposing ~20 memory tools |
| `cli` | `yourmemory` binary: init, setup, mine, search, wakeup, persist |

## Quick start

```sh
cargo install yourmemory
yourmemory init
yourmemory setup claude
```

## Build

```sh
cargo build --workspace
```
