use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palace {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub schema: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wing {
    pub id: i64,
    pub palace_id: i64,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: i64,
    pub wing_id: i64,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Inferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    Conversation,
    Config,
    System,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Drawer {
    pub id: i64,
    pub room_id: i64,
    pub content: String,
    pub compressed_content: Option<String>,
    pub confidence: Confidence,
    pub source: Source,
    pub access_count: i64,
    pub last_accessed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub is_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgEdge {
    pub id: i64,
    pub subject_drawer_id: i64,
    pub predicate: String,
    pub object_drawer_id: i64,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}
