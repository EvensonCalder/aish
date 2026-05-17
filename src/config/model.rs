use std::path::PathBuf;

use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer, Serialize};

use super::default_aish_dir;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub shell: ShellConfig,
    pub prompt: PromptConfig,
    pub storage: StorageConfig,
    pub draft: DraftConfig,
    pub editor: EditorConfig,
    pub paste: PasteConfig,
    pub completion: CompletionConfig,
    pub ai: AiConfig,
    pub context: ContextConfig,
    pub encryption: EncryptionConfig,
    pub sync: SyncConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    pub backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PromptConfig {
    pub draft: String,
    pub history: String,
    pub ai: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub home: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DraftConfig {
    pub persist: bool,
    pub sync: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorConfig {
    pub command: Vec<String>,
    pub execute_after_save: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PasteConfig {
    pub multiline: String,
    pub confirm_execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompletionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<CompletionMode>,
    pub enabled: bool,
    pub max_results: usize,
    pub coalesce_ms: u64,
    pub display_delay_ms: u64,
    pub ignore_spaces: bool,
    pub template_first: bool,
    pub inline: bool,
    pub fuzzy: bool,
    pub tab_accept: CompletionTabAccept,
    pub match_threshold_percent: usize,
    pub typo_threshold_percent: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionMode {
    Auto,
    Tab,
    Off,
}

impl CompletionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tab => "tab",
            Self::Off => "off",
        }
    }
}

impl CompletionConfig {
    pub fn mode(&self) -> CompletionMode {
        if let Some(mode) = self.mode {
            mode
        } else if !self.enabled {
            CompletionMode::Off
        } else if self.inline {
            CompletionMode::Auto
        } else {
            CompletionMode::Tab
        }
    }

    pub fn set_mode(&mut self, mode: CompletionMode) {
        self.mode = Some(mode);
        match mode {
            CompletionMode::Auto => {
                self.enabled = true;
                self.inline = true;
            }
            CompletionMode::Tab => {
                self.enabled = true;
                self.inline = false;
            }
            CompletionMode::Off => {
                self.enabled = false;
                self.inline = false;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionTabAccept {
    Full,
    #[default]
    Word,
}

impl CompletionTabAccept {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Word => "word",
        }
    }
}

impl<'de> Deserialize<'de> for CompletionTabAccept {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.trim() {
            "" => Ok(Self::Word),
            "full" => Ok(Self::Full),
            "word" => Ok(Self::Word),
            other => Err(de::Error::invalid_value(
                Unexpected::Str(other),
                &"\"full\" or \"word\"",
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    pub model: String,
    pub base_url: String,
    pub env_key: String,
    #[serde(skip)]
    pub api_key_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextConfig {
    pub enabled: bool,
    pub confirm: bool,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EncryptionConfig {
    pub enabled: bool,
    pub key_fingerprint: String,
    /// Deprecated compatibility field. New writes should persist
    /// `key_fingerprint` after resolving any user-facing key selector.
    pub recipient: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    pub remote: String,
    pub enabled: bool,
    pub schedule: String,
    pub ai: bool,
    pub history: bool,
    pub templates: bool,
    pub drafts: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            confirm: true,
            max_bytes: 65_536,
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            backend: "auto".to_string(),
        }
    }
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            draft: "{user}@{host} {cwd} > ".to_string(),
            history: "{user}@{host} {cwd} $ ".to_string(),
            ai: "{user}@{host} {cwd} % ".to_string(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            home: default_aish_dir(),
        }
    }
}

impl Default for DraftConfig {
    fn default() -> Self {
        Self {
            persist: true,
            sync: false,
        }
    }
}

impl Default for PasteConfig {
    fn default() -> Self {
        Self {
            multiline: "editor".to_string(),
            confirm_execute: true,
        }
    }
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            mode: None,
            enabled: true,
            max_results: 5,
            coalesce_ms: 50,
            display_delay_ms: 120,
            ignore_spaces: true,
            template_first: true,
            inline: true,
            fuzzy: true,
            tab_accept: CompletionTabAccept::Word,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        }
    }
}
