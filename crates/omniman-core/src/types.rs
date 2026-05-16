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
