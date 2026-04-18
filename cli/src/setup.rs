use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Client {
    Claude,
    Gemini,
    Kiro,
}

/// Resolve the path to the `yourmemory-mcp` binary alongside the current exe.
fn mcp_binary_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(if cfg!(windows) {
                "yourmemory-mcp.exe"
            } else {
                "yourmemory-mcp"
            });
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "yourmemory-mcp".to_string()
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("cannot determine home directory")
}

/// Read existing JSON from `path`, or return an empty object if it doesn't exist.
fn read_json_or_empty(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &std::path::Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(value)?;
    std::fs::write(path, content)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

pub fn run(client: &Client) -> Result<()> {
    let binary = mcp_binary_path();

    match client {
        Client::Claude => setup_claude(&binary),
        Client::Gemini => setup_gemini(&binary),
        Client::Kiro => setup_kiro(&binary),
    }
}

fn setup_claude(binary: &str) -> Result<()> {
    let path = home()?.join(".claude").join("mcp_servers.json");
    let mut config = read_json_or_empty(&path);

    let entry = json!({
        "command": binary,
        "args": [],
        "env": {}
    });

    config
        .as_object_mut()
        .context("mcp_servers.json is not a JSON object")?
        .insert("yourmemory".to_string(), entry);

    write_json(&path, &config)?;
    println!("Claude MCP server registered at {}", path.display());
    Ok(())
}

fn setup_gemini(binary: &str) -> Result<()> {
    let path = home()?.join(".gemini").join("mcp.json");
    let mut config = read_json_or_empty(&path);

    let servers = config
        .as_object_mut()
        .context("mcp.json is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    servers
        .as_object_mut()
        .context("mcpServers is not a JSON object")?
        .insert(
            "yourmemory".to_string(),
            json!({ "command": binary, "args": [] }),
        );

    write_json(&path, &config)?;
    println!("Gemini MCP server registered at {}", path.display());
    Ok(())
}

fn setup_kiro(binary: &str) -> Result<()> {
    let path = home()?.join(".kiro").join("mcp.json");
    let mut config = read_json_or_empty(&path);

    let servers = config
        .as_object_mut()
        .context("mcp.json is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    servers
        .as_object_mut()
        .context("mcpServers is not a JSON object")?
        .insert(
            "yourmemory".to_string(),
            json!({ "command": binary, "args": [] }),
        );

    write_json(&path, &config)?;
    println!("Kiro MCP server registered at {}", path.display());
    Ok(())
}
