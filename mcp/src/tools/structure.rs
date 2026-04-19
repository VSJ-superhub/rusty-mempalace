use anyhow::Result;
use serde_json::Value;

pub async fn create_wing(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "id": null}))
}

pub async fn create_room(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "id": null}))
}

pub async fn list_wings(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"wings": []}))
}

pub async fn list_rooms(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"rooms": []}))
}
