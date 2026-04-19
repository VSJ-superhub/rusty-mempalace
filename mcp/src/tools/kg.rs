use anyhow::Result;
use serde_json::Value;

pub async fn add_relation(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok", "id": null}))
}

pub async fn query_relations(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"relations": []}))
}

pub async fn set_validity(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"status": "ok"}))
}
