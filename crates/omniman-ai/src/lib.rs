pub mod client;
pub mod heuristic;
pub mod openai;

pub use client::{GeminiClient, ModelInfo, RateLimitError, parse_retry_secs};
pub use heuristic::is_question;
pub use openai::OpenAiClient;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub content: String,
}

impl ChatTurn {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: ChatRole::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: ChatRole::Assistant, content: content.into() }
    }
}
