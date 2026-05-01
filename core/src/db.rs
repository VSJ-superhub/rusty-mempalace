use std::path::Path;
use rusqlite::{Connection, Result, params};

pub fn open(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
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

    // Additive migration: tolerate databases created before invalidated_at column.
    let res = conn.execute_batch("ALTER TABLE drawers ADD COLUMN invalidated_at TEXT;");
    if let Err(e) = res {
        if !e.to_string().contains("duplicate column name") {
            return Err(e);
        }
    }

    Ok(())
}

pub fn wal_append(
    conn: &Connection,
    operation: &str,
    table_name: &str,
    record_id: i64,
    payload: &str,
) -> Result<()> {
    use chrono::Utc;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO write_ahead_log (operation, table_name, record_id, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![operation, table_name, record_id, payload, now],
    )?;
    Ok(())
}
