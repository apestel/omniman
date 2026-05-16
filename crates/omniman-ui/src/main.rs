mod prefs;
mod window;

use std::{sync::Arc, time::Duration};

use gtk4::glib;
use gtk4::prelude::*;
use omniman_core::{
    ipc::OmnimanProxy,
    types::{ClipEntry, Hit},
};

pub enum AiMsg {
    Start,
    Chunk(String),
    Done,
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
    let (ai_req_tx, ai_req_rx) = async_channel::bounded::<String>(4);
    let (ai_result_tx, ai_result_rx) = async_channel::bounded::<AiMsg>(64);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(dbus_worker(
            query_rx,
            result_tx,
            show_tx,
            clip_req_rx,
            clip_result_tx,
            ai_req_rx,
            ai_result_tx,
        ));
    });

    let result_rx = Arc::new(std::sync::Mutex::new(Some(result_rx)));
    let show_rx = Arc::new(std::sync::Mutex::new(Some(show_rx)));
    let clip_result_rx = Arc::new(std::sync::Mutex::new(Some(clip_result_rx)));
    let ai_result_rx = Arc::new(std::sync::Mutex::new(Some(ai_result_rx)));

    let app = libadwaita::Application::builder()
        .application_id("org.adrien.Omniman")
        .build();

    app.connect_activate(move |app| {
        let rx = result_rx.lock().unwrap().take().expect("activate once");
        let srx = show_rx.lock().unwrap().take().expect("activate once");
        let crx = clip_result_rx.lock().unwrap().take().expect("activate once");
        let arx = ai_result_rx.lock().unwrap().take().expect("activate once");
        window::build(
            app,
            query_tx.clone(),
            rx,
            srx,
            clip_req_tx.clone(),
            crx,
            ai_req_tx.clone(),
            arx,
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
    ai_req_rx: async_channel::Receiver<String>,
    ai_result_tx: async_channel::Sender<AiMsg>,
) {
    loop {
        match connect_and_serve(
            &query_rx,
            &result_tx,
            &show_tx,
            &clip_req_rx,
            &clip_result_tx,
            &ai_req_rx,
            &ai_result_tx,
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
    ai_req_rx: &async_channel::Receiver<String>,
    ai_result_tx: &async_channel::Sender<AiMsg>,
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
    let mut clip_changed_stream = proxy.receive_clipboard_changed().await.context("subscribing ClipboardChanged")?;

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
            Ok(prompt) = ai_req_rx.recv() => {
                // Re-read config on every request so model/endpoint changes from
                // the prefs window take effect without restarting the process.
                let config = omniman_core::config::Config::load().unwrap_or_default();
                let oai: Option<Arc<omniman_ai::OpenAiClient>> =
                    match (&config.ai.openai_endpoint, &config.ai.openai_key) {
                        (Some(ep), Some(key)) if !ep.is_empty() && !key.is_empty() => Some(
                            Arc::new(omniman_ai::OpenAiClient::new(
                                key.clone(),
                                ep.clone(),
                                config.ai.openai_model.clone(),
                            )),
                        ),
                        _ => None,
                    };
                let gem: Option<Arc<omniman_ai::GeminiClient>> = if oai.is_none() {
                    omniman_ai::GeminiClient::from_env(&config.ai.model).ok().map(Arc::new)
                } else {
                    None
                };
                let result_tx = ai_result_tx.clone();
                let proxy_clone = proxy.clone();
                tokio::spawn(async move {
                    let _ = result_tx.send(AiMsg::Start).await;

                    // Helper: stream from any client that implements ask_streaming
                    // and forward chunks to the UI channel.
                    async fn stream_and_forward<F, Fut>(
                        ask: F,
                        result_tx: &async_channel::Sender<AiMsg>,
                    ) where
                        F: FnOnce(async_channel::Sender<String>) -> Fut,
                        Fut: std::future::Future<Output = anyhow::Result<()>>,
                    {
                        let (chunk_tx, chunk_rx) = async_channel::bounded::<String>(64);
                        let fwd_tx = result_tx.clone();
                        let forwarder = tokio::spawn(async move {
                            while let Ok(chunk) = chunk_rx.recv().await {
                                if fwd_tx.send(AiMsg::Chunk(chunk)).await.is_err() {
                                    break;
                                }
                            }
                        });
                        if let Err(e) = ask(chunk_tx.clone()).await {
                            let _ = result_tx.send(AiMsg::Chunk(format!("Error: {e}"))).await;
                        }
                        drop(chunk_tx);
                        let _ = forwarder.await;
                    }

                    if let Some(client) = oai {
                        stream_and_forward(
                            |tx| async move { client.ask_streaming(&prompt, &tx).await },
                            &result_tx,
                        ).await;
                    } else if let Some(client) = gem {
                        stream_and_forward(
                            |tx| async move { client.ask_streaming(&prompt, &tx).await },
                            &result_tx,
                        ).await;
                    } else {
                        let resp = proxy_clone
                            .ask_ai(&prompt)
                            .await
                            .unwrap_or_else(|e| format!("D-Bus error: {e}"));
                        let _ = result_tx.send(AiMsg::Chunk(resp)).await;
                    }
                    let _ = result_tx.send(AiMsg::Done).await;
                });
            }
            else => break,
        }
    }
    Ok(())
}
