use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use walkdir::WalkDir;

use crate::FileIndex;

pub type WatcherHandle = Arc<Mutex<RecommendedWatcher>>;

/// Spawn a background inotify task.
///
/// `skip` must be an absolute path to exclude from watching and event processing —
/// pass `Config::data_dir()` so the daemon's own index/DB files never trigger
/// a re-indexing feedback loop.
///
/// File-system events are accumulated for 2 seconds before being flushed as a
/// single `apply_batch` call (one IndexWriter, one commit). This prevents the
/// O(n-writers) cost of handling every rapid-fire event individually.
pub fn spawn(index: Arc<FileIndex>, root: PathBuf, skip: PathBuf) -> anyhow::Result<WatcherHandle> {
    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

    let watcher = Arc::new(Mutex::new(
        notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|e| anyhow::anyhow!("creating file watcher: {e}"))?,
    ));

    // Walk root, add a non-recursive watch per accessible dir, skipping `skip`.
    {
        let mut w = watcher.lock().unwrap();
        add_watches_recursive(&mut *w, &root, &skip);
    }

    let watcher_clone = Arc::clone(&watcher);
    tokio::spawn(async move {
        // true = upsert, false = delete. HashMap deduplicates: last event for a path wins.
        let mut pending: HashMap<PathBuf, bool> = HashMap::new();
        let mut flush_ticker = tokio::time::interval(Duration::from_secs(2));
        flush_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                Some(res) = rx.recv() => {
                    let Ok(event) = res else { continue; };
                    for path in event.paths {
                        // Never process events from our own data directory.
                        if path.starts_with(&skip) {
                            continue;
                        }
                        match event.kind {
                            EventKind::Create(_) => {
                                if path.is_dir() {
                                    // Watch new directories immediately so we don't miss events inside them.
                                    if let Ok(mut w) = watcher_clone.lock() {
                                        if let Err(e) = w.watch(&path, RecursiveMode::NonRecursive) {
                                            if !is_permission_error(&e) {
                                                warn!("failed to watch new dir {path:?}: {e}");
                                            }
                                        }
                                    }
                                } else {
                                    pending.insert(path, true);
                                }
                            }
                            EventKind::Modify(_) => {
                                // Only queue if not already marked for delete.
                                if pending.get(&path).copied() != Some(false) {
                                    pending.insert(path, true);
                                }
                            }
                            EventKind::Remove(_) => {
                                pending.insert(path, false);
                            }
                            _ => {}
                        }
                    }
                }
                _ = flush_ticker.tick() => {
                    if pending.is_empty() {
                        continue;
                    }
                    let batch = std::mem::take(&mut pending);
                    let index = Arc::clone(&index);
                    tokio::task::spawn_blocking(move || {
                        let mut upserts = Vec::new();
                        let mut deletes = Vec::new();
                        for (path, is_upsert) in batch {
                            if is_upsert { upserts.push(path); } else { deletes.push(path); }
                        }
                        debug!(upserts = upserts.len(), deletes = deletes.len(), "flushing index batch");
                        if let Err(e) = index.apply_batch(&upserts, &deletes) {
                            warn!("batch index update failed: {e}");
                        }
                    });
                }
            }
        }
    });

    Ok(watcher)
}

fn add_watches_recursive(watcher: &mut RecommendedWatcher, root: &std::path::Path, skip: &std::path::Path) {
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !e.path().starts_with(skip))
        .filter_map(|e| match e {
            Ok(e) => Some(e),
            Err(ref err) if is_permission_denied_walk(err) => {
                debug!("skipping watch (permission denied): {:?}", err.path());
                None
            }
            Err(err) => {
                warn!("walk error during watch setup: {err}");
                None
            }
        })
        .filter(|e| e.file_type().is_dir())
    {
        if let Err(e) = watcher.watch(entry.path(), RecursiveMode::NonRecursive) {
            if is_permission_error(&e) {
                debug!("skipping watch (permission denied): {:?}", entry.path());
            } else {
                warn!("failed to watch {:?}: {e}", entry.path());
            }
        }
    }
}

fn is_permission_denied_walk(err: &walkdir::Error) -> bool {
    err.io_error()
        .map(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        .unwrap_or(false)
}

fn is_permission_error(e: &notify::Error) -> bool {
    matches!(
        e.kind,
        notify::ErrorKind::Io(ref io) if io.kind() == std::io::ErrorKind::PermissionDenied
    )
}
