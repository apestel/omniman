use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use dashmap::DashMap;
use omniman_ai::{GeminiClient, OpenAiClient};
use omniman_clipboard::ClipboardStore;
use omniman_core::types::{ChatTurn, ClipEntry, Hit, ModelEntry};
use omniman_index::FileIndex;
use zbus::interface;

pub struct OmnimanService {
    pub index: Arc<FileIndex>,
    pub clipboard: Arc<Mutex<ClipboardStore>>,
    pub ai_gemini: Option<Arc<GeminiClient>>,
    pub ai_openai: Option<Arc<OpenAiClient>>,
    pub home: PathBuf,
    pub max_depth: usize,
    pub sessions: DashMap<u64, async_channel::Sender<String>>,
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

    async fn chat_streaming(
        &self,
        history: Vec<ChatTurn>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> u64 {
        if self.ai_openai.is_none() && self.ai_gemini.is_none() {
            Self::chat_error(&emitter, 0, "No AI client configured. Set a Gemini or OpenAI API key in preferences.").await.ok();
            return 0;
        }

        let session = self.next_session_id();
        let (chunk_tx, _chunk_rx) = async_channel::bounded::<String>(64);
        self.sessions.insert(session, chunk_tx);

        let emitter = emitter.into_owned();

        if let Some(client) = self.ai_openai.clone() {
            let sessions = self.sessions.clone();
            let emitter = emitter.clone();
            tracing::info!(session, client = "openai", turns = history.len(), "chat_streaming started");
            tokio::spawn(async move {
                stream_openai(emitter, session, sessions, client, history).await;
            });
        } else if let Some(client) = self.ai_gemini.clone() {
            let sessions = self.sessions.clone();
            let emitter = emitter.clone();
            tracing::info!(session, client = "gemini", turns = history.len(), "chat_streaming started");
            tokio::spawn(async move {
                stream_gemini(emitter, session, sessions, client, history).await;
            });
        }

        session
    }

    async fn list_models(&self) -> Vec<ModelEntry> {
        let Some(client) = &self.ai_gemini else {
            tracing::debug!("list_models: no Gemini client configured");
            return vec![];
        };
        match client.list_models().await {
            Ok(models) => models
                .into_iter()
                .map(|m| ModelEntry { id: m.id, display_name: m.display_name })
                .collect(),
            Err(e) => {
                tracing::warn!("list_models error: {e}");
                vec![]
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
        let max_depth = self.max_depth;
        tokio::task::spawn_blocking(move || {
            if let Err(e) = index.crawl(&home, max_depth) {
                tracing::error!("manual reindex failed: {e}");
            }
        });
        Ok(())
    }

    async fn store_clip_entry(
        &self,
        kind: &str,
        content: &str,
        mime: &str,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let changed = if let Ok(store) = self.clipboard.lock() {
            store.insert(kind, content, mime).unwrap_or(false)
        } else {
            false
        };
        if changed {
            Self::clipboard_changed(&emitter).await.ok();
        }
        Ok(())
    }

    #[zbus(signal)]
    pub async fn show_ui(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn clipboard_changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn chat_chunk(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session: u64,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn chat_done(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session: u64,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn chat_error(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session: u64,
        msg: &str,
    ) -> zbus::Result<()>;
}

impl OmnimanService {
    fn next_session_id(&self) -> u64 {
        let mut id = self.sessions.len() as u64 + 1;
        while self.sessions.contains_key(&id) {
            id += 1;
        }
        id
    }
}

/// Stream a Gemini chat, forwarding chunks via D-Bus signals.
async fn stream_gemini(
    emitter: zbus::object_server::SignalEmitter<'static>,
    session: u64,
    sessions: DashMap<u64, async_channel::Sender<String>>,
    client: Arc<GeminiClient>,
    history: Vec<ChatTurn>,
) {
    let (chunk_tx, chunk_rx) = async_channel::bounded::<String>(64);

    let stream_task = tokio::spawn(async move {
        client.chat_streaming(&history, &chunk_tx).await
    });

    let emit = emitter.clone();
    let accumulated = tokio::spawn(async move {
        let mut acc = String::new();
        while let Ok(chunk) = chunk_rx.recv().await {
            acc.push_str(&chunk);
            OmnimanService::chat_chunk(&emit, session, &chunk).await.ok();
        }
        acc
    }).await.unwrap_or_default();

    finish_stream(&emitter, session, &sessions, accumulated, stream_task.await).await;
}

/// Stream an OpenAI-compatible chat, forwarding chunks via D-Bus signals.
async fn stream_openai(
    emitter: zbus::object_server::SignalEmitter<'static>,
    session: u64,
    sessions: DashMap<u64, async_channel::Sender<String>>,
    client: Arc<OpenAiClient>,
    history: Vec<ChatTurn>,
) {
    let (chunk_tx, chunk_rx) = async_channel::bounded::<String>(64);

    let stream_task = tokio::spawn(async move {
        client.chat_streaming(&history, &chunk_tx).await
    });

    let emit = emitter.clone();
    let accumulated = tokio::spawn(async move {
        let mut acc = String::new();
        while let Ok(chunk) = chunk_rx.recv().await {
            acc.push_str(&chunk);
            OmnimanService::chat_chunk(&emit, session, &chunk).await.ok();
        }
        acc
    }).await.unwrap_or_default();

    finish_stream(&emitter, session, &sessions, accumulated, stream_task.await).await;
}

async fn finish_stream(
    emitter: &zbus::object_server::SignalEmitter<'static>,
    session: u64,
    sessions: &DashMap<u64, async_channel::Sender<String>>,
    accumulated: String,
    stream_result: Result<anyhow::Result<()>, tokio::task::JoinError>,
) {
    match stream_result {
        Ok(Ok(())) => {
            OmnimanService::chat_done(emitter, session, &accumulated).await.ok();
        }
        Ok(Err(e)) => {
            let msg = format_error(&e);
            tracing::warn!(session, "chat_streaming error: {e}");
            OmnimanService::chat_error(emitter, session, &msg).await.ok();
        }
        Err(join_err) => {
            let msg = format!("error:{}", join_err);
            tracing::warn!(session, "chat_streaming task failed: {join_err}");
            OmnimanService::chat_error(emitter, session, &msg).await.ok();
        }
    }
    sessions.remove(&session);
}

fn format_error(e: &anyhow::Error) -> String {
    if let Some(rl) = e.downcast_ref::<omniman_ai::RateLimitError>() {
        format!("rate_limit:{}:{}", rl.retry_after_secs, "Rate limited")
    } else {
        format!("error::{}", e)
    }
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
            ai_gemini: None,
            ai_openai: None,
            home: tmp.path().to_path_buf(),
            max_depth: IndexConfig::default().max_depth,
            sessions: DashMap::new(),
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
        svc.index.crawl(&home, svc.max_depth).unwrap();
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
}
