use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use crate::{ChatRole, ChatTurn};

#[derive(Debug, thiserror::Error)]
#[error("rate limited: retry after {retry_after_secs}s")]
pub struct RateLimitError {
    pub retry_after_secs: u64,
}

/// Extract the retry delay in seconds from a Gemini 429 JSON body.
/// Reads `google.rpc.RetryInfo.retryDelay`; falls back to parsing the
/// message text; defaults to 60s if neither is found.
pub fn parse_retry_secs(body: &str) -> u64 {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(details) = v["error"]["details"].as_array() {
            for d in details {
                if d["@type"].as_str()
                    == Some("type.googleapis.com/google.rpc.RetryInfo")
                {
                    if let Some(s) = d["retryDelay"]
                        .as_str()
                        .and_then(|s| s.trim_end_matches('s').parse::<f64>().ok())
                    {
                        return s.ceil() as u64;
                    }
                }
            }
        }
        if let Some(msg) = v["error"]["message"].as_str() {
            if let Some(secs) = parse_retry_from_message(msg) {
                return secs;
            }
        }
    }
    parse_retry_from_message(body).unwrap_or(60)
}

fn parse_retry_from_message(msg: &str) -> Option<u64> {
    let rest = msg.split("Please retry in ").nth(1)?;
    let end = rest.find('s')?;
    rest[..end].trim().parse::<f64>().ok().map(|s| s.ceil() as u64)
}

pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    name: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "supportedGenerationMethods", default)]
    supported_generation_methods: Vec<String>,
}

#[derive(Deserialize)]
struct StreamResponse {
    candidates: Option<Vec<Candidate>>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<Content>,
}

#[derive(Deserialize)]
struct Content {
    parts: Option<Vec<Part>>,
}

#[derive(Deserialize)]
struct Part {
    text: Option<String>,
}

pub struct GeminiClient {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self { client: Client::new(), api_key, model }
    }

    /// Fetch all models that support generateContent, sorted by display name.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            self.api_key
        );
        let client = &self.client;
        let resp = client.get(&url).send().await.context("listing Gemini models")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini models API {status}: {text}");
        }

        let body: ModelsResponse = resp.json().await.context("parsing models response")?;
        let mut models: Vec<ModelInfo> = body
            .models
            .into_iter()
            .filter(|m| {
                m.supported_generation_methods
                    .iter()
                    .any(|s| s == "generateContent")
            })
            .map(|m| ModelInfo {
                id: m.name.strip_prefix("models/").unwrap_or(&m.name).to_owned(),
                display_name: m.display_name,
            })
            .collect();
        models.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(models)
    }

    /// Send a multi-turn conversation and stream each text chunk to `tx`.
    pub async fn chat_streaming(
        &self,
        turns: &[ChatTurn],
        tx: &async_channel::Sender<String>,
    ) -> Result<()> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            self.model, self.api_key
        );
        let contents: Vec<serde_json::Value> = turns.iter().map(|t| {
            let role = match t.role { ChatRole::User => "user", ChatRole::Assistant => "model" };
            serde_json::json!({"role": role, "parts": [{"text": t.content}]})
        }).collect();
        let body = serde_json::json!({
            "contents": contents,
            "generationConfig": {"maxOutputTokens": 32768}
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("sending Gemini request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RateLimitError { retry_after_secs: parse_retry_secs(&text) }.into());
            }
            anyhow::bail!("Gemini API {status}: {text}");
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
                    if let Ok(resp) = serde_json::from_str::<StreamResponse>(data) {
                        for candidate in resp.candidates.unwrap_or_default() {
                            if let Some(content) = candidate.content {
                                for part in content.parts.unwrap_or_default() {
                                    if let Some(text) = part.text {
                                        if tx.send(text).await.is_err() { return Ok(()); }
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

    /// Send a multi-turn conversation and collect the full response.
    pub async fn chat(&self, turns: &[ChatTurn]) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            self.model, self.api_key
        );
        let contents: Vec<serde_json::Value> = turns.iter().map(|t| {
            let role = match t.role { ChatRole::User => "user", ChatRole::Assistant => "model" };
            serde_json::json!({"role": role, "parts": [{"text": t.content}]})
        }).collect();
        let body = serde_json::json!({
            "contents": contents,
            "generationConfig": {"maxOutputTokens": 32768}
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("sending Gemini request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RateLimitError { retry_after_secs: parse_retry_secs(&text) }.into());
            }
            anyhow::bail!("Gemini API {status}: {text}");
        }

        let mut stream = response.bytes_stream();
        let mut result = String::new();
        let mut line_buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("reading SSE stream")?;
            line_buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(nl) = line_buf.find('\n') {
                let line = line_buf[..nl].trim_end_matches('\r').to_owned();
                line_buf.drain(..=nl);
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(resp) = serde_json::from_str::<StreamResponse>(data) {
                        for candidate in resp.candidates.unwrap_or_default() {
                            if let Some(content) = candidate.content {
                                for part in content.parts.unwrap_or_default() {
                                    if let Some(text) = part.text { result.push_str(&text); }
                                }
                            }
                        }
                    }
                    debug!(bytes = data.len(), "SSE chunk processed");
                }
            }
        }
        Ok(result)
    }

    /// Single-prompt streaming (convenience wrapper).
    pub async fn ask_streaming(&self, prompt: &str, tx: &async_channel::Sender<String>) -> Result<()> {
        self.chat_streaming(&[ChatTurn::user(prompt)], tx).await
    }

    /// Single-prompt blocking (convenience wrapper).
    pub async fn ask(&self, prompt: &str) -> Result<String> {
        self.chat(&[ChatTurn::user(prompt)]).await
    }
}
