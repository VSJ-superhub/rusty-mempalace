use anyhow::Result;
use serde_json::Value;

pub async fn get_schema(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"schema": null}))
}

pub async fn list_schemas(params: Value) -> Result<Value> {
    let _ = params;
    Ok(serde_json::json!({"schemas": []}))
}
