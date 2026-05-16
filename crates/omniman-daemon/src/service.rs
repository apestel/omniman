use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use omniman_ai::GeminiClient;
use omniman_clipboard::ClipboardStore;
use omniman_core::types::{ClipEntry, Hit};
use omniman_index::FileIndex;
use zbus::interface;

pub struct OmnimanService {
    pub index: Arc<FileIndex>,
    pub clipboard: Arc<Mutex<ClipboardStore>>,
    pub ai: Option<Arc<GeminiClient>>,
    pub home: PathBuf,
}

#[interface(name = "org.adrien.Omniman1")]
impl OmnimanService {
    async fn search(&self, query: &str, limit: u32) -> Vec<Hit> {
        tracing::debug!(%query, limit, "search");
        self.index
            .search(query, limit as usize)
            .unwrap_or_else(|e| {
                tracing::warn!("search error: {e}");
                vec![]
            })
    }

    async fn clipboard_history(&self, limit: u32) -> Vec<ClipEntry> {
        tracing::debug!(limit, "clipboard_history");
        match self.clipboard.lock() {
            Ok(store) => store.history(limit as usize).unwrap_or_else(|e| {
                tracing::warn!("clipboard history error: {e}");
                vec![]
            }),
            Err(_) => vec![],
        }
    }

    async fn ask_ai(&self, prompt: &str) -> String {
        tracing::debug!(%prompt, "ask_ai");
        let Some(client) = &self.ai else {
            return "GEMINI_API_KEY is not configured. Set it as an environment variable for omnimand.".into();
        };
        match client.ask(prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!("Gemini error: {e}");
                format!("Error: {e}")
            }
        }
    }

    /// D-Bus method callable by external tools (e.g. the GNOME custom keybinding
    /// command) to trigger the ShowUi signal without needing a portal.
    async fn request_show_ui(
        &self,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        tracing::debug!("request_show_ui called");
        Self::show_ui(&emitter).await.map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Force a full re-crawl of $HOME from scratch.  The normal startup pass is
    /// an incremental mtime sweep; this method is the escape hatch when the user
    /// suspects the index has drifted (e.g. after `rm -rf` while the daemon was off).
    /// Runs on a blocking thread; returns immediately, completion logged to journald.
    async fn reindex(&self) -> zbus::fdo::Result<()> {
        tracing::info!("Reindex requested via D-Bus");
        let index = Arc::clone(&self.index);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = index.crawl(&home) {
                tracing::error!("manual reindex failed: {e}");
            }
        });
        Ok(())
    }

    #[zbus(signal)]
    pub async fn show_ui(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniman_core::config::IndexConfig;
    use tempfile::TempDir;

    fn make_service(tmp: &TempDir) -> OmnimanService {
        let index = Arc::new(
            omniman_index::FileIndex::open(
                &tmp.path().join("index"),
                IndexConfig::default(),
            )
            .unwrap(),
        );
        let clipboard = Arc::new(Mutex::new(
            omniman_clipboard::ClipboardStore::open(&tmp.path().join("clip.db")).unwrap(),
        ));
        OmnimanService {
            index,
            clipboard,
            ai: None,
            home: tmp.path().to_path_buf(),
        }
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let svc = make_service(&tmp);
        assert!(svc.search("", 10).await.is_empty());
    }

    #[tokio::test]
    async fn search_after_crawl_finds_file() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("needle.txt"), "content").unwrap();

        let svc = make_service(&tmp);
        svc.index.crawl(&home).unwrap();
        svc.index.reload().unwrap();

        let hits = svc.search("needle", 10).await;
        assert!(!hits.is_empty(), "should find needle.txt");
    }

    #[tokio::test]
    async fn clipboard_history_empty_initially() {
        let tmp = TempDir::new().unwrap();
        let svc = make_service(&tmp);
        assert!(svc.clipboard_history(10).await.is_empty());
    }

    #[tokio::test]
    async fn ask_ai_without_key_returns_message() {
        let tmp = TempDir::new().unwrap();
        let svc = make_service(&tmp);
        let resp = svc.ask_ai("hello").await;
        assert!(
            resp.contains("GEMINI_API_KEY"),
            "should mention missing key, got: {resp}"
        );
    }
}
