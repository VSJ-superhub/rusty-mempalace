use anyhow::Result;
use serde_json::Value;

pub async fn persist(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "id": null}))
}

pub async fn store_fact(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "id": null}))
}

pub async fn update_fact(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok"}))
}

pub async fn invalidate_fact(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok"}))
}
