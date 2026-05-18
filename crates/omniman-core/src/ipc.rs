use crate::types::{ChatTurn, ClipEntry, Hit, ModelEntry};
use zbus::proxy;

/// The daemon's well-known D-Bus service name.
/// Deliberately distinct from the GTK4 app ID ("org.adrien.Omniman") so
/// GApplication can own that name without conflicting with the daemon.
pub const BUS_NAME: &str = "org.adrien.OmnimanDaemon";
pub const OBJECT_PATH: &str = "/org/adrien/Omniman";

/// D-Bus proxy used by the UI client to talk to `omnimand`.
#[proxy(
    interface = "org.adrien.Omniman1",
    default_service = "org.adrien.OmnimanDaemon",
    default_path = "/org/adrien/Omniman"
)]
pub trait Omniman {
    async fn search(&self, query: &str, limit: u32) -> zbus::Result<Vec<Hit>>;

    async fn clipboard_history(&self, limit: u32) -> zbus::Result<Vec<ClipEntry>>;

    /// Start a streaming AI chat. Returns a session ID used to correlate
    /// subsequent `chat_chunk`, `chat_done`, and `chat_error` signals.
    async fn chat_streaming(&self, history: Vec<ChatTurn>) -> zbus::Result<u64>;

    async fn request_show_ui(&self) -> zbus::Result<()>;

    /// Returns models available via the configured Gemini API key.
    /// Returns an empty vec if no key is configured or the request fails.
    async fn list_models(&self) -> zbus::Result<Vec<ModelEntry>>;

    async fn reindex(&self) -> zbus::Result<()>;

    async fn store_clip_entry(
        &self,
        kind: &str,
        content: &str,
        mime: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn show_ui(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn clipboard_changed(&self) -> zbus::Result<()>;

    /// Streaming text chunk for an active chat session.
    #[zbus(signal)]
    fn chat_chunk(&self, session: u64, text: &str) -> zbus::Result<()>;

    /// Final accumulated response for a completed chat session.
    #[zbus(signal)]
    fn chat_done(&self, session: u64, text: &str) -> zbus::Result<()>;

    /// Error for a chat session (includes rate-limit and API errors).
    #[zbus(signal)]
    fn chat_error(&self, session: u64, msg: &str) -> zbus::Result<()>;
}
