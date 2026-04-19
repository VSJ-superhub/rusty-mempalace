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

// ── Read tools ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WakeupParams {
    pub project: Option<String>,
    pub budget: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct WakeupResult {
    pub context: String,
    pub tokens_used: u32,
    pub layers_loaded: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub project: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub drawers: Vec<DrawerSummary>,
}

#[derive(Debug, Deserialize)]
pub struct RecallParams {
    pub wing: String,
    pub room: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecallResult {
    pub content: String,
    pub wing: String,
    pub room: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetDrawerParams {
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub struct GetDrawerResult {
    pub drawer: DrawerFull,
}

// ── Write tools ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PersistParams {
    pub content: String,
    pub project: Option<String>,
    pub wing: Option<String>,
    pub room: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct PersistResult {
    pub id: i64,
    pub wing: String,
    pub room: String,
    pub compressed: bool,
}

#[derive(Debug, Deserialize)]
pub struct StoreFactParams {
    pub content: String,
    pub wing: String,
    pub room: String,
    pub project: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct StoreFactResult {
    pub id: i64,
    pub compressed: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFactParams {
    pub id: i64,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct UpdateFactResult {
    pub id: i64,
    pub updated: bool,
}

#[derive(Debug, Deserialize)]
pub struct InvalidateFactParams {
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub struct InvalidateFactResult {
    pub id: i64,
    pub invalidated: bool,
}

// ── Structure tools ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateWingParams {
    pub name: String,
    pub project: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateWingResult {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoomParams {
    pub wing: String,
    pub name: String,
    pub project: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateRoomResult {
    pub id: i64,
    pub wing: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ListWingsParams {
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListWingsResult {
    pub wings: Vec<WingSummary>,
}

#[derive(Debug, Deserialize)]
pub struct ListRoomsParams {
    pub wing: String,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListRoomsResult {
    pub rooms: Vec<RoomSummary>,
}

// ── Maintenance tools ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HealthParams {
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthResult {
    pub wings: u32,
    pub rooms: u32,
    pub drawers: u32,
    pub db_size_bytes: u64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct CompactParams {
    pub project: Option<String>,
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct CompactResult {
    pub purged: u32,
    pub remaining: u32,
}

#[derive(Debug, Deserialize)]
pub struct ForgetParams {
    pub wing: String,
    pub room: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ForgetResult {
    pub deleted: u32,
}

#[derive(Debug, Deserialize)]
pub struct ExportParams {
    pub project: Option<String>,
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub json: Value,
}

// ── KG tools ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddRelationParams {
    pub from_id: i64,
    pub to_id: i64,
    pub relation: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddRelationResult {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct QueryRelationsParams {
    pub drawer_id: i64,
    pub relation: Option<String>,
    pub direction: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QueryRelationsResult {
    pub relations: Vec<RelationEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SetValidityParams {
    pub relation_id: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetValidityResult {
    pub relation_id: i64,
    pub updated: bool,
}

// ── Meta tools ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GetSchemaParams {
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetSchemaResult {
    pub name: String,
    pub taxonomy: Value,
}

#[derive(Debug, Deserialize)]
pub struct ListSchemasParams {}

#[derive(Debug, Serialize)]
pub struct ListSchemasResult {
    pub schemas: Vec<String>,
}

// ── Shared value types ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DrawerSummary {
    pub id: i64,
    pub wing: String,
    pub room: String,
    pub summary: String,
    pub score: f32,
    pub access_count: u32,
}

#[derive(Debug, Serialize)]
pub struct DrawerFull {
    pub id: i64,
    pub wing: String,
    pub room: String,
    pub content: String,
    pub compressed_content: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub access_count: u32,
    pub valid: bool,
}

#[derive(Debug, Serialize)]
pub struct WingSummary {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub room_count: u32,
}

#[derive(Debug, Serialize)]
pub struct RoomSummary {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub drawer_count: u32,
}

#[derive(Debug, Serialize)]
pub struct RelationEntry {
    pub id: i64,
    pub from_id: i64,
    pub to_id: i64,
    pub relation: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}
