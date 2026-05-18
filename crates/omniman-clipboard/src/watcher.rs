use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::watch;
use tracing::{debug, info, warn};
use wl_clipboard_rs::paste::{get_contents, ClipboardType, Error as PasteError, MimeType, Seat};

use crate::store::ClipboardStore;

/// Spawn a background thread that polls the Wayland clipboard for changes
/// using `wl-clipboard-rs` (no external binary required).
///
/// Returns a `watch::Receiver` that fires whenever a new entry is actually
/// stored (duplicates do not trigger it).
pub fn spawn(store: Arc<Mutex<ClipboardStore>>) -> (tokio::task::JoinHandle<()>, watch::Receiver<()>) {
    let (tx, rx) = watch::channel(());
    let handle = tokio::task::spawn_blocking(move || poll_loop(Arc::clone(&store), &tx));
    (handle, rx)
}

/// Poll clipboard every 3 s via `wl-clipboard-rs`.
fn poll_loop(store: Arc<Mutex<ClipboardStore>>, notify: &watch::Sender<()>) {
    info!("clipboard watcher: polling every 3 s via wl-clipboard-rs");
    read_and_store(&store, notify);
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(3));
        match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
            Ok((mut pipe, mime)) => {
                let mut buf = Vec::new();
                if pipe.read_to_end(&mut buf).is_err() {
                    continue;
                }
                let Ok(content) = String::from_utf8(buf) else { continue };
                if content.trim().is_empty() || content == last {
                    continue;
                }
                last = content.clone();
                store_entry(&store, &content, &mime, notify);
            }
            Err(PasteError::NoSeats | PasteError::ClipboardEmpty | PasteError::NoMimeType) => {}
            Err(e) => {
                warn!("clipboard read error: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

/// Read the current clipboard content using `wl-clipboard-rs` and store it.
fn read_and_store(store: &Arc<Mutex<ClipboardStore>>, notify: &watch::Sender<()>) {
    match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
        Ok((mut pipe, mime)) => {
            let mut buf = Vec::new();
            if pipe.read_to_end(&mut buf).is_err() {
                return;
            }
            let Ok(content) = String::from_utf8(buf) else { return };
            if content.trim().is_empty() {
                return;
            }
            debug!("clipboard initial read: {} chars", content.len());
            store_entry(store, &content, &mime, notify);
        }
        Err(PasteError::NoSeats | PasteError::ClipboardEmpty | PasteError::NoMimeType) => {
            debug!("clipboard empty on initial read");
        }
        Err(e) => {
            warn!("clipboard initial read error: {e}");
        }
    }
}

fn store_entry(store: &Arc<Mutex<ClipboardStore>>, content: &str, mime: &str, notify: &watch::Sender<()>) {
    let kind = if content.starts_with("file://") { "Uri" } else { "Text" };
    if let Ok(guard) = store.lock() {
        match guard.insert(kind, content, mime) {
            Ok(true) => { notify.send(()).ok(); }
            Ok(false) => {}
            Err(e) => warn!("failed to store clipboard entry: {e}"),
        }
    }
}
