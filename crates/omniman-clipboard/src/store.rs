use std::path::Path;

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use omniman_core::types::ClipEntry;
use rusqlite::{params, Connection};

use crate::encryption::ClipEncryption;

pub struct ClipboardStore {
    conn: Connection,
    encryption: Option<ClipEncryption>,
}

impl ClipboardStore {
    pub fn open(path: &Path, encryption: Option<ClipEncryption>) -> Result<Self> {
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
                created_at INTEGER NOT NULL,
                encrypted  INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_clip_created
                ON clip_entries (created_at DESC);",
        )?;

        // Migration: add `encrypted` column if it doesn't exist yet.
        let has_column: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('clip_entries') WHERE name = 'encrypted'",
            [],
            |r| r.get(0),
        )?;
        if !has_column {
            conn.execute(
                "ALTER TABLE clip_entries ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 0",
                [],
            )?;

            // Best-effort: try to decrypt each entry so we can mark it correctly.
            if let Some(enc) = &encryption {
                let mut stmt = conn.prepare(
                    "SELECT id, content FROM clip_entries WHERE encrypted = 0",
                )?;
                let rows: Vec<(i64, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .filter_map(|r| r.ok())
                    .collect();
                for (id, content) in rows {
                    if let Ok(raw) = STANDARD.decode(&content) {
                        if let Ok(decrypted) = enc.decrypt(&raw) {
                            if String::from_utf8(decrypted).is_ok() {
                                let _ = conn.execute(
                                    "UPDATE clip_entries SET encrypted = 1 WHERE id = ?",
                                    params![id],
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(Self { conn, encryption })
    }

    /// Insert a new entry, or promote an existing one if the content already exists.
    /// Skips entirely if content is identical to the most recent entry.
    /// Returns `true` if the history changed (new row or reordering), `false` otherwise.
    pub fn insert(&self, kind: &str, content: &str, mime: &str) -> Result<bool> {
        let (stored_content, was_encrypted): (String, i32) = if let Some(enc) = &self.encryption {
            if let Ok(encrypted_bytes) = enc.encrypt(content.as_bytes()) {
                (STANDARD.encode(&encrypted_bytes), 1)
            } else {
                (content.to_string(), 0)
            }
        } else {
            (content.to_string(), 0)
        };

        let recent: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM clip_entries ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();
        if recent.as_deref() == Some(&stored_content) {
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
                params![stored_content],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing_id {
            self.conn.execute(
                "UPDATE clip_entries SET created_at = ?1, kind = ?2, mime = ?3, encrypted = ?4 WHERE id = ?5",
                params![now, kind, mime, was_encrypted, id],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO clip_entries (kind, content, mime, created_at, encrypted) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![kind, stored_content, mime, now, was_encrypted],
            )?;
        }
        Ok(true)
    }

    pub fn history(&self, limit: usize) -> Result<Vec<ClipEntry>> {
        let mut stmt = self.conn.prepare(
                        "SELECT id, kind, content, mime, created_at, encrypted
                 FROM clip_entries
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1",
        )?;
        let entries = stmt
            .query_map(params![limit as i64], |row| {
                let mut content: String = row.get(2)?;
                let encrypted: i32 = row.get(5)?;
                if encrypted == 1 {
                    if let Some(enc) = &self.encryption {
                        if let Ok(raw) = STANDARD.decode(&content) {
                            if let Ok(decrypted) = enc.decrypt(&raw) {
                                if let Ok(text) = String::from_utf8(decrypted) {
                                    content = text;
                                } else {
                                    tracing::warn!(id = row.get::<_, i64>(0).unwrap_or(-1), "decrypted bytes are not valid UTF-8");
                                }
                            } else {
                                tracing::warn!(id = row.get::<_, i64>(0).unwrap_or(-1), "failed to decrypt clipboard entry");
                            }
                        } else {
                            tracing::warn!(id = row.get::<_, i64>(0).unwrap_or(-1), "failed to base64-decode encrypted clipboard entry");
                        }
                    } else {
                        tracing::warn!(id = row.get::<_, i64>(0).unwrap_or(-1), "clipboard entry is encrypted but no key available");
                    }
                }
                Ok(ClipEntry {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    content,
                    mime: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    /// Delete entries beyond `limit`, keeping the most recent ones.
    pub fn prune(&self, limit: usize) -> Result<()> {
        self.conn.execute(
                  "DELETE FROM clip_entries WHERE id NOT IN (
                SELECT id FROM clip_entries ORDER BY created_at DESC, id DESC LIMIT ?1
            )",
            params![limit as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

     fn open_tmp() -> ClipboardStore {
        ClipboardStore::open(Path::new(":memory:"), None).unwrap()
    }

    fn insert_at(store: &ClipboardStore, kind: &str, content: &str, mime: &str, ts: i64) {
        store.conn.execute(
            "INSERT INTO clip_entries (kind, content, mime, created_at, encrypted) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![kind, content, mime, ts, 0],
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

    #[test]
    fn prune_removes_oldest_entries() {
        let store = open_tmp();
        for i in 0..10 {
            insert_at(&store, "Text", &format!("entry{}", i), "text/plain", i);
        }
        assert_eq!(store.history(100).unwrap().len(), 10);
        store.prune(3).unwrap();
        let history = store.history(100).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "entry9");
        assert_eq!(history[1].content, "entry8");
        assert_eq!(history[2].content, "entry7");
    }
}
