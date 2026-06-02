use std::path::{Path, PathBuf};
use chrono::Utc;
use rusqlite::params;

use crate::db;
use crate::storage::{
    Confidence, Drawer, KgEdge, NewDrawer, Relation, Result, Room, Source, StorageError, Wing,
};

const DRAWER_SELECT: &str =
    "id, wing_id, room_id, content, compressed_content, confidence, source, \
     access_count, last_accessed_at, created_at, is_invalidated, invalidated_at";

pub struct Palace {
    pub root: PathBuf,
    conn: rusqlite::Connection,
}

impl Palace {
    /// Walk up from `path` looking for a `.yourmemory/` directory.
    /// Falls back to `~/.yourmemory/global/` if none found.
    pub fn open(path: &Path) -> Result<Self> {
        let palace_dir = find_palace_dir(path)?;
        std::fs::create_dir_all(&palace_dir)?;
        let db_path = palace_dir.join("palace.db");
        let conn = db::open(&db_path).map_err(StorageError::Sqlite)?;
        Ok(Palace { root: palace_dir, conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = db::open_in_memory().map_err(StorageError::Sqlite)?;
        Ok(Palace {
            root: PathBuf::from(":memory:"),
            conn,
        })
    }

    pub fn create_wing(&self, name: &str, description: Option<&str>) -> Result<Wing> {
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
        db::wal_append(&self.conn, "INSERT", "wings", wing.id, name)?;
        Ok(wing)
    }

    pub fn create_room(&self, wing_id: i64, name: &str, summary: Option<&str>) -> Result<Room> {
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
        db::wal_append(&self.conn, "INSERT", "rooms", room.id, name)?;
        Ok(room)
    }

    pub fn store_drawer(&self, d: &NewDrawer) -> Result<Drawer> {
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
        db::wal_append(&self.conn, "INSERT", "drawers", id, &d.content)?;
        let drawer = self.conn.query_row(
            &format!("SELECT {DRAWER_SELECT} FROM drawers WHERE id = ?1"),
            params![id],
            row_to_drawer,
        )?;
        Ok(drawer)
    }

    pub fn get_drawer(&self, id: i64) -> Result<Option<Drawer>> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE drawers SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        match self.conn.query_row(
            &format!("SELECT {DRAWER_SELECT} FROM drawers WHERE id = ?1"),
            params![id],
            row_to_drawer,
        ) {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    pub fn list_wings(&self) -> Result<Vec<Wing>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, description, created_at FROM wings ORDER BY name")?;
        let rows = stmt
            .query_map([], row_to_wing)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_rooms(&self, wing_id: i64) -> Result<Vec<Room>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, wing_id, name, summary, created_at FROM rooms WHERE wing_id = ?1 ORDER BY name",
        )?;
        let rows = stmt
            .query_map(params![wing_id], row_to_room)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn search_drawers(&self, query: &str, limit: usize) -> Result<Vec<Drawer>> {
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
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn update_drawer(&self, id: i64, content: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE drawers SET content = ?1 WHERE id = ?2",
            params![content, id],
        )?;
        db::wal_append(&self.conn, "UPDATE", "drawers", id, content)?;
        Ok(())
    }

    pub fn invalidate_drawer(&self, id: i64) -> Result<()> {
        self.invalidate_fact(id)
    }

    pub fn invalidate_fact(&self, id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE drawers SET is_invalidated = 1, invalidated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        db::wal_append(&self.conn, "INVALIDATE", "drawers", id, "")?;
        Ok(())
    }

    pub fn get_recent_drawers(&self, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE is_invalidated = 0 \
             ORDER BY created_at DESC \
             LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_drawer)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_drawers_by_room(&self, room_id: i64, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE room_id = ?1 AND is_invalidated = 0 \
             ORDER BY created_at DESC \
             LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![room_id, limit as i64], row_to_drawer)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_invalidated_drawers(&self, limit: usize) -> Result<Vec<Drawer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRAWER_SELECT} FROM drawers \
             WHERE is_invalidated = 1 \
             ORDER BY invalidated_at DESC \
             LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_drawer)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Compact a drawer by replacing its content with the compressed version (or
    /// a placeholder) when it has been accessed fewer than `access_threshold`
    /// times and its last-access (or creation) time is older than `ttl_days`.
    /// Returns true if the drawer was compacted.
    pub fn compact_drawer(&self, id: i64, access_threshold: i64, ttl_days: i64) -> Result<bool> {
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
        db::wal_append(&self.conn, "COMPACT", "drawers", id, &new_content)?;
        Ok(true)
    }

    pub fn add_kg_edge(
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

    pub fn get_kg_edges(&self, drawer_id: i64) -> Result<Vec<KgEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, subject_drawer_id, predicate, object_drawer_id, valid_from, valid_until
             FROM knowledge_graph_edges
             WHERE subject_drawer_id = ?1 OR object_drawer_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![drawer_id], row_to_kg_edge)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_relation(
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

    pub fn query_relations(&self, drawer_id: i64, at_time: Option<&str>) -> Result<Vec<Relation>> {
        if let Some(t) = at_time {
            let mut stmt = self.conn.prepare(
                "SELECT id, source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until
                 FROM relations
                 WHERE (source_drawer_id = ?1 OR target_drawer_id = ?1)
                   AND (valid_from  IS NULL OR valid_from  <= ?2)
                   AND (valid_until IS NULL OR valid_until >= ?2)",
            )?;
            let rows = stmt
                .query_map(params![drawer_id, t], row_to_relation)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source_drawer_id, target_drawer_id, relation_type, valid_from, valid_until
                 FROM relations
                 WHERE source_drawer_id = ?1 OR target_drawer_id = ?1",
            )?;
            let rows = stmt
                .query_map(params![drawer_id], row_to_relation)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }

    pub fn set_validity(
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

    pub fn drawer_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM drawers WHERE is_invalidated = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(count)
    }
}

/// Strip FTS5 special characters that would cause a syntax error if passed verbatim.
pub fn sanitize_fts_query(query: &str) -> String {
    // Wrap each whitespace-separated token as a quoted phrase so FTS5
    // treats user input literally rather than as a query expression.
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if tokens.is_empty() {
        String::new()
    } else {
        tokens.join(" ")
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

fn row_to_wing(row: &rusqlite::Row) -> rusqlite::Result<Wing> {
    Ok(Wing {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn row_to_room(row: &rusqlite::Row) -> rusqlite::Result<Room> {
    Ok(Room {
        id: row.get(0)?,
        wing_id: row.get(1)?,
        name: row.get(2)?,
        summary: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn row_to_drawer(row: &rusqlite::Row) -> rusqlite::Result<Drawer> {
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

fn row_to_kg_edge(row: &rusqlite::Row) -> rusqlite::Result<KgEdge> {
    Ok(KgEdge {
        id: row.get(0)?,
        subject_drawer_id: row.get(1)?,
        predicate: row.get(2)?,
        object_drawer_id: row.get(3)?,
        valid_from: row.get(4)?,
        valid_until: row.get(5)?,
    })
}

fn row_to_relation(row: &rusqlite::Row) -> rusqlite::Result<Relation> {
    Ok(Relation {
        id: row.get(0)?,
        source_drawer_id: row.get(1)?,
        target_drawer_id: row.get(2)?,
        relation_type: row.get(3)?,
        valid_from: row.get(4)?,
        valid_until: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Confidence, NewDrawer, Source};

    fn setup() -> Palace {
        Palace::open_in_memory().unwrap()
    }

    fn make_drawer(p: &Palace) -> Drawer {
        let wing = p.create_wing("w", None).unwrap();
        let room = p.create_room(wing.id, "r", None).unwrap();
        p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "test content".to_string(),
            compressed_content: None,
            confidence: Confidence::Medium,
            source: Source::User,
        }).unwrap()
    }

    #[test]
    fn test_create_wing_and_room() {
        let p = setup();
        let wing = p.create_wing("project", Some("main wing")).unwrap();
        assert_eq!(wing.name, "project");
        let room = p.create_room(wing.id, "auth", Some("auth room")).unwrap();
        assert_eq!(room.wing_id, wing.id);
    }

    #[test]
    fn test_store_and_get_drawer() {
        let p = setup();
        let wing = p.create_wing("w", None).unwrap();
        let room = p.create_room(wing.id, "r", None).unwrap();
        let d = p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "JWT tokens used".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        }).unwrap();
        assert!(!d.is_invalidated);
        let fetched = p.get_drawer(d.id).unwrap().unwrap();
        assert_eq!(fetched.access_count, 1);
    }

    #[test]
    fn test_search_drawers() {
        let p = setup();
        let wing = p.create_wing("w", None).unwrap();
        let room = p.create_room(wing.id, "r", None).unwrap();
        p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "authentication bearer tokens".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        }).unwrap();
        let results = p.search_drawers("authentication", 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_invalidate_excludes_from_recent() {
        let p = setup();
        let d = make_drawer(&p);
        p.invalidate_drawer(d.id).unwrap();
        let recent = p.get_recent_drawers(10).unwrap();
        assert!(recent.iter().all(|dr| dr.id != d.id));
    }

    #[test]
    fn test_invalidate_fact_queryable_with_flag() {
        let p = setup();
        let d = make_drawer(&p);
        p.invalidate_fact(d.id).unwrap();
        let invalidated = p.get_invalidated_drawers(10).unwrap();
        assert!(invalidated.iter().any(|dr| dr.id == d.id));
        let inv = invalidated.iter().find(|dr| dr.id == d.id).unwrap();
        assert!(inv.is_invalidated);
        assert!(inv.invalidated_at.is_some());
    }

    #[test]
    fn test_get_drawers_by_room() {
        let p = setup();
        let wing = p.create_wing("w", None).unwrap();
        let room = p.create_room(wing.id, "r", None).unwrap();
        p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "fact one".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        }).unwrap();
        p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "fact two".to_string(),
            compressed_content: None,
            confidence: Confidence::Medium,
            source: Source::User,
        }).unwrap();
        let drawers = p.get_drawers_by_room(room.id, 10).unwrap();
        assert_eq!(drawers.len(), 2);
    }

    #[test]
    fn test_compact_drawer() {
        let p = setup();
        let wing = p.create_wing("w", None).unwrap();
        let room = p.create_room(wing.id, "r", None).unwrap();
        let d = p.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "verbose original".to_string(),
            compressed_content: Some("summary".to_string()),
            confidence: Confidence::Low,
            source: Source::Conversation,
        }).unwrap();
        let compacted = p.compact_drawer(d.id, 1, 0).unwrap();
        assert!(compacted);
        let fetched = p.get_drawer(d.id).unwrap().unwrap();
        assert_eq!(fetched.content, "summary");
    }

    #[test]
    fn test_drawer_count() {
        let p = setup();
        assert_eq!(p.drawer_count().unwrap(), 0);
        make_drawer(&p);
        assert_eq!(p.drawer_count().unwrap(), 1);
    }
}
