use crate::types::{ClipEntry, Hit, ModelEntry};
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

    async fn ask_ai(&self, prompt: &str) -> zbus::Result<String>;

    async fn request_show_ui(&self) -> zbus::Result<()>;

    /// Returns models available via the configured Gemini API key.
    /// Returns an empty vec if no key is configured or the request fails.
    async fn list_models(&self) -> zbus::Result<Vec<ModelEntry>>;

    async fn reindex(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn show_ui(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn clipboard_changed(&self) -> zbus::Result<()>;
}
