use anyhow::Result;
use serde_json::Value;

pub async fn health(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"wings": 0, "rooms": 0, "drawers": 0, "db_bytes": 0}))
}

pub async fn compact(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "removed": 0}))
}

pub async fn forget(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok"}))
}

pub async fn export(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"palace": {}}))
}
