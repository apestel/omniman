mod chat;
mod prefs;
mod window;

use std::{sync::Arc, time::Duration};

use gtk4::glib;
use gtk4::prelude::*;
use omniman_ai::ChatTurn;
use omniman_core::{
    ipc::OmnimanProxy,
    types::{ClipEntry, Hit},
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

    let (clip_content_tx, clip_content_rx) = async_channel::bounded::<String>(8);
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
        let rx = result_rx.lock().unwrap().take().expect("activate once");
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
    clip_content_rx: Arc<std::sync::Mutex<Option<async_channel::Receiver<String>>>>,
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

/// Stream turns through a client, forwarding chunks to the UI channel.
/// Returns the accumulated full text.
async fn do_stream<F, Fut>(
    ask: F,
    conv_id: i64,
    msg_tx: &async_channel::Sender<ChatMsg>,
) -> String
where
    F: FnOnce(async_channel::Sender<String>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let (chunk_tx, chunk_rx) = async_channel::bounded::<String>(64);
    let fwd_tx = msg_tx.clone();

    // Accumulate + forward on a separate task so the client future can run
    // concurrently with the forwarder.
    let accumulator = tokio::spawn(async move {
        let mut acc = String::new();
        while let Ok(chunk) = chunk_rx.recv().await {
            acc.push_str(&chunk);
            let _ = fwd_tx.send(ChatMsg::Chunk { conv_id, text: chunk }).await;
        }
        acc
    });

    let result = ask(chunk_tx.clone()).await;
    drop(chunk_tx); // signal accumulator that stream ended

    let accumulated = accumulator.await.unwrap_or_default();

    if let Err(e) = result {
        if let Some(rl) = e.downcast_ref::<omniman_ai::RateLimitError>() {
            let _ = msg_tx.send(ChatMsg::RateLimit { conv_id, secs: rl.retry_after_secs }).await;
            return String::new();
        }
        let err_text = format!("Error: {e}");
        let _ = msg_tx.send(ChatMsg::Chunk { conv_id, text: err_text.clone() }).await;
        return err_text;
    }

    accumulated
}

async fn connect_and_serve(
    query_rx: &async_channel::Receiver<String>,
    result_tx: &async_channel::Sender<Vec<Hit>>,
    show_tx: &async_channel::Sender<()>,
    clip_req_rx: &async_channel::Receiver<()>,
    clip_result_tx: &async_channel::Sender<Vec<ClipEntry>>,
    chat_req_rx: &async_channel::Receiver<ChatReq>,
    chat_msg_tx: &async_channel::Sender<ChatMsg>,
    clip_content_rx: &async_channel::Receiver<String>,
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
            Ok(content) = clip_content_rx.recv() => {
                let kind = if content.starts_with("file://") { "Uri" } else { "Text" };
                let _ = proxy.store_clip_entry(&kind, &content, "text/plain").await;
            }
            Ok(req) = chat_req_rx.recv() => {
                let config = omniman_core::config::Config::load().unwrap_or_default();
                let oai: Option<Arc<omniman_ai::OpenAiClient>> =
                    match (&config.ai.openai_endpoint, &config.ai.openai_key) {
                        (Some(ep), Some(key)) if !ep.is_empty() && !key.is_empty() => Some(
                            Arc::new(omniman_ai::OpenAiClient::new(
                                key.clone(), ep.clone(), config.ai.openai_model.clone(),
                            )),
                        ),
                        _ => None,
                    };
                let gem: Option<Arc<omniman_ai::GeminiClient>> = if oai.is_none() {
                    omniman_ai::GeminiClient::from_env(&config.ai.model).ok().map(Arc::new)
                } else {
                    None
                };

                let msg_tx = chat_msg_tx.clone();
                let proxy_clone = proxy.clone();

                tokio::spawn(async move {
                    match req {
                        ChatReq::NewTurn { conv_id, history } => {
                            tracing::debug!(
                                conv_id,
                                turns = history.len(),
                                has_oai = oai.is_some(),
                                has_gem = gem.is_some(),
                                "NewTurn worker"
                            );
                            let _ = msg_tx.send(ChatMsg::Start { conv_id }).await;

                            let full_text = if let Some(client) = oai {
                                let h = history.clone();
                                do_stream(
                                    |tx| async move { client.chat_streaming(&h, &tx).await },
                                    conv_id, &msg_tx,
                                ).await
                            } else if let Some(client) = gem {
                                let h = history.clone();
                                do_stream(
                                    |tx| async move { client.chat_streaming(&h, &tx).await },
                                    conv_id, &msg_tx,
                                ).await
                            } else {
                                // D-Bus fallback: format full history as a single prompt
                                // so the daemon at least has the conversation context.
                                let prompt = history.iter().map(|t| {
                                    let role = match t.role {
                                        omniman_ai::ChatRole::User => "User",
                                        omniman_ai::ChatRole::Assistant => "Assistant",
                                    };
                                    format!("{role}: {}", t.content)
                                }).collect::<Vec<_>>().join("\n\n");
                                tracing::debug!(conv_id, "D-Bus fallback, prompt turns = {}", history.len());
                                let resp = proxy_clone
                                    .ask_ai(&prompt)
                                    .await
                                    .unwrap_or_else(|e| format!("D-Bus error: {e}"));
                                if resp.contains("429") {
                                    let json_start = resp.find('{').unwrap_or(resp.len());
                                    let secs = omniman_ai::parse_retry_secs(&resp[json_start..]);
                                    let _ = msg_tx.send(ChatMsg::RateLimit { conv_id, secs }).await;
                                    String::new()
                                } else {
                                    let _ = msg_tx.send(ChatMsg::Chunk { conv_id, text: resp.clone() }).await;
                                    resp
                                }
                            };

                            let _ = msg_tx.send(ChatMsg::Done { conv_id, full_text }).await;
                        }

                        ChatReq::Summarize { conv_id, user_msg, assistant_msg } => {
                            let prompt = format!(
                                "Summarize the topic of this exchange in 3 to 5 words.\n\
                                 Return ONLY the title text — no quotes, no trailing punctuation.\n\n\
                                 User: {user_msg}\nAssistant: {assistant_msg}"
                            );
                            let raw = if let Some(client) = oai {
                                // collect streaming result for title
                                let (tx, rx) = async_channel::bounded::<String>(64);
                                let acc = tokio::spawn(async move {
                                    let mut s = String::new();
                                    while let Ok(c) = rx.recv().await { s.push_str(&c); }
                                    s
                                });
                                let _ = client.chat_streaming(&[ChatTurn::user(&prompt)], &tx).await;
                                drop(tx);
                                acc.await.unwrap_or_default()
                            } else if let Some(client) = gem {
                                client.chat(&[ChatTurn::user(&prompt)]).await.unwrap_or_default()
                            } else {
                                proxy_clone.ask_ai(&prompt).await.unwrap_or_default()
                            };
                            let title = raw.trim().trim_matches('"').trim_matches('\'').to_owned();
                            if !title.is_empty() {
                                let _ = msg_tx.send(ChatMsg::Title { conv_id, title }).await;
                            }
                        }
                    }
                });
            }
            else => break,
        }
    }
    Ok(())
}
