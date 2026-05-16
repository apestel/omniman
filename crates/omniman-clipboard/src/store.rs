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

    /// Insert a new entry, or promote an existing one if the content already exists.
    /// Skips entirely if content is identical to the most recent entry.
    /// Returns `true` if the history changed (new row or reordering), `false` otherwise.
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
        // Promote existing entry instead of inserting a duplicate row.
        let existing_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM clip_entries WHERE content = ?1 LIMIT 1",
                params![content],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing_id {
            self.conn.execute(
                "UPDATE clip_entries SET created_at = ?1, kind = ?2, mime = ?3 WHERE id = ?4",
                params![now, kind, mime, id],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO clip_entries (kind, content, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![kind, content, mime, now],
            )?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn open_tmp() -> ClipboardStore {
        ClipboardStore::open(Path::new(":memory:")).unwrap()
    }

    fn insert_at(store: &ClipboardStore, kind: &str, content: &str, mime: &str, ts: i64) {
        store.conn.execute(
            "INSERT INTO clip_entries (kind, content, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![kind, content, mime, ts],
        ).unwrap();
    }

    #[test]
    fn insert_promotes_duplicate_no_new_row() {
        let store = open_tmp();
        // Use explicit timestamps so ordering is deterministic.
        insert_at(&store, "Text", "aaa", "text/plain", 1000);
        insert_at(&store, "Text", "bbb", "text/plain", 2000);
        // re-insert aaa — should promote, not duplicate
        let changed = store.insert("Text", "aaa", "text/plain").unwrap();
        assert!(changed, "promotion should signal a history change");
        let history = store.history(10).unwrap();
        assert_eq!(history.len(), 2, "should still be 2 entries");
        assert_eq!(history[0].content, "aaa", "promoted entry should be newest");
        assert_eq!(history[1].content, "bbb");
    }

    #[test]
    fn insert_skips_when_already_most_recent() {
        let store = open_tmp();
        store.insert("Text", "aaa", "text/plain").unwrap();
        let changed = store.insert("Text", "aaa", "text/plain").unwrap();
        assert!(!changed);
        assert_eq!(store.history(10).unwrap().len(), 1);
    }
}
