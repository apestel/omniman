mod chat;
mod prefs;
mod window;

use std::{collections::HashMap, sync::Arc, time::Duration};

use gtk4::glib;
use gtk4::prelude::*;
use omniman_core::{
    ipc::OmnimanProxy,
    types::{ChatTurn, ClipEntry, Hit},
};

/// Request sent from UI → worker thread.
pub enum ChatReq {
    /// Stream a new AI turn; `history` already includes the new user message.
    NewTurn { conv_id: i64, history: Vec<ChatTurn> },
    /// Generate a short title for a newly-created conversation.
    Summarize { conv_id: i64, user_msg: String, assistant_msg: String },
}

/// Message sent from worker thread → UI.
pub enum ChatMsg {
    Start { conv_id: i64 },
    Chunk { conv_id: i64, text: String },
    Done { conv_id: i64, full_text: String },
    RateLimit { conv_id: i64, secs: u64 },
    Title { conv_id: i64, title: String },
}

/// Clipboard content detected by the UI monitor.
pub struct ClipContent {
    pub content: String,
    pub kind: String,
    pub mime: String,
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("omniman=debug".parse().unwrap()),
        )
        .init();

    let (query_tx, query_rx) = async_channel::bounded::<String>(32);
    let (result_tx, result_rx) = async_channel::bounded::<Vec<Hit>>(32);
    let (show_tx, show_rx) = async_channel::bounded::<()>(8);
    let (clip_req_tx, clip_req_rx) = async_channel::bounded::<()>(8);
    let (clip_result_tx, clip_result_rx) = async_channel::bounded::<Vec<ClipEntry>>(8);
    let (chat_req_tx, chat_req_rx) = async_channel::bounded::<ChatReq>(4);
    let (chat_msg_tx, chat_msg_rx) = async_channel::bounded::<ChatMsg>(128);

     let (clip_content_tx, clip_content_rx) = async_channel::bounded::<ClipContent>(8);
    let clip_content_rx = Arc::new(std::sync::Mutex::new(Some(clip_content_rx)));

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(dbus_worker(
            query_rx,
            result_tx,
            show_tx,
            clip_req_rx,
            clip_result_tx,
            chat_req_rx,
            chat_msg_tx,
            clip_content_rx,
        ));
    });

    let result_rx = Arc::new(std::sync::Mutex::new(Some(result_rx)));
    let show_rx = Arc::new(std::sync::Mutex::new(Some(show_rx)));
    let clip_result_rx = Arc::new(std::sync::Mutex::new(Some(clip_result_rx)));
    let chat_msg_rx = Arc::new(std::sync::Mutex::new(Some(chat_msg_rx)));

    let app = libadwaita::Application::builder()
        .application_id("org.adrien.Omniman")
        .build();

    app.connect_activate(move |app| {
        let rx = match result_rx.lock().unwrap().take() {
            Some(rx) => rx,
            None => return,
        };
        let srx = show_rx.lock().unwrap().take().expect("activate once");
        let crx = clip_result_rx.lock().unwrap().take().expect("activate once");
        let cmrx = chat_msg_rx.lock().unwrap().take().expect("activate once");
        window::build(
            app,
            query_tx.clone(),
            rx,
            srx,
            clip_req_tx.clone(),
            crx,
            chat_req_tx.clone(),
            cmrx,
            clip_content_tx.clone(),
        );
    });

    app.run()
}

async fn dbus_worker(
    query_rx: async_channel::Receiver<String>,
    result_tx: async_channel::Sender<Vec<Hit>>,
    show_tx: async_channel::Sender<()>,
    clip_req_rx: async_channel::Receiver<()>,
    clip_result_tx: async_channel::Sender<Vec<ClipEntry>>,
    chat_req_rx: async_channel::Receiver<ChatReq>,
    chat_msg_tx: async_channel::Sender<ChatMsg>,
    clip_content_rx: Arc<std::sync::Mutex<Option<async_channel::Receiver<ClipContent>>>>,
) {
    let clip_content_rx = clip_content_rx.lock().unwrap().take().expect("dbus_worker once");
    loop {
        match connect_and_serve(
            &query_rx,
            &result_tx,
            &show_tx,
            &clip_req_rx,
            &clip_result_tx,
            &chat_req_rx,
            &chat_msg_tx,
            &clip_content_rx,
        )
        .await
        {
            Ok(()) => break,
            Err(e) => {
                tracing::warn!("D-Bus worker error: {e}, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn connect_and_serve(
    query_rx: &async_channel::Receiver<String>,
    result_tx: &async_channel::Sender<Vec<Hit>>,
    show_tx: &async_channel::Sender<()>,
    clip_req_rx: &async_channel::Receiver<()>,
    clip_result_tx: &async_channel::Sender<Vec<ClipEntry>>,
    chat_req_rx: &async_channel::Receiver<ChatReq>,
    chat_msg_tx: &async_channel::Sender<ChatMsg>,
    clip_content_rx: &async_channel::Receiver<ClipContent>,
) -> anyhow::Result<()> {
    use anyhow::Context;
    use futures_util::StreamExt;

    let conn = zbus::Connection::session()
        .await
        .context("D-Bus session connection")?;

    let proxy = OmnimanProxy::new(&conn)
        .await
        .context("creating Omniman proxy")?;

    tracing::info!("connected to omnimand via D-Bus");

    let mut show_stream = proxy.receive_show_ui().await.context("subscribing ShowUi")?;
    let mut clip_changed_stream = proxy
        .receive_clipboard_changed()
        .await
        .context("subscribing ClipboardChanged")?;
    let mut chat_chunk_stream = proxy
        .receive_chat_chunk()
        .await
        .context("subscribing ChatChunk")?;
    let mut chat_done_stream = proxy
        .receive_chat_done()
        .await
        .context("subscribing ChatDone")?;
    let mut chat_error_stream = proxy
        .receive_chat_error()
        .await
        .context("subscribing ChatError")?;

    // Maps session_id (from daemon) → conv_id (our internal ID)
    let mut session_map: HashMap<u64, i64> = HashMap::new();

    loop {
        tokio::select! {
            Some(_) = show_stream.next() => {
                if show_tx.send(()).await.is_err() { break; }
            }
            Ok(query) = query_rx.recv() => {
                let hits = proxy.search(&query, 20).await.unwrap_or_default();
                if result_tx.send(hits).await.is_err() { break; }
            }
            Ok(()) = clip_req_rx.recv() => {
                let entries = proxy.clipboard_history(50).await.unwrap_or_default();
                if clip_result_tx.send(entries).await.is_err() { break; }
            }
            Some(_) = clip_changed_stream.next() => {
                let entries = proxy.clipboard_history(50).await.unwrap_or_default();
                if clip_result_tx.send(entries).await.is_err() { break; }
            }
            Ok(cc) = clip_content_rx.recv() => {
                let _ = proxy.store_clip_entry(&cc.kind, &cc.content, &cc.mime).await;
            }
            Ok(req) = chat_req_rx.recv() => {
                match req {
                    ChatReq::NewTurn { conv_id, history } => {
                        let _ = chat_msg_tx.send(ChatMsg::Start { conv_id }).await;
                        match proxy.chat_streaming(history).await {
                            Ok(session) => {
                                session_map.insert(session, conv_id);
                            }
                            Err(e) => {
                                tracing::warn!(conv_id, "chat_streaming D-Bus call failed: {e}");
                                let _ = chat_msg_tx.send(ChatMsg::Chunk {
                                    conv_id,
                                    text: format!("Error: {e}"),
                                }).await;
                                let _ = chat_msg_tx.send(ChatMsg::Done { conv_id, full_text: String::new() }).await;
                            }
                        }
                    }
                    ChatReq::Summarize { conv_id, user_msg, assistant_msg } => {
                        let prompt = format!(
                            "Summarize the topic of this exchange in 3 to 5 words.\n\
                             Return ONLY the title text — no quotes, no trailing punctuation.\n\n\
                             User: {user_msg}\nAssistant: {assistant_msg}"
                        );
                        let turns = vec![ChatTurn::user(prompt)];
                        match proxy.chat_streaming(turns).await {
                            Ok(session) => {
                                // Use negative session-like conv_id to distinguish summarize from chat
                                // Since session IDs are positive, we store the conv_id directly.
                                session_map.insert(session, conv_id);
                            }
                            Err(e) => {
                                tracing::warn!(conv_id, "summarize D-Bus call failed: {e}");
                            }
                        }
                    }
                }
            }
            Some(signal) = chat_chunk_stream.next() => {
                if let Ok(args) = signal.args() {
                    if let Some(&conv_id) = session_map.get(&args.session) {
                        let _ = chat_msg_tx.send(ChatMsg::Chunk {
                            conv_id,
                            text: args.text.to_string(),
                        }).await;
                    }
                }
            }
            Some(signal) = chat_done_stream.next() => {
                if let Ok(args) = signal.args() {
                    if let Some(&conv_id) = session_map.get(&args.session) {
                        let _ = chat_msg_tx.send(ChatMsg::Done {
                            conv_id,
                            full_text: args.text.to_string(),
                        }).await;
                    }
                    session_map.remove(&args.session);
                }
            }
            Some(signal) = chat_error_stream.next() => {
                if let Ok(args) = signal.args() {
                    let conv_id = session_map.remove(&args.session);
                    let msg = args.msg.to_string();
                    if let Some(secs) = parse_rate_limit(&msg) {
                        if let Some(conv_id) = conv_id {
                            let _ = chat_msg_tx.send(ChatMsg::RateLimit { conv_id, secs }).await;
                        }
                    } else if let Some(conv_id) = conv_id {
                        let _ = chat_msg_tx.send(ChatMsg::Chunk {
                            conv_id,
                            text: msg.clone(),
                        }).await;
                        let _ = chat_msg_tx.send(ChatMsg::Done { conv_id, full_text: msg }).await;
                    }
                }
            }
            else => break,
        }
    }
    Ok(())
}

/// Parse "rate_limit:<secs>:<text>" → secs.
fn parse_rate_limit(msg: &str) -> Option<u64> {
    let parts: Vec<&str> = msg.split(':').collect();
    if parts.first().map(|s| *s == "rate_limit").unwrap_or(false) && parts.len() >= 2 {
        parts[1].parse::<u64>().ok()
    } else {
        None
    }
}
