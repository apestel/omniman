use std::path::Path;

use anyhow::Result;
use omniman_core::types::ClipEntry;
use rusqlite::{params, Connection};

pub struct ClipboardStore {
    conn: Connection,
}

impl ClipboardStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clip_entries (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                kind       TEXT    NOT NULL,
                content    TEXT    NOT NULL,
                mime       TEXT    NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_clip_created
                ON clip_entries (created_at DESC);",
        )?;
        Ok(Self { conn })
    }

    /// Insert a new entry, skipping if identical to the most recent one.
    /// Returns `true` if a new row was stored, `false` if it was a duplicate.
    pub fn insert(&self, kind: &str, content: &str, mime: &str) -> Result<bool> {
        let recent: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM clip_entries ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();
        if recent.as_deref() == Some(content) {
            return Ok(false);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        self.conn.execute(
            "INSERT INTO clip_entries (kind, content, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![kind, content, mime, now],
        )?;
        Ok(true)
    }

    pub fn history(&self, limit: usize) -> Result<Vec<ClipEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, content, mime, created_at
             FROM clip_entries
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let entries = stmt
            .query_map(params![limit as i64], |row| {
                Ok(ClipEntry {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    content: row.get(2)?,
                    mime: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }
}
