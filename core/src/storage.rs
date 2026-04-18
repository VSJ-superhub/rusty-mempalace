use std::path::{Path, PathBuf};
use chrono::Utc;
use rusqlite::{Connection, Result as SqlResult, params};
use thiserror::Error;

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

#[derive(Debug, Clone)]
pub struct Wing {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: i64,
    pub wing_id: i64,
    pub name: String,
    pub summary: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct NewDrawer {
    pub wing_id: i64,
    pub room_id: i64,
    pub content: String,
    pub compressed_content: Option<String>,
    pub confidence: Confidence,
    pub source: Source,
}

#[derive(Debug, Clone)]
pub struct KgEdge {
    pub id: i64,
    pub subject_drawer_id: i64,
    pub predicate: String,
    pub object_drawer_id: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

/// A temporal relation between two drawers in the knowledge graph.
#[derive(Debug, Clone)]
pub struct Relation {
    pub id: i64,
    pub source_drawer_id: i64,
    pub target_drawer_id: i64,
    pub relation_type: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
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
            .query_map(params![query, limit as i64], row_to_drawer)?
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
}

fn find_palace_dir(start: &Path) -> Result<PathBuf> {
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
}
