use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use tracing::{debug, info, warn};
use wl_clipboard_rs::paste::{get_contents, ClipboardType, Error as PasteError, MimeType, Seat};

use crate::store::ClipboardStore;

/// Spawn a background thread that monitors the Wayland clipboard for changes
/// and stores new text entries into `store`.
///
/// Primary strategy: event-driven via `wl-paste --watch sh -c 'echo .'`.
/// The child process emits one line per clipboard change; we call `get_contents()`
/// only when notified — zero polling CPU overhead.
///
/// Fallback: poll every 3 s if `wl-paste` is unavailable.
pub fn spawn(store: Arc<Mutex<ClipboardStore>>) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || loop {
        if let Err(e) = event_driven_loop(Arc::clone(&store)) {
            warn!("event-driven clipboard watcher failed ({e}), falling back to polling");
            poll_loop(Arc::clone(&store));
        }
        // event_driven_loop returned Ok means wl-paste exited cleanly; restart.
        warn!("wl-paste --watch exited, restarting clipboard watcher in 5 s");
        std::thread::sleep(Duration::from_secs(5));
    })
}

/// Run `wl-paste --watch sh -c 'echo .'` and call `get_contents()` on each line.
/// Returns `Err` if `wl-paste` could not be spawned (binary missing).
/// Returns `Ok` if the process exited (restartable).
fn event_driven_loop(store: Arc<Mutex<ClipboardStore>>) -> anyhow::Result<()> {
    let mut child = Command::new("wl-paste")
        .args(["--watch", "sh", "-c", "echo ."])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("wl-paste spawn failed: {e}"))?;

    info!("clipboard watcher: event-driven via wl-paste --watch");
    read_and_store(&store);

    let stdout = child.stdout.take().expect("piped");
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        match line {
            Ok(_) => {
                debug!("clipboard change notification received");
                read_and_store(&store);
            }
            Err(_) => break,
        }
    }

    let _ = child.wait();
    Ok(())
}

/// Fallback: poll clipboard every 3 s (used when wl-paste is not installed).
fn poll_loop(store: Arc<Mutex<ClipboardStore>>) {
    warn!("clipboard watcher: polling every 3 s (install wl-clipboard for better performance)");
    read_and_store(&store);
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
                store_entry(&store, &content, &mime);
            }
            Err(PasteError::NoSeats | PasteError::ClipboardEmpty | PasteError::NoMimeType) => {}
            Err(e) => {
                warn!("clipboard read error: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn read_and_store(store: &Arc<Mutex<ClipboardStore>>) {
    // Use the wl-paste binary directly — the wl-clipboard-rs library silently
    // returns ClipboardEmpty/NoSeats in some compositor configurations even when
    // the binary works correctly.
    let output = match Command::new("wl-paste")
        .arg("--no-newline")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return, // wl-paste not installed
        Err(e) => {
            warn!("wl-paste read failed: {e}");
            return;
        }
    };
    if !output.status.success() {
        // Exit code != 0 means the clipboard is empty.
        return;
    }
    let Ok(content) = String::from_utf8(output.stdout) else { return };
    if content.trim().is_empty() {
        return;
    }
    store_entry(store, &content, "text/plain");
}

fn store_entry(store: &Arc<Mutex<ClipboardStore>>, content: &str, mime: &str) {
    let kind = if content.starts_with("file://") { "Uri" } else { "Text" };
    if let Ok(guard) = store.lock() {
        if let Err(e) = guard.insert(kind, content, mime) {
            warn!("failed to store clipboard entry: {e}");
        }
    }
}
