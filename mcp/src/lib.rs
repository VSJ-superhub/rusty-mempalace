use std::path::PathBuf;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use yourmemory_core::{
    budget,
    storage::{Confidence, NewDrawer, Palace, Source, Storage},
};
use yourmemory_gemma::{build_compressor, CompressionConfig};

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = handle_message(trimmed).await {
            let s = serde_json::to_string(&response)?;
            stdout.write_all(s.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

async fn handle_message(line: &str) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("invalid JSON: {}", e);
            return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}));
        }
    };

    let id = req.get("id")?.clone();

    let method = req.get("method")?.as_str()?;
    let params = req.get("params").cloned().unwrap_or(json!({}));

    let result: anyhow::Result<Value> = match method {
        "initialize" => Ok(handle_initialize()),
        "initialized" => return None,
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(&params).await,
        "ping" => Ok(json!({})),
        _ => Err(anyhow::anyhow!("method not found: {}", method)),
    };

    Some(match result {
        Ok(r) => json!({"jsonrpc":"2.0","id":id,"result":r}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":e.to_string()}}),
    })
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "yourmemory", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn handle_tools_list() -> Value {
    let tools = vec![
        tool_def("wakeup", "Restore memories for a session. Returns tiered context (wings, rooms, drawers) within a token budget.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string", "description": "Project directory (defaults to cwd)" },
                "token_budget": { "type": "integer", "description": "Max tokens to return (default 4000)" }
            }
        })),
        tool_def("search", "Full-text search across drawers.", json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "project_path": { "type": "string" },
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 10)" }
            }
        })),
        tool_def("recall", "Retrieve recent drawers from a wing (and optionally a room).", json!({
            "type": "object",
            "required": ["wing"],
            "properties": {
                "project_path": { "type": "string" },
                "wing": { "type": "string", "description": "Wing name" },
                "room": { "type": "string", "description": "Room name (optional)" },
                "limit": { "type": "integer", "description": "Max results (default 20)" }
            }
        })),
        tool_def("get_drawer", "Fetch a single drawer by ID.", json!({
            "type": "object",
            "required": ["drawer_id"],
            "properties": {
                "project_path": { "type": "string" },
                "drawer_id": { "type": "integer" }
            }
        })),
        tool_def("persist", "Store a memory. Auto-routes to 'general/notes' if wing/room omitted. Runs compression.", json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "project_path": { "type": "string" },
                "content": { "type": "string" },
                "wing": { "type": "string", "description": "Wing name (default: general)" },
                "room": { "type": "string", "description": "Room name (default: notes)" },
                "source": { "type": "string", "enum": ["conversation","config","system","user"] },
                "confidence": { "type": "string", "enum": ["high","medium","low","inferred"] }
            }
        })),
        tool_def("store_fact", "Store a fact with explicit wing and room.", json!({
            "type": "object",
            "required": ["wing", "room", "content"],
            "properties": {
                "project_path": { "type": "string" },
                "wing": { "type": "string" },
                "room": { "type": "string" },
                "content": { "type": "string" },
                "source": { "type": "string", "enum": ["conversation","config","system","user"] },
                "confidence": { "type": "string", "enum": ["high","medium","low","inferred"] }
            }
        })),
        tool_def("update_fact", "Replace the content of an existing drawer.", json!({
            "type": "object",
            "required": ["drawer_id", "content"],
            "properties": {
                "project_path": { "type": "string" },
                "drawer_id": { "type": "integer" },
                "content": { "type": "string" }
            }
        })),
        tool_def("invalidate_fact", "Soft-delete a drawer (excluded from reads, kept for audit).", json!({
            "type": "object",
            "required": ["drawer_id"],
            "properties": {
                "project_path": { "type": "string" },
                "drawer_id": { "type": "integer" }
            }
        })),
        tool_def("create_wing", "Create a wing (idempotent).", json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "project_path": { "type": "string" },
                "name": { "type": "string" },
                "description": { "type": "string" }
            }
        })),
        tool_def("create_room", "Create a room inside a wing (idempotent).", json!({
            "type": "object",
            "required": ["wing", "name"],
            "properties": {
                "project_path": { "type": "string" },
                "wing": { "type": "string" },
                "name": { "type": "string" },
                "summary": { "type": "string" }
            }
        })),
        tool_def("list_wings", "List all wings in the palace.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" }
            }
        })),
        tool_def("list_rooms", "List rooms in a wing.", json!({
            "type": "object",
            "required": ["wing"],
            "properties": {
                "project_path": { "type": "string" },
                "wing": { "type": "string" }
            }
        })),
        tool_def("health", "Return palace statistics (wings, rooms, drawers, db size).", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" }
            }
        })),
        tool_def("compact", "Purge low-access drawers older than a threshold.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" },
                "max_age_days": { "type": "integer", "description": "Purge drawers older than N days with zero accesses (default 90)" }
            }
        })),
        tool_def("forget", "Permanently delete a wing or room and all its drawers.", json!({
            "type": "object",
            "required": ["wing"],
            "properties": {
                "project_path": { "type": "string" },
                "wing": { "type": "string" },
                "room": { "type": "string", "description": "If omitted, deletes the entire wing" }
            }
        })),
        tool_def("export", "Dump the entire palace to JSON.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" }
            }
        })),
        tool_def("add_relation", "Add a knowledge-graph relation between two drawers.", json!({
            "type": "object",
            "required": ["from_id", "to_id", "relation"],
            "properties": {
                "project_path": { "type": "string" },
                "from_id": { "type": "integer" },
                "to_id": { "type": "integer" },
                "relation": { "type": "string" },
                "valid_from": { "type": "string" },
                "valid_until": { "type": "string" }
            }
        })),
        tool_def("query_relations", "Query KG relations for a drawer.", json!({
            "type": "object",
            "required": ["drawer_id"],
            "properties": {
                "project_path": { "type": "string" },
                "drawer_id": { "type": "integer" },
                "relation": { "type": "string", "description": "Filter by relation type" },
                "direction": { "type": "string", "enum": ["from","to","both"], "description": "Default: both" }
            }
        })),
        tool_def("set_validity", "Update the validity window of a KG relation.", json!({
            "type": "object",
            "required": ["relation_id"],
            "properties": {
                "project_path": { "type": "string" },
                "relation_id": { "type": "integer" },
                "valid_from": { "type": "string" },
                "valid_until": { "type": "string" }
            }
        })),
        tool_def("get_schema", "Return the active schema taxonomy.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" },
                "name": { "type": "string", "description": "Schema name (default: general)" }
            }
        })),
        tool_def("list_schemas", "List available schema names.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string" }
            }
        })),
        tool_def("setup_claude", "Generate the .claude/settings.json MCP entry for yourmemory.", json!({
            "type": "object",
            "properties": {
                "project_path": { "type": "string", "description": "Project to scope the MCP server to" }
            }
        })),
    ];
    json!({ "tools": tools })
}

fn tool_def(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

async fn handle_tools_call(params: &Value) -> anyhow::Result<Value> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "wakeup" => tool_wakeup(&args).await,
        "search" => tool_search(&args).await,
        "recall" => tool_recall(&args).await,
        "get_drawer" => tool_get_drawer(&args).await,
        "persist" => tool_persist(&args).await,
        "store_fact" => tool_store_fact(&args).await,
        "update_fact" => tool_update_fact(&args).await,
        "invalidate_fact" => tool_invalidate_fact(&args).await,
        "create_wing" => tool_create_wing(&args).await,
        "create_room" => tool_create_room(&args).await,
        "list_wings" => tool_list_wings(&args).await,
        "list_rooms" => tool_list_rooms(&args).await,
        "health" => tool_health(&args).await,
        "compact" => tool_compact(&args).await,
        "forget" => tool_forget(&args).await,
        "export" => tool_export(&args).await,
        "add_relation" => tool_add_relation(&args).await,
        "query_relations" => tool_query_relations(&args).await,
        "set_validity" => tool_set_validity(&args).await,
        "get_schema" => tool_get_schema(&args).await,
        "list_schemas" => tool_list_schemas(&args).await,
        "setup_claude" => tool_setup_claude(&args),
        _ => Err(anyhow::anyhow!("unknown tool: {}", name)),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_path(args: &Value) -> PathBuf {
    args.get("project_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn open_palace(args: &Value) -> anyhow::Result<Palace> {
    let path = resolve_path(args);
    Palace::open(&path).map_err(|e| anyhow::anyhow!("{}", e))
}

fn text_result(s: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": s.into() }] })
}

fn json_result(v: &Value) -> Value {
    text_result(serde_json::to_string_pretty(v).unwrap_or_default())
}

fn parse_source(s: Option<&str>) -> Source {
    match s {
        Some("config") => Source::Config,
        Some("system") => Source::System,
        Some("user") => Source::User,
        _ => Source::Conversation,
    }
}

fn parse_confidence(s: Option<&str>) -> Confidence {
    match s {
        Some("high") => Confidence::High,
        Some("low") => Confidence::Low,
        Some("inferred") => Confidence::Inferred,
        _ => Confidence::Medium,
    }
}

async fn compress_content(content: &str, project_path: &PathBuf) -> String {
    let config_path = project_path.join("config.toml");
    let toml = std::fs::read_to_string(&config_path).unwrap_or_default();
    let cfg = CompressionConfig::from_toml(&toml).unwrap_or_default();
    let compressor = build_compressor(&cfg);
    compressor.compress(content).await.unwrap_or_else(|_| content.to_string())
}

fn find_wing_by_name<'a>(
    wings: &'a [yourmemory_core::storage::Wing],
    name: &str,
) -> Option<&'a yourmemory_core::storage::Wing> {
    wings.iter().find(|w| w.name == name)
}

// ── Tool implementations ──────────────────────────────────────────────────────

async fn tool_wakeup(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let budget_tokens = args.get("token_budget").and_then(|v| v.as_u64()).unwrap_or(4000) as usize;
    let ctx = budget::build_wakeup(&palace.storage, budget_tokens)?;

    let mut response = json!({
        "wings": ctx.l0_wings,
        "rooms": ctx.l1_rooms,
        "drawers": ctx.l2_drawers,
        "tokens_used": ctx.tokens_used,
        "budget_tokens": ctx.budget_tokens,
        "palace_path": palace.root.to_string_lossy()
    });

    if let Ok(wings) = palace.storage.list_wings() {
        if let Some(events_wing) = find_wing_by_name(&wings, "events") {
            if let Ok(rooms) = palace.storage.list_rooms(events_wing.id) {
                let mut all_drawers = Vec::new();
                for room in &rooms {
                    if let Ok(mut drawers) = palace.storage.get_drawers_by_room(room.id, usize::MAX) {
                        all_drawers.append(&mut drawers);
                    }
                }
                all_drawers.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                all_drawers.truncate(10);
                if !all_drawers.is_empty() {
                    response["recent_events"] = json!(all_drawers);
                }
            }
        }
    }

    Ok(json_result(&response))
}

async fn tool_search(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("query is required"))?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let drawers = palace.storage.search_drawers(query, limit)?;
    Ok(json_result(&json!(drawers)))
}

async fn tool_recall(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wing_name = args
        .get("wing")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("wing is required"))?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let wings = palace.storage.list_wings()?;
    let wing = find_wing_by_name(&wings, wing_name)
        .ok_or_else(|| anyhow::anyhow!("wing '{}' not found", wing_name))?;

    let drawers = if let Some(room_name) = args.get("room").and_then(|v| v.as_str()) {
        let rooms = palace.storage.list_rooms(wing.id)?;
        let room = rooms
            .iter()
            .find(|r| r.name == room_name)
            .ok_or_else(|| anyhow::anyhow!("room '{}' not found in wing '{}'", room_name, wing_name))?;
        palace.storage.get_drawers_by_room(room.id, limit)?
    } else {
        let rooms = palace.storage.list_rooms(wing.id)?;
        let mut all = Vec::new();
        for room in &rooms {
            let mut d = palace.storage.get_drawers_by_room(room.id, limit)?;
            all.append(&mut d);
            if all.len() >= limit {
                break;
            }
        }
        all.truncate(limit);
        all
    };

    Ok(json_result(&json!(drawers)))
}

async fn tool_get_drawer(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let id = args
        .get("drawer_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("drawer_id is required"))?;
    match palace.storage.get_drawer(id)? {
        Some(d) => Ok(json_result(&json!(d))),
        None => Ok(text_result(format!("drawer {} not found", id))),
    }
}

async fn tool_persist(args: &Value) -> anyhow::Result<Value> {
    let project_path = resolve_path(args);
    let palace = Palace::open(&project_path).map_err(|e| anyhow::anyhow!("{}", e))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("content is required"))?;
    let wing_name = args.get("wing").and_then(|v| v.as_str()).unwrap_or("general");
    let room_name = args.get("room").and_then(|v| v.as_str()).unwrap_or("notes");

    let compressed = compress_content(content, &project_path).await;
    let compressed_content = if compressed != content { Some(compressed) } else { None };

    let wing = palace.storage.create_wing(wing_name, None)?;
    let room = palace.storage.create_room(wing.id, room_name, None)?;
    let drawer = palace.storage.store_drawer(&NewDrawer {
        wing_id: wing.id,
        room_id: room.id,
        content: content.to_string(),
        compressed_content,
        confidence: parse_confidence(args.get("confidence").and_then(|v| v.as_str())),
        source: parse_source(args.get("source").and_then(|v| v.as_str())),
    })?;
    Ok(json_result(&json!(drawer)))
}

async fn tool_store_fact(args: &Value) -> anyhow::Result<Value> {
    let project_path = resolve_path(args);
    let palace = Palace::open(&project_path).map_err(|e| anyhow::anyhow!("{}", e))?;

    let wing_name = args.get("wing").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("wing is required"))?;
    let room_name = args.get("room").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("room is required"))?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("content is required"))?;

    let compressed = compress_content(content, &project_path).await;
    let compressed_content = if compressed != content { Some(compressed) } else { None };

    let wing = palace.storage.create_wing(wing_name, None)?;
    let room = palace.storage.create_room(wing.id, room_name, None)?;
    let drawer = palace.storage.store_drawer(&NewDrawer {
        wing_id: wing.id,
        room_id: room.id,
        content: content.to_string(),
        compressed_content,
        confidence: parse_confidence(args.get("confidence").and_then(|v| v.as_str())),
        source: parse_source(args.get("source").and_then(|v| v.as_str())),
    })?;
    Ok(json_result(&json!(drawer)))
}

async fn tool_update_fact(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let id = args.get("drawer_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("drawer_id is required"))?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("content is required"))?;
    palace.storage.update_drawer(id, content)?;
    Ok(text_result(format!("drawer {} updated", id)))
}

async fn tool_invalidate_fact(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let id = args.get("drawer_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("drawer_id is required"))?;
    palace.storage.invalidate_fact(id)?;
    Ok(text_result(format!("drawer {} invalidated", id)))
}

async fn tool_create_wing(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("name is required"))?;
    let desc = args.get("description").and_then(|v| v.as_str());
    let wing = palace.storage.create_wing(name, desc)?;
    Ok(json_result(&json!(wing)))
}

async fn tool_create_room(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wing_name = args.get("wing").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("wing is required"))?;
    let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("name is required"))?;
    let summary = args.get("summary").and_then(|v| v.as_str());

    let wings = palace.storage.list_wings()?;
    let wing = find_wing_by_name(&wings, wing_name)
        .ok_or_else(|| anyhow::anyhow!("wing '{}' not found", wing_name))?;
    let room = palace.storage.create_room(wing.id, name, summary)?;
    Ok(json_result(&json!(room)))
}

async fn tool_list_wings(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wings = palace.storage.list_wings()?;
    Ok(json_result(&json!(wings)))
}

async fn tool_list_rooms(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wing_name = args.get("wing").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("wing is required"))?;
    let wings = palace.storage.list_wings()?;
    let wing = find_wing_by_name(&wings, wing_name)
        .ok_or_else(|| anyhow::anyhow!("wing '{}' not found", wing_name))?;
    let rooms = palace.storage.list_rooms(wing.id)?;
    Ok(json_result(&json!(rooms)))
}

async fn tool_health(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wings = palace.storage.list_wings()?;
    let wing_count = wings.len();
    let mut room_count = 0usize;
    for wing in &wings {
        room_count += palace.storage.list_rooms(wing.id)?.len();
    }
    let drawer_count = palace.storage.drawer_count()?;
    let db_path = palace.root.join("palace.db");
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    Ok(json_result(&json!({
        "palace_path": palace.root.to_string_lossy(),
        "wings": wing_count,
        "rooms": room_count,
        "drawers": drawer_count,
        "db_size_bytes": db_size,
        "status": "ok"
    })))
}

async fn tool_compact(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let max_age_days = args.get("max_age_days").and_then(|v| v.as_i64()).unwrap_or(90);
    let access_threshold = 1i64;
    let wings = palace.storage.list_wings()?;
    let mut purged = 0i64;
    for wing in &wings {
        let rooms = palace.storage.list_rooms(wing.id)?;
        for room in &rooms {
            let drawers = palace.storage.get_drawers_by_room(room.id, usize::MAX)?;
            for drawer in drawers {
                if palace.storage.compact_drawer(drawer.id, access_threshold, max_age_days)? {
                    purged += 1;
                }
            }
        }
    }
    let drawer_count = palace.storage.drawer_count()?;
    Ok(json_result(&json!({ "purged": purged, "remaining": drawer_count })))
}

async fn tool_forget(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wing_name = args.get("wing").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("wing is required"))?;

    let wings = palace.storage.list_wings()?;
    let wing = find_wing_by_name(&wings, wing_name)
        .ok_or_else(|| anyhow::anyhow!("wing '{}' not found", wing_name))?;

    let deleted = if let Some(room_name) = args.get("room").and_then(|v| v.as_str()) {
        let rooms = palace.storage.list_rooms(wing.id)?;
        let room = rooms
            .iter()
            .find(|r| r.name == room_name)
            .ok_or_else(|| anyhow::anyhow!("room '{}' not found", room_name))?;
        palace.storage.forget_room(room.id)?
    } else {
        palace.storage.forget_wing(wing.id)?
    };

    Ok(json_result(&json!({ "deleted": deleted })))
}

async fn tool_export(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let wings = palace.storage.list_wings()?;
    let mut export = Vec::new();
    for wing in &wings {
        let rooms = palace.storage.list_rooms(wing.id)?;
        let mut wing_data = json!({ "id": wing.id, "name": wing.name, "rooms": [] });
        let mut rooms_data = Vec::new();
        for room in &rooms {
            let drawers = palace.storage.get_drawers_by_room(room.id, usize::MAX)?;
            rooms_data.push(json!({ "id": room.id, "name": room.name, "drawers": drawers }));
        }
        wing_data["rooms"] = json!(rooms_data);
        export.push(wing_data);
    }
    Ok(json_result(&json!({ "palace": export })))
}

async fn tool_add_relation(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let from_id = args.get("from_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("from_id is required"))?;
    let to_id = args.get("to_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("to_id is required"))?;
    let relation = args.get("relation").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("relation is required"))?;
    let valid_from = args.get("valid_from").and_then(|v| v.as_str());
    let valid_until = args.get("valid_until").and_then(|v| v.as_str());
    let id = palace.storage.add_relation(from_id, to_id, relation, valid_from, valid_until)?;
    Ok(json_result(&json!({ "id": id })))
}

async fn tool_query_relations(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let drawer_id = args.get("drawer_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("drawer_id is required"))?;
    let relation_filter = args.get("relation").and_then(|v| v.as_str());
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("both");

    let mut relations = palace.storage.query_relations(drawer_id, None)?;

    if let Some(rel) = relation_filter {
        relations.retain(|r| r.relation_type == rel);
    }
    match direction {
        "from" => relations.retain(|r| r.source_drawer_id == drawer_id),
        "to"   => relations.retain(|r| r.target_drawer_id == drawer_id),
        _      => {}
    }

    Ok(json_result(&json!({ "relations": relations })))
}

async fn tool_set_validity(args: &Value) -> anyhow::Result<Value> {
    let palace = open_palace(args)?;
    let relation_id = args.get("relation_id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("relation_id is required"))?;
    let valid_from = args.get("valid_from").and_then(|v| v.as_str());
    let valid_until = args.get("valid_until").and_then(|v| v.as_str());
    palace.storage.set_validity(relation_id, valid_from, valid_until)?;
    Ok(json_result(&json!({ "relation_id": relation_id, "updated": true })))
}

async fn tool_get_schema(args: &Value) -> anyhow::Result<Value> {
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("general");
    let project_path = resolve_path(args);
    let schema_path = project_path.join("schemas").join(format!("{}.toml", name));
    let content = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|_| format!("# {} schema\n[taxonomy]\n", name));
    Ok(json_result(&json!({ "name": name, "content": content })))
}

async fn tool_list_schemas(args: &Value) -> anyhow::Result<Value> {
    let project_path = resolve_path(args);
    let schemas_dir = project_path.join("schemas");
    let schemas: Vec<String> = std::fs::read_dir(&schemas_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("toml") {
                        p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json_result(&json!({ "schemas": schemas })))
}

fn tool_setup_claude(args: &Value) -> anyhow::Result<Value> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "yourmemory-mcp".to_string());

    let project_path = args.get("project_path").and_then(|v| v.as_str());

    let server_entry = if let Some(path) = project_path {
        json!({ "type": "stdio", "command": exe, "args": ["--project", path] })
    } else {
        json!({ "type": "stdio", "command": exe, "args": [] })
    };

    let config = json!({ "mcpServers": { "yourmemory": server_entry } });

    Ok(text_result(format!(
        "Add this to your .claude/settings.json:\n\n{}",
        serde_json::to_string_pretty(&config)?
    )))
}
