use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(flatten)]
    pub body: ResponseBody,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Ok  { result: Value },
    Err { error: RpcError },
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl Response {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Response { jsonrpc: "2.0".into(), id, body: ResponseBody::Ok { result } }
    }

    pub fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Response {
            jsonrpc: "2.0".into(),
            id,
            body: ResponseBody::Err {
                error: RpcError { code, message: message.into() },
            },
        }
    }
}

#[derive(Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    pub capabilities: Value,
}
