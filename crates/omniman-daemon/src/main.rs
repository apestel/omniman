mod service;

use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use omniman_ai::GeminiClient;
use omniman_clipboard::ClipboardStore;
use omniman_core::{config::Config, ipc};
use omniman_index::{watcher, FileIndex};
use service::OmnimanService;
use std::sync::Mutex;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    lower_priority();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("omnimand=debug".parse()?),
        )
        .init();

    let config = Config::load().context("loading config")?;
    info!("omnimand starting");

    // ── Index ─────────────────────────────────────────────────────────────────
    let index_dir = Config::data_dir().join("index");
    let file_index = Arc::new(
        FileIndex::open(&index_dir, config.index.clone()).context("opening file index")?,
    );

    let index_clone = Arc::clone(&file_index);
    let home = home_dir();
    let home_clone = home.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = index_clone.sweep(&home_clone) {
            tracing::error!("startup sweep failed: {e}");
        }
    });

    let _watcher = watcher::spawn(Arc::clone(&file_index), home.clone(), Config::data_dir())
        .context("creating file watcher")?;

    // ── Clipboard ─────────────────────────────────────────────────────────────
    let clip_db = Config::data_dir().join("clipboard.db");
    let clip_store = Arc::new(Mutex::new(
        ClipboardStore::open(&clip_db).context("opening clipboard store")?,
    ));

    // ── Gemini client ─────────────────────────────────────────────────────────
    let ai_client = {
        let key = std::env::var("GEMINI_API_KEY")
            .ok()
            .or_else(|| config.ai.gemini_api_key.clone());
        match key {
            Some(k) => {
                info!(model = %config.ai.model, "Gemini client ready");
                Some(Arc::new(GeminiClient::new(k, config.ai.model.clone())))
            }
            None => {
                tracing::warn!("AI unavailable: no GEMINI_API_KEY and no key in settings");
                None
            }
        }
    };

    // ── D-Bus service ─────────────────────────────────────────────────────────
    let service = OmnimanService {
        index: Arc::clone(&file_index),
        clipboard: Arc::clone(&clip_store),
        ai: ai_client,
        home: home.clone(),
    };
    let conn = zbus::connection::Builder::session()?
        .name(ipc::BUS_NAME)?
        .serve_at(ipc::OBJECT_PATH, service)?
        .build()
        .await
        .context("connecting to session D-Bus")?;

    info!(
        bus_name = ipc::BUS_NAME,
        object_path = ipc::OBJECT_PATH,
        "D-Bus service running"
    );

    // Global shortcut registration is handled by the UI process (omniman), which
    // has a Wayland connection that GNOME's portal requires.

    std::future::pending::<()>().await;
    drop(conn);
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Make the daemon nice and IO-idle so indexing/sweeping yields CPU and disk to
/// the foreground.  Errors are swallowed — these calls can't usefully fail and
/// we don't want to refuse to start if the kernel rejects the request.
fn lower_priority() {
    // nice(19) — lowest CPU priority.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
    // IOPRIO_CLASS_IDLE via the raw ioprio_set syscall (libc has no wrapper).
    // who=IOPRIO_WHO_PROCESS(1), who_id=0 (= self), data=class<<13.
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    const IOPRIO_CLASS_IDLE: libc::c_int = 3;
    let prio = IOPRIO_CLASS_IDLE << 13;
    unsafe {
        libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, prio);
    }
}
