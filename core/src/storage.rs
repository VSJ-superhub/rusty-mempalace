use std::path::{Path, PathBuf};
use chrono::Utc;
use rusqlite::{Connection, Result as SqlResult, params};
use thiserror::Error;
use crate::access::{
    generate_token_secret, hash_secret, AccessScope, AccessToken, Grant, GrantLevel,
};
use crate::palace::sanitize_fts_query;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Palace not found: no .yourmemory/ directory in path or home")]
    NotFound,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

// ── Domain enums ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
    Inferred,
}

impl Confidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
            Confidence::Inferred => "inferred",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "high" => Confidence::High,
            "medium" => Confidence::Medium,
            "low" => Confidence::Low,
            _ => Confidence::Inferred,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Conversation,
    Config,
    System,
    User,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Conversation => "conversation",
            Source::Config => "config",
            Source::System => "system",
            Source::User => "user",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "config" => Source::Config,
            "system" => Source::System,
            "user" => Source::User,
            _ => Source::Conversation,
        }
    }
}

// ── Structs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Wing {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: i64,
    pub wing_id: i64,
    pub name: String,
    pub summary: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Drawer {
    pub id: i64,
    pub wing_id: i64,
    pub room_id: i64,
    pub content: String,
    pub compressed_content: Option<String>,
    pub confidence: Confidence,
    pub source: Source,
    pub access_count: i64,
    pub last_accessed_at: Option<String>,
    pub created_at: String,
    pub is_invalidated: bool,
    pub invalidated_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewDrawer {
    pub wing_id: i64,
    pub room_id: i64,
    pub content: String,
    pub compressed_content: Option<String>,
    pub confidence: Confidence,
    pub source: Source,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgEdge {
    pub id: i64,
    pub subject_drawer_id: i64,
    pub predicate: String,
    pub object_drawer_id: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

/// A temporal relation between two drawers in the knowledge graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Relation {
    pub id: i64,
    pub source_drawer_id: i64,
    pub target_drawer_id: i64,
    pub relation_type: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

/// One write-ahead-log entry, surfaced for the Audit Log view. `wing` is resolved
/// where the operation can be attributed to one (drawer/room/wing ops); entries that
/// cannot be attributed (e.g. token ops) are visible only to a global admin.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub operation: String,
    pub table_name: String,
    pub record_id: i64,
    pub wing: Option<String>,
    pub preview: String,
    pub created_at: String,
}

/// Aggregated per-room statistics for the Confidence Heatmap.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomStat {
    pub wing: String,
    pub room: String,
    pub room_id: i64,
    pub drawer_count: i64,
    /// Mean confidence in [0,1]; high=1.0, medium=0.66, low=0.33, inferred=0.1.
    pub avg_confidence: f64,
    pub last_write: Option<String>,
    pub last_read: Option<String>,
}

/// A single retrieval-probe hit: the drawer plus the wing/room it lives in, so the
/// UI can show *where* a result came from. `rank` is FTS5's relevance score (lower =
/// better match); it lets the probe display the same ordering the engine used.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub drawer: Drawer,
    pub wing: String,
    pub room: String,
    pub rank: f64,
}

/// A burst of write activity, reconstructed from the WAL by clustering adjacent
/// entries whose time gap is below a threshold. There are no stored "sessions" — a
/// session is just a window of the audit trail, so this needs no new storage and
/// stays consistent with the audit log. `entries` is a capped sample (newest first)
/// so the UI can show *what* changed without paging the whole window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriteSession {
    pub started_at: String,
    pub ended_at: String,
    pub total_changes: i64,
    /// Count per operation (e.g. INSERT/UPDATE/INVALIDATE), descending.
    pub op_counts: Vec<LabelCount>,
    /// Count per wing touched in this window, descending.
    pub wing_counts: Vec<LabelCount>,
    pub entries: Vec<AuditEntry>,
    /// True if `entries` was truncated (more changes than the cap).
    pub truncated: bool,
}

/// A `(label, count)` pair, used for per-op and per-wing rollups in a session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LabelCount {
    pub label: String,
    pub count: i64,
}

/// Palace-level counts for the Admin view. `db_size_bytes` is filled by the server
/// (it owns the file path); storage supplies the rest.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PalaceStats {
    pub total_drawers: i64,
    pub total_relations: i64,
    pub total_wings: i64,
    pub last_compact: Option<String>,
}

/// A node in the knowledge-graph view. `id` is namespaced (`wing:1`, `room:2`, `drawer:3`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub wing: Option<String>,
    pub confidence: Option<String>,
}

/// An edge in the knowledge-graph view. `kind` is `structural` (wing→room→drawer) or
/// `relation` (a KG relation between drawers). Expired relations are kept but flagged.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub label: String,
    pub kind: String,
    pub expired: bool,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// True if the drawer cap was hit and the graph is a partial view.
    pub truncated: bool,
}

// ── Storage trait ─────────────────────────────────────────────────────────────

pub trait Storage {
    fn create_wing(&self, name: &str, description: Option<&str>) -> Result<Wing>;
    fn create_room(&self, wing_id: i64, name: &str, summary: Option<&str>) -> Result<Room>;
    fn store_drawer(&self, drawer: &NewDrawer) -> Result<Drawer>;
    fn get_drawer(&self, id: i64) -> Result<Option<Drawer>>;
    fn list_wings(&self) -> Result<Vec<Wing>>;
    fn list_rooms(&self, wing_id: i64) -> Result<Vec<Room>>;
    fn search_drawers(&self, query: &str, limit: usize) -> Result<Vec<Drawer>>;
    fn update_drawer(&self, id: i64, content: &str) -> Result<()>;
    fn invalidate_drawer(&self, id: i64) -> Result<()>;
    fn get_recent_drawers(&self, limit: usize) -> Result<Vec<Drawer>>;

    // KG edges (legacy API kept for backward compat)
    fn add_kg_edge(
        &self,
        subject_drawer_id: i64,
        predicate: &str,
        object_drawer_id: i64,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<KgEdge>;
    fn get_kg_edges(&self, drawer_id: i64) -> Result<Vec<KgEdge>>;

    // Knowledge Graph — temporal relations
    fn add_relation(
        &self,
        source_drawer_id: i64,
        target_drawer_id: i64,
        relation_type: &str,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<Relation>;
    /// Returns relations for `drawer_id`. If `at_time` is set (RFC3339), only
    /// relations whose validity window contains that instant are returned.
    fn query_relations(&self, drawer_id: i64, at_time: Option<&str>) -> Result<Vec<Relation>>;
    fn set_validity(
        &self,
        relation_id: i64,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<()>;

    fn drawer_count(&self) -> Result<i64>;
    fn get_drawers_by_room(&self, room_id: i64, limit: usize) -> Result<Vec<Drawer>>;

    // Structural deletion
    fn forget_room(&self, room_id: i64) -> Result<i64>;
    fn forget_wing(&self, wing_id: i64) -> Result<i64>;

    // Forgetting subsystem
    /// Mark a drawer as invalidated, recording the timestamp. Invalidated
    /// drawers are excluded from normal reads but can be retrieved explicitly.
    fn invalidate_fact(&self, id: i64) -> Result<()>;
    /// Replace content with compressed_content (or a placeholder) when the
    /// drawer has been accessed fewer than `access_threshold` times and its
    /// last-access (or creation) time is older than `ttl_days`. Returns true
    /// if the drawer was compacted.
    fn compact_drawer(&self, id: i64, access_threshold: i64, ttl_days: i64) -> Result<bool>;
    /// Return drawers that have been invalidated (excluded from normal reads).
    fn get_invalidated_drawers(&self, limit: usize) -> Result<Vec<Drawer>>;
}

// ── SqliteStorage ─────────────────────────────────────────────────────────────

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let storage = SqliteStorage { conn };
        storage.migrate()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = SqliteStorage { conn };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS wings (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL UNIQUE,
                description TEXT,
                created_at  TEXT    NOT NULL
            );

            CREATE TABLE IF NOT EXISTS rooms (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                wing_id     INTEGER NOT NULL REFERENCES wings(id),
                name        TEXT    NOT NULL,
                summary     TEXT,
                created_at  TEXT    NOT NULL,
                UNIQUE(wing_id, name)
            );

            CREATE TABLE IF NOT EXISTS drawers (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                wing_id             INTEGER NOT NULL REFERENCES wings(id),
                room_id             INTEGER NOT NULL REFERENCES rooms(id),
                content             TEXT    NOT NULL,
                compressed_content  TEXT,
                confidence          TEXT    NOT NULL DEFAULT 'medium',
                source              TEXT    NOT NULL DEFAULT 'conversation',
                access_count        INTEGER NOT NULL DEFAULT 0,
                last_accessed_at    TEXT,
                created_at          TEXT    NOT NULL,
                is_invalidated      INTEGER NOT NULL DEFAULT 0,
                invalidated_at      TEXT
            );

            CREATE INDEX IF NOT EXISTS drawers_wing_room ON drawers(wing_id, room_id);
            CREATE INDEX IF NOT EXISTS drawers_created   ON drawers(created_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS drawers_fts USING fts5(
                content,
                content='drawers',
                content_rowid='id'
            );

            CREATE TRIGGER IF NOT EXISTS drawers_fts_insert
            AFTER INSERT ON drawers BEGIN
                INSERT INTO drawers_fts(rowid, content) VALUES (new.id, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS drawers_fts_update
            AFTER UPDATE ON drawers BEGIN
                INSERT INTO drawers_fts(drawers_fts, rowid, content) VALUES ('delete', old.id, old.content);
                INSERT INTO drawers_fts(rowid, content) VALUES (new.id, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS drawers_fts_delete
            AFTER DELETE ON drawers BEGIN
                INSERT INTO drawers_fts(drawers_fts, rowid, content) VALUES ('delete', old.id, old.content);
            END;

            CREATE TABLE IF NOT EXISTS knowledge_graph_edges (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                subject_drawer_id INTEGER NOT NULL REFERENCES drawers(id),
                predicate         TEXT    NOT NULL,
                object_drawer_id  INTEGER NOT NULL REFERENCES drawers(id),
                valid_from        TEXT,
                valid_until       TEXT
            );

            CREATE TABLE IF NOT EXISTS relations (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                source_drawer_id  INTEGER NOT NULL REFERENCES drawers(id),
                target_drawer_id  INTEGER NOT NULL REFERENCES drawers(id),
                relation_type     TEXT    NOT NULL,
                valid_from        TEXT,
                valid_until       TEXT
            );

            CREATE INDEX IF NOT EXISTS relations_source ON relations(source_drawer_id);
            CREATE INDEX IF NOT EXISTS relations_target ON relations(target_drawer_id);

            CREATE TABLE IF NOT EXISTS write_ahead_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                operation   TEXT    NOT NULL,
                table_name  TEXT    NOT NULL,
                record_id   INTEGER NOT NULL,
                payload     TEXT    NOT NULL,
                created_at  TEXT    NOT NULL
            );

            -- Access control for the network server. Secrets are stored hashed,
            -- never plaintext. Revoke is soft (revoked_at) to preserve audit history.
            CREATE TABLE IF NOT EXISTS access_tokens (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                label        TEXT    NOT NULL,
                secret_hash  TEXT    NOT NULL UNIQUE,
                created_at   TEXT    NOT NULL,
                last_used_at TEXT,
                revoked_at   TEXT
            );

            CREATE TABLE IF NOT EXISTS token_grants (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                token_id INTEGER NOT NULL REFERENCES access_tokens(id),
                wing     TEXT    NOT NULL,
                level    TEXT    NOT NULL,
                UNIQUE(token_id, wing)
            );

            CREATE INDEX IF NOT EXISTS token_grants_token ON token_grants(token_id);
            ",
        )?;

        // Additive migration: add invalidated_at if this db predates it.
        let res = self.conn.execute_batch(
            "ALTER TABLE drawers ADD COLUMN invalidated_at TEXT;",
        );
        if let Err(e) = res {
            if !e.to_string().contains("duplicate column name") {
                return Err(StorageError::Sqlite(e));
            }
        }

        Ok(())
    }

    fn wal_append(&self, operation: &str, table_name: &str, record_id: i64, payload: &str) -> SqlResult<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO write_ahead_log (operation, table_name, record_id, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![operation, table_name, record_id, payload, now],
        )?;
        Ok(())
    }
}

// ── Access control: token store + scoping ───────────────────────────────────────
//
// Lives on SqliteStorage because it touches the private connection. The pure
// types and scope logic are in `crate::access`.

impl SqliteStorage {
    /// Create a token with the given grants and return `(stored_token, plaintext_secret)`.
    /// The secret is shown to the operator exactly once — only its hash is persisted.
    pub fn create_access_token(
        &self,
        label: &str,
        grants: &[Grant],
    ) -> Result<(AccessToken, String)> {
        let secret = generate_token_secret();
        let hash = hash_secret(&secret);
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO access_tokens (label, secret_hash, created_at) VALUES (?1, ?2, ?3)",
            params![label, hash, now],
        )?;
        let token_id = self.conn.last_insert_rowid();
        for g in grants {
            self.set_grant(token_id, &g.wing, g.level)?;
        }
        self.wal_append("INSERT", "access_tokens", token_id, label)?;
        let token = self.get_access_token(token_id)?.expect("just inserted");
        Ok((token, secret))
    }

    /// Add or update a single wing grant for a token.
    pub fn set_grant(&self, token_id: i64, wing: &str, level: GrantLevel) -> Result<()> {
        self.conn.execute(
            "INSERT INTO token_grants (token_id, wing, level) VALUES (?1, ?2, ?3)
             ON CONFLICT(token_id, wing) DO UPDATE SET level = excluded.level",
            params![token_id, wing, level.as_str()],
        )?;
        self.wal_append("GRANT", "token_grants", token_id, &format!("{wing}:{}", level.as_str()))?;
        Ok(())
    }

    /// Soft-revoke a token. Returns false if no such (un-revoked) token exists.
    pub fn revoke_access_token(&self, token_id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let n = self.conn.execute(
            "UPDATE access_tokens SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
            params![now, token_id],
        )?;
        if n > 0 {
            self.wal_append("REVOKE", "access_tokens", token_id, "")?;
        }
        Ok(n > 0)
    }

    pub fn get_access_token(&self, token_id: i64) -> Result<Option<AccessToken>> {
        let res = self.conn.query_row(
            "SELECT id, label, created_at, last_used_at, revoked_at FROM access_tokens WHERE id = ?1",
            params![token_id],
            row_to_access_token,
        );
        match res {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    pub fn list_access_tokens(&self) -> Result<Vec<AccessToken>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, created_at, last_used_at, revoked_at FROM access_tokens ORDER BY id",
        )?;
        let rows = stmt.query_map([], row_to_access_token)?.collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_grants(&self, token_id: i64) -> Result<Vec<Grant>> {
        let mut stmt = self
            .conn
            .prepare("SELECT wing, level FROM token_grants WHERE token_id = ?1 ORDER BY wing")?;
        let rows = stmt
            .query_map(params![token_id], |r| {
                let wing: String = r.get(0)?;
                let level: String = r.get(1)?;
                Ok((wing, level))
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(wing, level)| GrantLevel::parse(&level).map(|level| Grant { wing, level }))
            .collect())
    }

    /// Number of tokens that could currently authenticate a request. Used to decide
    /// whether auth must be enforced and whether non-loopback binding is permitted.
    pub fn active_token_count(&self) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM access_tokens WHERE revoked_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Resolve a presented bearer secret into an `AccessScope`. Returns `None` if the
    /// secret matches no active token. Bumps `last_used_at` on success.
    pub fn resolve_scope(&self, secret: &str) -> Result<Option<AccessScope>> {
        let hash = hash_secret(secret);
        let res = self.conn.query_row(
            "SELECT id, label FROM access_tokens WHERE secret_hash = ?1 AND revoked_at IS NULL",
            params![hash],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        );
        let (token_id, label) = match res {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(StorageError::Sqlite(e)),
        };
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE access_tokens SET last_used_at = ?1 WHERE id = ?2",
            params![now, token_id],
        )?;
        let grants = self.list_grants(token_id)?;
        Ok(Some(AccessScope::new(token_id, label, grants)))
    }

    /// Write an auth failure event to the WAL for auditing. Never panics.
    pub fn log_auth_failure(&self, remote_addr: &str) -> Result<()> {
        self.wal_append("AUTH_FAIL", "access_tokens", 0, remote_addr)
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    // ── id → wing-name resolution (for scoping by resource id) ──────────────────

    pub fn wing_name(&self, wing_id: i64) -> Result<Option<String>> {
        opt(self.conn.query_row(
            "SELECT name FROM wings WHERE id = ?1",
            params![wing_id],
            |r| r.get::<_, String>(0),
        ))
    }

    pub fn room_wing_name(&self, room_id: i64) -> Result<Option<String>> {
        opt(self.conn.query_row(
            "SELECT w.name FROM rooms r JOIN wings w ON w.id = r.wing_id WHERE r.id = ?1",
            params![room_id],
            |r| r.get::<_, String>(0),
        ))
    }

    pub fn drawer_wing_name(&self, drawer_id: i64) -> Result<Option<String>> {
        opt(self.conn.query_row(
            "SELECT w.name FROM drawers d JOIN wings w ON w.id = d.wing_id WHERE d.id = ?1",
            params![drawer_id],
            |r| r.get::<_, String>(0),
        ))
    }

    // ── Scoped reads: visibility decided here, never in a handler ────────────────

    /// Wings the scope may read.
    pub fn list_wings_scoped(&self, scope: &AccessScope) -> Result<Vec<Wing>> {
        Ok(self
            .list_wings()?
            .into_iter()
            .filter(|w| scope.can_read(&w.name))
            .collect())
    }

    /// Rooms in `wing_id`, or `None` if the scope cannot read that wing (handler → 404/403).
    pub fn list_rooms_scoped(&self, scope: &AccessScope, wing_id: i64) -> Result<Option<Vec<Room>>> {
        match self.wing_name(wing_id)? {
            Some(name) if scope.can_read(&name) => Ok(Some(self.list_rooms(wing_id)?)),
            _ => Ok(None),
        }
    }

    /// Paginated drawers in `room_id`, or `None` if the scope cannot read the room's wing.
    pub fn drawers_by_room_scoped(
        &self,
        scope: &AccessScope,
        room_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Option<Vec<Drawer>>> {
        match self.room_wing_name(room_id)? {
            Some(name) if scope.can_read(&name) => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT {DRAWER_SELECT} FROM drawers \
                     WHERE room_id = ?1 AND is_invalidated = 0 \
                     ORDER BY created_at DESC LIMIT ?2 OFFSET ?3"
                ))?;
                let rows = stmt
                    .query_map(params![room_id, limit as i64, offset as i64], row_to_drawer)?
                    .collect::<SqlResult<Vec<_>>>()?;
                Ok(Some(rows))
            }
            _ => Ok(None),
        }
    }

    /// A single drawer, but only if the scope can read its wing. `None` covers both
    /// "does not exist" and "out of scope" so existence is not leaked.
    pub fn get_drawer_scoped(&self, scope: &AccessScope, id: i64) -> Result<Option<Drawer>> {
        match self.drawer_wing_name(id)? {
            Some(name) if scope.can_read(&name) => self.get_drawer(id),
            _ => Ok(None),
        }
    }

    /// Run the real FTS retrieval engine, then drop hits the scope cannot read. The
    /// join carries each hit's wing/room name (which raw `search_drawers` lacks) so
    /// scoping happens here, not in the handler, and the UI can show provenance.
    /// `limit` bounds rows *before* scoping, so a token may see fewer than `limit`.
    pub fn search_drawers_scoped(
        &self,
        scope: &AccessScope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let safe_query = sanitize_fts_query(query);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {}, w.name, r.name, f.rank \
             FROM drawers d \
             JOIN drawers_fts f ON f.rowid = d.id \
             JOIN wings w ON w.id = d.wing_id \
             JOIN rooms r ON r.id = d.room_id \
             WHERE drawers_fts MATCH ?1 AND d.is_invalidated = 0 \
             ORDER BY f.rank \
             LIMIT ?2",
            DRAWER_SELECT
                .split(", ")
                .map(|c| format!("d.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        ))?;
        let rows = stmt
            .query_map(params![safe_query, limit as i64], |r| {
                let drawer = row_to_drawer(r)?;
                let n = DRAWER_SELECT.split(", ").count();
                Ok(SearchHit {
                    drawer,
                    wing: r.get(n)?,
                    room: r.get(n + 1)?,
                    rank: r.get(n + 2)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|h| scope.can_read(&h.wing)).collect())
    }
}

fn opt<T>(res: SqlResult<T>) -> Result<Option<T>> {
    match res {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e)),
    }
}

fn row_to_access_token(row: &rusqlite::Row) -> SqlResult<AccessToken> {
    Ok(AccessToken {
        id: row.get(0)?,
        label: row.get(1)?,
        created_at: row.get(2)?,
        last_used_at: row.get(3)?,
        revoked_at: row.get(4)?,
    })
}

/// SQL expression mapping a drawer's confidence to a [0,1] score. Used by the heatmap.
const CONF_SCORE: &str =
    "CASE confidence WHEN 'high' THEN 1.0 WHEN 'medium' THEN 0.66 \
     WHEN 'low' THEN 0.33 ELSE 0.1 END";

// ── Scoped views for the dashboard: graph, audit, heatmap, stats ─────────────────

impl SqliteStorage {
    /// Audit-log entries, newest first, scoped to what `scope` may see. Optional filters:
    /// `op` (operation), `from`/`to` (RFC3339 created_at bounds), `wing` (exact wing name).
    /// Entries that cannot be attributed to a wing are returned only to a global admin.
    pub fn audit_scoped(
        &self,
        scope: &AccessScope,
        op: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        wing: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT wal.id, wal.operation, wal.table_name, wal.record_id, wal.payload, wal.created_at, \
                    COALESCE(dw.name, rw.name, ww.name) AS wing_name \
             FROM write_ahead_log wal \
             LEFT JOIN drawers d ON wal.table_name='drawers' AND d.id=wal.record_id \
             LEFT JOIN wings dw ON dw.id=d.wing_id \
             LEFT JOIN rooms  r ON wal.table_name='rooms'   AND r.id=wal.record_id \
             LEFT JOIN wings rw ON rw.id=r.wing_id \
             LEFT JOIN wings ww ON wal.table_name='wings'   AND ww.id=wal.record_id \
             WHERE (?1 IS NULL OR wal.operation = ?1) \
               AND (?2 IS NULL OR wal.created_at >= ?2) \
               AND (?3 IS NULL OR wal.created_at <= ?3) \
             ORDER BY wal.created_at DESC \
             LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(params![op, from, to, limit as i64], |r| {
                let payload: String = r.get(4)?;
                let preview: String = payload.chars().take(80).collect();
                Ok(AuditEntry {
                    id: r.get(0)?,
                    operation: r.get(1)?,
                    table_name: r.get(2)?,
                    record_id: r.get(3)?,
                    preview,
                    created_at: r.get(5)?,
                    wing: r.get(6)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;

        Ok(rows
            .into_iter()
            .filter(|e| match &e.wing {
                Some(w) => scope.can_read(w) && wing.map_or(true, |f| f == w),
                // Unattributed entry: global admin only, and not when filtering by a wing.
                None => scope.is_global_admin() && wing.is_none(),
            })
            .collect())
    }

    /// Group the scoped audit trail into write sessions: clusters of data changes
    /// separated by an idle gap of more than `gap_minutes`. Reuses [`Self::audit_scoped`]
    /// so scoping/attribution is identical to the Audit Log — token/auth ops (no wing)
    /// are excluded here; this view is about *content* changes. `max_sessions` bounds
    /// how many recent sessions are returned; `entry_cap` bounds entries kept per session.
    pub fn sessions_scoped(
        &self,
        scope: &AccessScope,
        gap_minutes: i64,
        max_sessions: usize,
        entry_cap: usize,
    ) -> Result<Vec<WriteSession>> {
        // Pull a generous slice of the trail (newest first) and keep only data ops we
        // can attribute to a wing — those are the changes a "diff between sessions" is about.
        let raw = self.audit_scoped(scope, None, None, None, None, 5000)?;
        let entries: Vec<AuditEntry> = raw
            .into_iter()
            .filter(|e| {
                e.wing.is_some()
                    && matches!(e.table_name.as_str(), "drawers" | "rooms" | "wings")
            })
            .collect();

        let gap = chrono::Duration::minutes(gap_minutes.max(1));
        let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();

        let mut sessions: Vec<WriteSession> = Vec::new();
        let mut bucket: Vec<AuditEntry> = Vec::new();
        // `entries` is newest-first; a new session starts when the previous (newer) entry
        // is more than `gap` after the current one.
        for e in entries {
            if let (Some(last), Some(cur)) = (bucket.last().and_then(|l| parse(&l.created_at)), parse(&e.created_at)) {
                if last.signed_duration_since(cur) > gap {
                    sessions.push(Self::roll_up_session(std::mem::take(&mut bucket), entry_cap));
                    if sessions.len() >= max_sessions {
                        return Ok(sessions);
                    }
                }
            }
            bucket.push(e);
        }
        if !bucket.is_empty() && sessions.len() < max_sessions {
            sessions.push(Self::roll_up_session(bucket, entry_cap));
        }
        Ok(sessions)
    }

    /// Collapse one cluster of newest-first audit entries into a [`WriteSession`].
    fn roll_up_session(entries: Vec<AuditEntry>, entry_cap: usize) -> WriteSession {
        let total = entries.len() as i64;
        let started_at = entries.last().map(|e| e.created_at.clone()).unwrap_or_default();
        let ended_at = entries.first().map(|e| e.created_at.clone()).unwrap_or_default();
        let op_counts = Self::tally(entries.iter().map(|e| e.operation.clone()));
        let wing_counts =
            Self::tally(entries.iter().filter_map(|e| e.wing.clone()));
        let truncated = entries.len() > entry_cap;
        let sample = entries.into_iter().take(entry_cap).collect();
        WriteSession {
            started_at,
            ended_at,
            total_changes: total,
            op_counts,
            wing_counts,
            entries: sample,
            truncated,
        }
    }

    /// Tally labels into `(label, count)` pairs sorted by count desc, then label asc.
    fn tally(labels: impl Iterator<Item = String>) -> Vec<LabelCount> {
        let mut map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for l in labels {
            *map.entry(l).or_insert(0) += 1;
        }
        let mut out: Vec<LabelCount> =
            map.into_iter().map(|(label, count)| LabelCount { label, count }).collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
        out
    }

    /// Per-room aggregates for the heatmap, scoped to readable wings.
    pub fn heatmap_scoped(&self, scope: &AccessScope) -> Result<Vec<RoomStat>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT w.name, r.name, r.id, \
                    COUNT(d.id), \
                    COALESCE(AVG({CONF_SCORE}), 0.0), \
                    MAX(d.created_at), MAX(d.last_accessed_at) \
             FROM rooms r \
             JOIN wings w ON w.id = r.wing_id \
             LEFT JOIN drawers d ON d.room_id = r.id AND d.is_invalidated = 0 \
             GROUP BY r.id \
             ORDER BY w.name, r.name"
        ))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RoomStat {
                    wing: r.get(0)?,
                    room: r.get(1)?,
                    room_id: r.get(2)?,
                    drawer_count: r.get(3)?,
                    avg_confidence: r.get(4)?,
                    last_write: r.get(5)?,
                    last_read: r.get(6)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|s| scope.can_read(&s.wing)).collect())
    }

    /// Palace-level counts for the Admin view (db size is added by the server).
    pub fn palace_stats(&self) -> Result<PalaceStats> {
        let total_drawers: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM drawers WHERE is_invalidated = 0", [], |r| r.get(0))?;
        let total_relations: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM relations", [], |r| r.get(0))?;
        let total_wings: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM wings", [], |r| r.get(0))?;
        let last_compact: Option<String> = self.conn.query_row(
            "SELECT MAX(created_at) FROM write_ahead_log WHERE operation = 'COMPACT'",
            [], |r| r.get::<_, Option<String>>(0)).optional_flat()?;
        Ok(PalaceStats { total_drawers, total_relations, total_wings, last_compact })
    }

    /// Build the knowledge-graph view scoped to readable wings. Structural edges connect
    /// wing→room→drawer; relation edges connect drawers (expired ones flagged, not dropped).
    /// Capped at `max_drawers` to keep a large palace from overwhelming the client.
    pub fn graph_scoped(&self, scope: &AccessScope, max_drawers: usize) -> Result<GraphData> {
        let now = Utc::now().to_rfc3339();
        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut included: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut truncated = false;

        'wings: for wing in self.list_wings_scoped(scope)? {
            nodes.push(GraphNode {
                id: format!("wing:{}", wing.id),
                kind: "wing".into(),
                label: wing.name.clone(),
                wing: Some(wing.name.clone()),
                confidence: None,
            });
            for room in self.list_rooms(wing.id)? {
                nodes.push(GraphNode {
                    id: format!("room:{}", room.id),
                    kind: "room".into(),
                    label: room.name.clone(),
                    wing: Some(wing.name.clone()),
                    confidence: None,
                });
                edges.push(GraphEdge {
                    source: format!("wing:{}", wing.id),
                    target: format!("room:{}", room.id),
                    label: "contains".into(),
                    kind: "structural".into(),
                    expired: false,
                    valid_from: None,
                    valid_until: None,
                });
                for d in self.get_drawers_by_room(room.id, usize::MAX)? {
                    if included.len() >= max_drawers {
                        truncated = true;
                        break 'wings;
                    }
                    included.insert(d.id);
                    nodes.push(GraphNode {
                        id: format!("drawer:{}", d.id),
                        kind: "drawer".into(),
                        label: d.content.chars().take(48).collect(),
                        wing: Some(wing.name.clone()),
                        confidence: Some(d.confidence.as_str().to_string()),
                    });
                    edges.push(GraphEdge {
                        source: format!("room:{}", room.id),
                        target: format!("drawer:{}", d.id),
                        label: "contains".into(),
                        kind: "structural".into(),
                        expired: false,
                        valid_from: None,
                        valid_until: None,
                    });
                }
            }
        }

        // Relation edges between included drawers (dedup by relation id).
        let mut seen_rel: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for &id in &included {
            for rel in self.query_relations(id, None)? {
                if !seen_rel.insert(rel.id) { continue; }
                if !included.contains(&rel.source_drawer_id) || !included.contains(&rel.target_drawer_id) {
                    continue;
                }
                let expired = rel.valid_until.as_deref().map_or(false, |u| u < now.as_str());
                edges.push(GraphEdge {
                    source: format!("drawer:{}", rel.source_drawer_id),
                    target: format!("drawer:{}", rel.target_drawer_id),
                    label: rel.relation_type,
                    kind: "relation".into(),
                    expired,
                    valid_from: rel.valid_from,
                    valid_until: rel.valid_until,
                });
            }
        }

        Ok(GraphData { nodes, edges, truncated })
    }
}

/// Small helper: turn a single-row query that may return no rows into `Option`.
trait OptionalFlat<T> {
    fn optional_flat(self) -> Result<Option<T>>;
}
impl<T> OptionalFlat<T> for SqlResult<Option<T>> {
    fn optional_flat(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }
}

fn row_to_wing(row: &rusqlite::Row) -> SqlResult<Wing> {
    Ok(Wing {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn row_to_room(row: &rusqlite::Row) -> SqlResult<Room> {
    Ok(Room {
        id: row.get(0)?,
        wing_id: row.get(1)?,
        name: row.get(2)?,
        summary: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn row_to_drawer(row: &rusqlite::Row) -> SqlResult<Drawer> {
    let is_invalidated: i64 = row.get(10)?;
    Ok(Drawer {
        id: row.get(0)?,
        wing_id: row.get(1)?,
        room_id: row.get(2)?,
        content: row.get(3)?,
        compressed_content: row.get(4)?,
        confidence: Confidence::from_str(&row.get::<_, String>(5)?),
        source: Source::from_str(&row.get::<_, String>(6)?),
        access_count: row.get(7)?,
        last_accessed_at: row.get(8)?,
        created_at: row.get(9)?,
        is_invalidated: is_invalidated != 0,
        invalidated_at: row.get(11)?,
    })
}

fn row_to_kg_edge(row: &rusqlite::Row) -> SqlResult<KgEdge> {
    Ok(KgEdge {
        id: row.get(0)?,
        subject_drawer_id: row.get(1)?,
        predicate: row.get(2)?,
        object_drawer_id: row.get(3)?,
        valid_from: row.get(4)?,
        valid_until: row.get(5)?,
    })
}

fn row_to_relation(row: &rusqlite::Row) -> SqlResult<Relation> {
    Ok(Relation {
        id: row.get(0)?,
        source_drawer_id: row.get(1)?,
        target_drawer_id: row.get(2)?,
        relation_type: row.get(3)?,
        valid_from: row.get(4)?,
        valid_until: row.get(5)?,
    })
}

/// Selects all columns for a drawer row; column order must match `row_to_drawer`.
const DRAWER_SELECT: &str =
    "id, wing_id, room_id, content, compressed_content, confidence, source, \
     access_count, last_accessed_at, created_at, is_invalidated, invalidated_at";

impl Storage for SqliteStorage {
    fn create_wing(&self, name: &str, description: Option<&str>) -> Result<Wing> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO wings (name, description, created_at) VALUES (?1, ?2, ?3)",
            params![name, description, now],
        )?;
        let wing = self.conn.query_row(
            "SELECT id, name, description, created_at FROM wings WHERE name = ?1",
            params![name],
            row_to_wing,
        )?;
        self.wal_append("INSERT", "wings", wing.id, name)?;
        Ok(wing)
    }

    fn create_room(&self, wing_id: i64, name: &str, summary: Option<&str>) -> Result<Room> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO rooms (wing_id, name, summary, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![wing_id, name, summary, now],
        )?;
        let room = self.conn.query_row(
            "SELECT id, wing_id, name, summary, created_at FROM rooms WHERE wing_id = ?1 AND name = ?2",
            params![wing_id, name],
            row_to_room,
        )?;
        self.wal_append("INSERT", "rooms", room.id, name)?;
        Ok(room)
    }

    fn store_drawer(&self, d: &NewDrawer) -> Result<Drawer> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO drawers
             (wing_id, room_id, content, compressed_content, confidence, source, access_count, created_at, is_invalidated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, 0)",
            params![
                d.wing_id,
                d.room_id,
                d.content,
                d.compressed_content,
                d.confidence.as_str(),
                d.source.as_str(),
                now,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.wal_append("INSERT", "drawers", id, &d.content)?;
        let drawer = self.conn.query_row(
            &format!("SELECT {DRAWER_SELECT} FROM drawers WHERE id = ?1"),
            params![id],
            row_to_drawer,
        )?;
        Ok(drawer)
    }

    fn get_drawer(&self, id: i64) -> Result<Option<Drawer>> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE drawers SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        let result = self.conn.query_row(
            &format!("SELECT {DRAWER_SELECT} FROM drawers WHERE id = ?1"),
            params![id],
            row_to_drawer,
        );
        match result {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn list_wings(&self) -> Result<Vec<Wing>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, created_at FROM wings ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_wing)?.collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn list_rooms(&self, wing_id: i64) -> Result<Vec<Room>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, wing_id, name, summary, created_at FROM rooms WHERE wing_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![wing_id], row_to_room)?.collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn search_drawers(&self, query: &str, limit: usize) -> Result<Vec<Drawer>> {
        let safe_query = sanitize_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT d.id, d.wing_id, d.room_id, d.content, d.compressed_content, \
                    d.confidence, d.source, d.access_count, d.last_accessed_at, \
                    d.created_at, d.is_invalidated, d.invalidated_at \
             FROM drawers d \
             JOIN drawers_fts f ON f.rowid = d.id \
             WHERE drawers_fts MATCH ?1 AND d.is_invalidated = 0 \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![safe_query, limit as i64], row_to_drawer)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn update_drawer(&self, id: i64, content: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE drawers SET content = ?1 WHERE id = ?2",
            params![content, id],
        )?;
        self.wal_append("UPDATE", "drawers", id, content)?;
        Ok(())
    }

    fn invalidate_drawer(&self, id: i64) -> Result<()> {
        self.invalidate_fact(id)
    }

    fn get_recent_drawers(&self, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE is_invalidated = 0 \
             ORDER BY created_at DESC \
             LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_drawer)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn add_kg_edge(
        &self,
        subject_drawer_id: i64,
        predicate: &str,
        object_drawer_id: i64,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<KgEdge> {
        self.conn.execute(
            "INSERT INTO knowledge_graph_edges
             (subject_drawer_id, predicate, object_drawer_id, valid_from, valid_until)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![subject_drawer_id, predicate, object_drawer_id, valid_from, valid_until],
        )?;
        let id = self.conn.last_insert_rowid();
        let edge = self.conn.query_row(
            "SELECT id, subject_drawer_id, predicate, object_drawer_id, valid_from, valid_until
             FROM knowledge_graph_edges WHERE id = ?1",
            params![id],
            row_to_kg_edge,
        )?;
        Ok(edge)
    }

    fn get_kg_edges(&self, drawer_id: i64) -> Result<Vec<KgEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, subject_drawer_id, predicate, object_drawer_id, valid_from, valid_until
             FROM knowledge_graph_edges
             WHERE subject_drawer_id = ?1 OR object_drawer_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![drawer_id], row_to_kg_edge)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Knowledge Graph — temporal relations ──────────────────────────────────

    fn add_relation(
        &self,
        source_drawer_id: i64,
        target_drawer_id: i64,
        relation_type: &str,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<Relation> {
        self.conn.execute(
            "INSERT INTO relations
             (source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until],
        )?;
        let id = self.conn.last_insert_rowid();
        let relation = self.conn.query_row(
            "SELECT id, source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until
             FROM relations WHERE id = ?1",
            params![id],
            row_to_relation,
        )?;
        Ok(relation)
    }

    fn query_relations(&self, drawer_id: i64, at_time: Option<&str>) -> Result<Vec<Relation>> {
        if let Some(t) = at_time {
            let mut stmt = self.conn.prepare(
                "SELECT id, source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until
                 FROM relations
                 WHERE (source_drawer_id = ?1 OR target_drawer_id = ?1)
                   AND (valid_from  IS NULL OR valid_from  <= ?2)
                   AND (valid_until IS NULL OR valid_until >= ?2)",
            )?;
            let rows = stmt.query_map(params![drawer_id, t], row_to_relation)?
                .collect::<SqlResult<Vec<_>>>()?;
            Ok(rows)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until
                 FROM relations
                 WHERE source_drawer_id = ?1 OR target_drawer_id = ?1",
            )?;
            let rows = stmt.query_map(params![drawer_id], row_to_relation)?
                .collect::<SqlResult<Vec<_>>>()?;
            Ok(rows)
        }
    }

    fn set_validity(
        &self,
        relation_id: i64,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE relations SET valid_from = ?1, valid_until = ?2 WHERE id = ?3",
            params![valid_from, valid_until, relation_id],
        )?;
        Ok(())
    }

    fn drawer_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM drawers WHERE is_invalidated = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    fn get_drawers_by_room(&self, room_id: i64, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE room_id = ?1 AND is_invalidated = 0 \
             ORDER BY created_at DESC \
             LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![room_id, limit as i64], row_to_drawer)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn forget_room(&self, room_id: i64) -> Result<i64> {
        let deleted = self.conn.execute(
            "DELETE FROM drawers WHERE room_id = ?1",
            params![room_id],
        )? as i64;
        self.conn.execute("DELETE FROM rooms WHERE id = ?1", params![room_id])?;
        Ok(deleted)
    }

    fn forget_wing(&self, wing_id: i64) -> Result<i64> {
        let deleted = self.conn.execute(
            "DELETE FROM drawers WHERE wing_id = ?1",
            params![wing_id],
        )? as i64;
        self.conn.execute("DELETE FROM rooms WHERE wing_id = ?1", params![wing_id])?;
        self.conn.execute("DELETE FROM wings WHERE id = ?1", params![wing_id])?;
        Ok(deleted)
    }

    // ── Forgetting subsystem ──────────────────────────────────────────────────

    fn invalidate_fact(&self, id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE drawers SET is_invalidated = 1, invalidated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        self.wal_append("INVALIDATE", "drawers", id, "")?;
        Ok(())
    }

    fn compact_drawer(&self, id: i64, access_threshold: i64, ttl_days: i64) -> Result<bool> {
        // Read without bumping access_count.
        let result = self.conn.query_row(
            &format!("SELECT {DRAWER_SELECT} FROM drawers WHERE id = ?1 AND is_invalidated = 0"),
            params![id],
            row_to_drawer,
        );
        let drawer = match result {
            Ok(d) => d,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
            Err(e) => return Err(StorageError::Sqlite(e)),
        };

        if drawer.access_count >= access_threshold {
            return Ok(false);
        }

        let now = Utc::now();
        let cutoff = now - chrono::Duration::days(ttl_days);

        let reference_str = drawer
            .last_accessed_at
            .as_deref()
            .unwrap_or(&drawer.created_at);

        let reference_time = chrono::DateTime::parse_from_rfc3339(reference_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(now);

        if reference_time > cutoff {
            return Ok(false);
        }

        let new_content = drawer
            .compressed_content
            .clone()
            .unwrap_or_else(|| "[compacted]".to_string());

        self.conn.execute(
            "UPDATE drawers SET content = ?1, compressed_content = ?2 WHERE id = ?3",
            params![new_content, new_content, id],
        )?;
        self.wal_append("COMPACT", "drawers", id, &new_content)?;
        Ok(true)
    }

    fn get_invalidated_drawers(&self, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE is_invalidated = 1 \
             ORDER BY invalidated_at DESC \
             LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_drawer)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }
}

// ── Palace ────────────────────────────────────────────────────────────────────

pub struct Palace {
    pub storage: SqliteStorage,
    pub root: PathBuf,
}

impl Palace {
    /// Walk up from `path` looking for a `.yourmemory/` directory.
    /// Falls back to `~/.yourmemory/global/` if none found.
    pub fn open(path: &Path) -> Result<Self> {
        let palace_dir = find_palace_dir(path)?;
        std::fs::create_dir_all(&palace_dir)?;
        let db_path = palace_dir.join("palace.db");
        let storage = SqliteStorage::open(&db_path)?;
        Ok(Palace { storage, root: palace_dir })
    }

    /// Open the palace whose `palace.db` lives directly in `palace_dir`, with no
    /// walk-up or fallback. If `palace_dir` points at a `palace.db` file, its parent
    /// directory is used. Created if it does not exist.
    pub fn open_at(palace_dir: &Path) -> Result<Self> {
        let dir = if palace_dir.is_file()
            || palace_dir.extension().map_or(false, |e| e == "db")
        {
            palace_dir.parent().unwrap_or(palace_dir).to_path_buf()
        } else {
            palace_dir.to_path_buf()
        };
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("palace.db");
        let storage = SqliteStorage::open(&db_path)?;
        Ok(Palace { storage, root: dir })
    }
}

fn find_palace_dir(start: &Path) -> Result<PathBuf> {
    // Opt-in anchor: when YOURMEMORY_PALACE is set, it overrides walk-up resolution
    // so every call targets one canonical palace dir (the dir containing palace.db),
    // regardless of cwd or the project_path passed. Unset = original behavior.
    if let Ok(anchor) = std::env::var("YOURMEMORY_PALACE") {
        if !anchor.trim().is_empty() {
            return Ok(PathBuf::from(anchor));
        }
    }

    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(".yourmemory");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !current.pop() {
            break;
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    Ok(PathBuf::from(home).join(".yourmemory").join("global"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open_temp_storage() -> SqliteStorage {
        let dir = tempdir().unwrap();
        SqliteStorage::open(&dir.into_path().join("test.db")).unwrap()
    }

    fn make_drawer(s: &SqliteStorage) -> Drawer {
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        s.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "test content".to_string(),
            compressed_content: None,
            confidence: Confidence::Medium,
            source: Source::User,
        }).unwrap()
    }

    // ── existing tests ────────────────────────────────────────────────────────

    #[test]
    fn test_create_wing_and_room() {
        let s = open_temp_storage();
        let wing = s.create_wing("project", Some("main project wing")).unwrap();
        assert_eq!(wing.name, "project");

        let room = s.create_room(wing.id, "auth", Some("authentication")).unwrap();
        assert_eq!(room.name, "auth");
        assert_eq!(room.wing_id, wing.id);
    }

    #[test]
    fn test_store_and_get_drawer() {
        let s = open_temp_storage();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();

        let nd = NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "The API uses JWT tokens".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        };
        let stored = s.store_drawer(&nd).unwrap();
        assert_eq!(stored.content, "The API uses JWT tokens");
        assert_eq!(stored.confidence, Confidence::High);
        assert!(!stored.is_invalidated);
        assert!(stored.invalidated_at.is_none());

        let fetched = s.get_drawer(stored.id).unwrap().unwrap();
        assert_eq!(fetched.id, stored.id);
        assert_eq!(fetched.access_count, 1);
    }

    #[test]
    fn test_list_wings_and_rooms() {
        let s = open_temp_storage();
        s.create_wing("alpha", None).unwrap();
        s.create_wing("beta", None).unwrap();
        let wings = s.list_wings().unwrap();
        assert_eq!(wings.len(), 2);

        let rooms = s.list_rooms(wings[0].id).unwrap();
        assert_eq!(rooms.len(), 0);
    }

    #[test]
    fn test_invalidate_drawer() {
        let s = open_temp_storage();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        let d = s.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "old fact".to_string(),
            compressed_content: None,
            confidence: Confidence::Low,
            source: Source::Conversation,
        }).unwrap();

        s.invalidate_drawer(d.id).unwrap();
        let recent = s.get_recent_drawers(10).unwrap();
        assert!(recent.iter().all(|dr| dr.id != d.id));
    }

    #[test]
    fn test_search_drawers() {
        let s = open_temp_storage();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        s.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "authentication uses bearer tokens".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        }).unwrap();
        s.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "database stores user preferences".to_string(),
            compressed_content: None,
            confidence: Confidence::Medium,
            source: Source::System,
        }).unwrap();

        let results = s.search_drawers("authentication", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("authentication"));
    }

    #[test]
    fn test_kg_edges() {
        let s = open_temp_storage();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        let d1 = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "user entity".to_string(), compressed_content: None,
            confidence: Confidence::High, source: Source::System,
        }).unwrap();
        let d2 = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "order entity".to_string(), compressed_content: None,
            confidence: Confidence::High, source: Source::System,
        }).unwrap();

        let edge = s.add_kg_edge(d1.id, "has_many", d2.id, None, None).unwrap();
        assert_eq!(edge.predicate, "has_many");

        let edges = s.get_kg_edges(d1.id).unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn test_palace_open_fallback() {
        let dir = tempdir().unwrap();
        let palace = Palace::open(dir.path()).unwrap();
        assert!(palace.root.to_string_lossy().contains(".yourmemory"));
    }

    #[test]
    fn test_palace_open_local() {
        let dir = tempdir().unwrap();
        let palace_dir = dir.path().join(".yourmemory");
        std::fs::create_dir_all(&palace_dir).unwrap();
        let palace = Palace::open(dir.path()).unwrap();
        assert_eq!(palace.root, palace_dir);
    }

    // ── Knowledge Graph — relations ───────────────────────────────────────────

    #[test]
    fn test_add_and_query_relations_no_filter() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d1 = make_drawer(&s);
        let wing = s.create_wing("w2", None).unwrap();
        let room = s.create_room(wing.id, "r2", None).unwrap();
        let d2 = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "target".to_string(), compressed_content: None,
            confidence: Confidence::Low, source: Source::System,
        }).unwrap();

        let rel = s.add_relation(d1.id, d2.id, "causes", None, None).unwrap();
        assert_eq!(rel.relation_type, "causes");
        assert_eq!(rel.source_drawer_id, d1.id);
        assert_eq!(rel.target_drawer_id, d2.id);

        let rels = s.query_relations(d1.id, None).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].id, rel.id);

        // Also queryable from the target side.
        let rels_target = s.query_relations(d2.id, None).unwrap();
        assert_eq!(rels_target.len(), 1);
    }

    #[test]
    fn test_query_relations_temporal_filter() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d1 = make_drawer(&s);
        let wing = s.create_wing("w2", None).unwrap();
        let room = s.create_room(wing.id, "r2", None).unwrap();
        let d2 = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "b".to_string(), compressed_content: None,
            confidence: Confidence::Low, source: Source::System,
        }).unwrap();

        // Relation valid only in 2023.
        s.add_relation(
            d1.id, d2.id, "was_deployed_to",
            Some("2023-01-01T00:00:00Z"),
            Some("2023-12-31T23:59:59Z"),
        ).unwrap();

        // A time inside the window should find it.
        let inside = s.query_relations(d1.id, Some("2023-06-01T00:00:00Z")).unwrap();
        assert_eq!(inside.len(), 1);

        // A time outside the window should not.
        let outside = s.query_relations(d1.id, Some("2024-01-01T00:00:00Z")).unwrap();
        assert_eq!(outside.len(), 0);

        // No filter returns it regardless.
        let all = s.query_relations(d1.id, None).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_set_validity() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d1 = make_drawer(&s);
        let wing = s.create_wing("w2", None).unwrap();
        let room = s.create_room(wing.id, "r2", None).unwrap();
        let d2 = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "b".to_string(), compressed_content: None,
            confidence: Confidence::Low, source: Source::System,
        }).unwrap();

        let rel = s.add_relation(d1.id, d2.id, "linked", None, None).unwrap();

        // Set a validity window that excludes 2025.
        s.set_validity(
            rel.id,
            Some("2020-01-01T00:00:00Z"),
            Some("2022-12-31T23:59:59Z"),
        ).unwrap();

        let found = s.query_relations(d1.id, Some("2021-06-01T00:00:00Z")).unwrap();
        assert_eq!(found.len(), 1);

        let not_found = s.query_relations(d1.id, Some("2025-01-01T00:00:00Z")).unwrap();
        assert_eq!(not_found.len(), 0);
    }

    // ── Forgetting subsystem ──────────────────────────────────────────────────

    #[test]
    fn test_invalidate_fact_excluded_from_normal_reads() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d = make_drawer(&s);

        s.invalidate_fact(d.id).unwrap();

        // Does not appear in recent drawers.
        let recent = s.get_recent_drawers(10).unwrap();
        assert!(recent.iter().all(|dr| dr.id != d.id));

        // Does not appear in search results.
        let results = s.search_drawers("test", 10).unwrap();
        assert!(results.iter().all(|dr| dr.id != d.id));
    }

    #[test]
    fn test_invalidate_fact_queryable_with_flag() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d = make_drawer(&s);

        s.invalidate_fact(d.id).unwrap();

        // Appears in get_invalidated_drawers.
        let invalidated = s.get_invalidated_drawers(10).unwrap();
        assert!(invalidated.iter().any(|dr| dr.id == d.id));

        // The invalidated drawer has is_invalidated = true and a timestamp.
        let inv = invalidated.iter().find(|dr| dr.id == d.id).unwrap();
        assert!(inv.is_invalidated);
        assert!(inv.invalidated_at.is_some());
    }

    #[test]
    fn test_compact_drawer_replaces_content_with_compressed() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        let d = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "verbose original content".to_string(),
            compressed_content: Some("summary".to_string()),
            confidence: Confidence::Low,
            source: Source::Conversation,
        }).unwrap();

        // Threshold 1, TTL 0 days — should compact immediately (access_count=0, age>=0).
        let compacted = s.compact_drawer(d.id, 1, 0).unwrap();
        assert!(compacted);

        let fetched = s.get_drawer(d.id).unwrap().unwrap();
        assert_eq!(fetched.content, "summary");
    }

    #[test]
    fn test_compact_drawer_uses_placeholder_when_no_compressed() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        let d = s.store_drawer(&NewDrawer {
            wing_id: wing.id, room_id: room.id,
            content: "verbose content without compressed version".to_string(),
            compressed_content: None,
            confidence: Confidence::Low,
            source: Source::Conversation,
        }).unwrap();

        let compacted = s.compact_drawer(d.id, 1, 0).unwrap();
        assert!(compacted);

        let fetched = s.get_drawer(d.id).unwrap().unwrap();
        assert_eq!(fetched.content, "[compacted]");
    }

    #[test]
    fn test_compact_drawer_skips_frequently_accessed() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d = make_drawer(&s);

        // Access it 5 times.
        for _ in 0..5 {
            s.get_drawer(d.id).unwrap();
        }

        // Threshold 3 — should NOT compact because access_count (5) >= threshold (3).
        let compacted = s.compact_drawer(d.id, 3, 0).unwrap();
        assert!(!compacted);
    }

    #[test]
    fn test_compact_drawer_skips_recent() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let d = make_drawer(&s);

        // TTL 365 days — should not compact a drawer created just now.
        let compacted = s.compact_drawer(d.id, 100, 365).unwrap();
        assert!(!compacted);
    }

    // ── Access control ────────────────────────────────────────────────────────

    /// Build two wings, each with a room + drawer, and a token scoped to only one.
    /// Returns (storage, scope_for_engineering, legal_room_id, legal_drawer_id).
    fn setup_two_wings() -> (SqliteStorage, AccessScope, i64, i64) {
        let s = SqliteStorage::open_in_memory().unwrap();
        let eng = s.create_wing("engineering", None).unwrap();
        let eng_room = s.create_room(eng.id, "auth", None).unwrap();
        s.store_drawer(&NewDrawer {
            wing_id: eng.id, room_id: eng_room.id,
            content: "eng secret".into(), compressed_content: None,
            confidence: Confidence::High, source: Source::User,
        }).unwrap();

        let legal = s.create_wing("legal", None).unwrap();
        let legal_room = s.create_room(legal.id, "contracts", None).unwrap();
        let legal_drawer = s.store_drawer(&NewDrawer {
            wing_id: legal.id, room_id: legal_room.id,
            content: "legal secret".into(), compressed_content: None,
            confidence: Confidence::High, source: Source::User,
        }).unwrap();

        let (token, _secret) = s
            .create_access_token(
                "eng-only",
                &[Grant { wing: "engineering".into(), level: GrantLevel::Read }],
            )
            .unwrap();
        let scope = AccessScope::new(token.id, token.label, s.list_grants(token.id).unwrap());
        (s, scope, legal_room.id, legal_drawer.id)
    }

    #[test]
    fn token_scope_hides_other_wings_everywhere() {
        let (s, scope, legal_room_id, legal_drawer_id) = setup_two_wings();

        // list_wings: only engineering visible.
        let wings = s.list_wings_scoped(&scope).unwrap();
        assert_eq!(wings.len(), 1);
        assert_eq!(wings[0].name, "engineering");

        // rooms / drawers / drawer-by-id in the legal wing are all invisible (None).
        assert!(s.list_rooms_scoped(&scope, wings[0].id).unwrap().is_some());
        assert!(s.drawers_by_room_scoped(&scope, legal_room_id, 50, 0).unwrap().is_none());
        assert!(s.get_drawer_scoped(&scope, legal_drawer_id).unwrap().is_none());
    }

    #[test]
    fn resolve_scope_roundtrips_and_revoke_blocks() {
        let s = SqliteStorage::open_in_memory().unwrap();
        s.create_wing("engineering", None).unwrap();
        let (token, secret) = s
            .create_access_token(
                "t",
                &[Grant { wing: "engineering".into(), level: GrantLevel::Write }],
            )
            .unwrap();

        let scope = s.resolve_scope(&secret).unwrap().expect("valid secret resolves");
        assert!(scope.can_write("engineering"));
        assert!(!scope.can_read("legal"));

        // A wrong secret resolves to nothing.
        assert!(s.resolve_scope("deadbeef").unwrap().is_none());

        // After revoke the same secret no longer authenticates.
        assert!(s.revoke_access_token(token.id).unwrap());
        assert!(s.resolve_scope(&secret).unwrap().is_none());
        assert_eq!(s.active_token_count().unwrap(), 0);
    }
}
