use anyhow::Result;
use serde_json::Value;

pub async fn wakeup(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "layers": []}))
}

pub async fn search(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"results": []}))
}

pub async fn recall(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"content": null}))
}

pub async fn get_drawer(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"drawer": null}))
}
