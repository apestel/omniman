use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

#[derive(Deserialize)]
struct ChatChunk {
    choices: Option<Vec<Choice>>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Option<Delta>,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

pub struct OpenAiClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_owned();
        Self { client: Client::new(), api_key, base_url, model }
    }

    pub async fn ask_streaming(
        &self,
        prompt: &str,
        tx: &async_channel::Sender<String>,
    ) -> Result<()> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": true,
            "max_tokens": 32768
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("sending OpenAI request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API {status}: {text}");
        }

        let mut stream = response.bytes_stream();
        let mut line_buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("reading SSE stream")?;
            line_buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = line_buf.find('\n') {
                let line = line_buf[..nl].trim_end_matches('\r').to_owned();
                line_buf.drain(..=nl);

                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        return Ok(());
                    }
                    if let Ok(chunk) = serde_json::from_str::<ChatChunk>(data) {
                        for choice in chunk.choices.unwrap_or_default() {
                            if let Some(delta) = choice.delta {
                                if let Some(text) = delta.content {
                                    if tx.send(text).await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    debug!(bytes = data.len(), "SSE chunk processed");
                }
            }
        }

        Ok(())
    }
}
