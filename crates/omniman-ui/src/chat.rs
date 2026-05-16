use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

#[allow(dead_code)]
pub struct Conversation {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
}

#[allow(dead_code)]
pub struct StoredMessage {
    pub id: i64,
    pub conv_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

pub struct ChatStore {
    conn: Connection,
}

impl ChatStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                title      TEXT    NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_conv_updated
                ON conversations (updated_at DESC);
            CREATE TABLE IF NOT EXISTS messages (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role            TEXT    NOT NULL,
                content         TEXT    NOT NULL,
                created_at      INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_msg_conv
                ON messages (conversation_id, created_at);",
        )?;
        Ok(Self { conn })
    }

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<Conversation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, updated_at FROM conversations ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(Conversation { id: row.get(0)?, title: row.get(1)?, updated_at: row.get(2)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn create_conversation(&self, title: &str) -> Result<i64> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO conversations (title, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![title, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename(&self, id: i64, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now_secs(), id],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn append_message(&self, conv_id: i64, role: &str, content: &str) -> Result<i64> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO messages (conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![conv_id, role, content, now],
        )?;
        self.conn.execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![now, conv_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn messages(&self, conv_id: i64) -> Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, conversation_id, role, content, created_at
             FROM messages WHERE conversation_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![conv_id], |row| {
            Ok(StoredMessage {
                id: row.get(0)?,
                conv_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn message_count(&self, conv_id: i64) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            params![conv_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_mem() -> ChatStore {
        ChatStore::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn create_and_list() {
        let store = open_mem();
        let id = store.create_conversation("Test").unwrap();
        let convs = store.list_conversations(10).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].id, id);
        assert_eq!(convs[0].title, "Test");
    }

    #[test]
    fn append_messages_and_load() {
        let store = open_mem();
        let id = store.create_conversation("C").unwrap();
        store.append_message(id, "user", "hello").unwrap();
        store.append_message(id, "assistant", "hi").unwrap();
        let msgs = store.messages(id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn delete_cascades_messages() {
        let store = open_mem();
        let id = store.create_conversation("C").unwrap();
        store.append_message(id, "user", "x").unwrap();
        store.delete(id).unwrap();
        assert!(store.list_conversations(10).unwrap().is_empty());
        assert!(store.messages(id).unwrap().is_empty());
    }

    #[test]
    fn rename_updates_title() {
        let store = open_mem();
        let id = store.create_conversation("Old").unwrap();
        store.rename(id, "New").unwrap();
        let convs = store.list_conversations(10).unwrap();
        assert_eq!(convs[0].title, "New");
    }
}
