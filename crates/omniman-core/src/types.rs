use serde::{Deserialize, Serialize};

/// A Gemini model available for AI queries.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct ModelEntry {
    /// Model ID as used in the API (e.g. "gemini-2.5-flash").
    pub id: String,
    /// Human-readable name from the Gemini models list.
    pub display_name: String,
}

/// A single file-search result.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct Hit {
    pub path: String,
    pub filename: String,
    pub score: f64,
    pub snippet: String,
}

/// A clipboard entry kind.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub enum ClipKind {
    Text,
    Image,
    Uri,
}

/// A single clipboard history entry.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct ClipEntry {
    pub id: i64,
    pub kind: String,
    /// UTF-8 text or base64-encoded image/bytes.
    pub content: String,
    pub mime: String,
    pub created_at: i64,
}

/// Role of a chat participant.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl Default for ChatRole {
    fn default() -> Self {
        Self::User
    }
}

/// A single turn in a multi-turn AI conversation.
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub content: String,
}

impl Default for ChatTurn {
    fn default() -> Self {
        Self {
            role: ChatRole::default(),
            content: String::new(),
        }
    }
}

impl ChatTurn {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}
