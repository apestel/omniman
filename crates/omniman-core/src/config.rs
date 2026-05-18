use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub index: IndexConfig,
    pub clipboard: ClipboardConfig,
    pub ai: AiConfig,
    pub hotkey: HotkeyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexConfig {
    pub exclude_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// Max number of entries to keep.
    pub history_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub model: String,
    /// Gemini API key. Managed via the UI preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini_api_key: Option<String>,
    /// Base URL for an OpenAI-compatible endpoint (e.g. https://api.openai.com/v1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_endpoint: Option<String>,
    /// API key for the OpenAI-compatible endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_key: Option<String>,
    /// Model name to use with the OpenAI-compatible endpoint.
    pub openai_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub shortcut: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            index: IndexConfig::default(),
            clipboard: ClipboardConfig::default(),
            ai: AiConfig::default(),
            hotkey: HotkeyConfig::default(),
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            exclude_dirs: vec![
                ".cache".into(),
                ".git".into(),
                "node_modules".into(),
                "target".into(),
                ".venv".into(),
                "__pycache__".into(),
                ".cargo".into(),
            ],
        }
    }
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self { history_limit: 500 }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            model: "gemini-2.5-flash".into(),
            gemini_api_key: None,
            openai_endpoint: None,
            openai_key: None,
            openai_model: "gpt-4o".into(),
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            shortcut: "Ctrl+Space".into(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path();
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            Ok(toml::from_str::<Self>(&raw)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(&path, raw)?;
        Ok(())
    }

    pub fn path() -> PathBuf {
        config_path()
    }

    pub fn data_dir() -> PathBuf {
        directories::ProjectDirs::from("org", "adrien", "omniman")
            .map(|d| d.data_local_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("/tmp/omniman"))
    }
}

fn config_path() -> PathBuf {
    directories::ProjectDirs::from("org", "adrien", "omniman")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("/tmp/omniman-config.toml"))
}
