use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer, Serialize};

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
    pub max_results: usize,
    pub ignore_spaces: bool,
    pub template_first: bool,
    pub inline: bool,
    pub tab_accept: CompletionTabAccept,
    pub match_threshold_percent: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionTabAccept {
    #[default]
    Full,
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
            "" | "full" => Ok(Self::Full),
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
            max_results: 5,
            ignore_spaces: true,
            template_first: true,
            inline: true,
            tab_accept: CompletionTabAccept::Full,
            match_threshold_percent: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryLayout {
    pub root: PathBuf,
    pub config: PathBuf,
    pub history: PathBuf,
    pub regular_history: PathBuf,
    pub ai_history: PathBuf,
    pub draft_history: PathBuf,
    pub notes: PathBuf,
    pub templates: PathBuf,
    pub template_store: PathBuf,
    pub secrets: PathBuf,
    pub logs: PathBuf,
    pub events: PathBuf,
    pub cache: PathBuf,
    pub runtime_cache: PathBuf,
    pub gitignore: PathBuf,
}

impl DirectoryLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let history = root.join("history");
        let templates = root.join("templates");
        let logs = root.join("logs");
        let cache = root.join("cache");

        Self {
            config: root.join("config.toml"),
            regular_history: history.join("regular.jsonl"),
            ai_history: history.join("ai.jsonl"),
            draft_history: history.join("draft.jsonl"),
            notes: history.join("notes.jsonl"),
            template_store: templates.join("templates.jsonl"),
            secrets: root.join("secrets"),
            events: logs.join("events.jsonl"),
            runtime_cache: cache.join("runtime"),
            gitignore: root.join(".gitignore"),
            root,
            history,
            templates,
            logs,
            cache,
        }
    }

    pub fn create_dirs(&self) -> Result<()> {
        for dir in [
            &self.root,
            &self.history,
            &self.templates,
            &self.secrets,
            &self.logs,
            &self.cache,
            &self.runtime_cache,
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create directory {}", dir.display()))?;
        }
        Ok(())
    }
}

pub fn default_aish_dir() -> PathBuf {
    if let Ok(home) = std::env::var("AISH_HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aish")
}

pub fn runtime_aish_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("AISH_HOME") {
        let home = home.trim();
        if !home.is_empty() {
            let home = PathBuf::from(home);
            if !home.is_absolute() {
                bail!("AISH_HOME must be set to an absolute path");
            }
            return Ok(home);
        }
    }

    let Some(home) = std::env::var_os("HOME") else {
        bail!("AISH_HOME or HOME must be set to an absolute path");
    };
    let home = PathBuf::from(home);
    if home.as_os_str().is_empty() || !home.is_absolute() {
        bail!("AISH_HOME or HOME must be set to an absolute path");
    }
    Ok(home.join(".aish"))
}

pub fn init_default_layout(root: impl Into<PathBuf>) -> Result<(DirectoryLayout, Config)> {
    let layout = DirectoryLayout::new(root);
    layout.create_dirs()?;
    let config = load_or_create_config(&layout.config, &layout.root)?;
    Ok((layout, config))
}

pub fn load_or_create_config(path: &Path, root: &Path) -> Result<Config> {
    if path.exists() {
        return load_config(path);
    }

    let config = Config {
        storage: StorageConfig {
            home: root.to_path_buf(),
        },
        ..Config::default()
    };
    save_config(path, &config)?;
    Ok(config)
}

pub fn load_config(path: &Path) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let mut config: Config =
        toml::from_str(&raw).with_context(|| format!("invalid config {}", path.display()))?;
    normalize_config(&mut config);
    Ok(config)
}

pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    let raw = toml::to_string_pretty(config).context("failed to serialize config")?;
    fs::write(path, raw).with_context(|| format!("failed to write config {}", path.display()))?;
    Ok(())
}

pub fn normalize_config(config: &mut Config) {
    if config.shell.backend.trim().is_empty() {
        config.shell.backend = "auto".to_string();
    }
    if config.prompt.draft.is_empty() {
        config.prompt.draft = PromptConfig::default().draft;
    }
    if config.prompt.history.is_empty() {
        config.prompt.history = PromptConfig::default().history;
    }
    if config.prompt.ai.is_empty() {
        config.prompt.ai = PromptConfig::default().ai;
    }
    if config.storage.home.as_os_str().is_empty() {
        config.storage.home = default_aish_dir();
    }
    config.editor.command.retain(|part| !part.trim().is_empty());
    if !matches!(
        config.paste.multiline.as_str(),
        "editor" | "execute" | "discard"
    ) {
        config.paste.multiline = PasteConfig::default().multiline;
    }
    if config.completion.max_results == 0 {
        config.completion.max_results = CompletionConfig::default().max_results;
    }
    if config.completion.match_threshold_percent > 100 {
        config.completion.match_threshold_percent =
            CompletionConfig::default().match_threshold_percent;
    }
    config.ai.model = config.ai.model.trim().to_string();
    config.ai.base_url = config.ai.base_url.trim().to_string();
    config.ai.env_key = config.ai.env_key.trim().to_string();
    config.ai.api_key_override = None;
    if config.context.max_bytes == 0 {
        config.context.max_bytes = ContextConfig::default().max_bytes;
    }
    config.encryption.key_fingerprint = config.encryption.key_fingerprint.trim().to_string();
    config.encryption.recipient = config.encryption.recipient.trim().to_string();
    config.sync.remote = config.sync.remote.trim().to_string();
    config.sync.schedule = config.sync.schedule.trim().to_string();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_matches_spec_basics() {
        let config = Config::default();
        assert_eq!(config.shell.backend, "auto");
        assert_eq!(config.prompt.draft, "{user}@{host} {cwd} > ");
        assert_eq!(config.prompt.history, "{user}@{host} {cwd} $ ");
        assert_eq!(config.prompt.ai, "{user}@{host} {cwd} % ");
        assert!(config.draft.persist);
        assert!(!config.draft.sync);
        assert!(config.editor.command.is_empty());
        assert!(!config.editor.execute_after_save);
        assert_eq!(config.paste.multiline, "editor");
        assert!(config.paste.confirm_execute);
        assert_eq!(config.completion.max_results, 5);
        assert!(config.completion.ignore_spaces);
        assert!(config.completion.template_first);
        assert!(config.completion.inline);
        assert_eq!(config.completion.tab_accept, CompletionTabAccept::Full);
        assert_eq!(config.completion.match_threshold_percent, 50);
        assert_eq!(config.ai, AiConfig::default());
        assert_eq!(config.context, ContextConfig::default());
        assert_eq!(config.encryption, EncryptionConfig::default());
        assert_eq!(config.sync, SyncConfig::default());
        assert!(config.storage.home.ends_with(".aish"));
    }

    #[test]
    fn normalize_replaces_empty_values() {
        let mut config = Config {
            shell: ShellConfig {
                backend: "   ".to_string(),
            },
            prompt: PromptConfig {
                draft: String::new(),
                history: String::new(),
                ai: String::new(),
            },
            storage: StorageConfig {
                home: PathBuf::new(),
            },
            draft: DraftConfig::default(),
            editor: EditorConfig {
                command: vec![String::new(), "vim".to_string()],
                execute_after_save: false,
            },
            paste: PasteConfig {
                multiline: "unknown".to_string(),
                confirm_execute: true,
            },
            completion: CompletionConfig {
                max_results: 0,
                ignore_spaces: true,
                template_first: true,
                inline: true,
                tab_accept: CompletionTabAccept::Full,
                match_threshold_percent: 101,
            },
            ai: AiConfig {
                model: "  gpt-test  ".to_string(),
                base_url: "  https://example.invalid/v1  ".to_string(),
                env_key: "  OPENAI_API_KEY  ".to_string(),
                api_key_override: Some("must-not-persist".to_string()),
            },
            context: ContextConfig {
                enabled: false,
                confirm: false,
                max_bytes: 0,
            },
            encryption: EncryptionConfig {
                enabled: true,
                key_fingerprint: "  ABCDEF0123456789ABCDEF0123456789ABCDEF01  ".to_string(),
                recipient: "  test@example.invalid  ".to_string(),
            },
            sync: SyncConfig {
                remote: "  git@example.invalid:aish.git  ".to_string(),
                enabled: true,
                schedule: "  0 * * * *  ".to_string(),
                ai: true,
                history: false,
                templates: true,
                drafts: false,
            },
        };

        normalize_config(&mut config);

        let mut expected = Config::default();
        expected.editor.command = vec!["vim".to_string()];
        expected.ai = AiConfig {
            model: "gpt-test".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            api_key_override: None,
        };
        expected.context = ContextConfig {
            enabled: false,
            confirm: false,
            max_bytes: 65_536,
        };
        expected.encryption = EncryptionConfig {
            enabled: true,
            key_fingerprint: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string(),
            recipient: "test@example.invalid".to_string(),
        };
        expected.completion.match_threshold_percent = 50;
        expected.sync = SyncConfig {
            remote: "git@example.invalid:aish.git".to_string(),
            enabled: true,
            schedule: "0 * * * *".to_string(),
            ai: true,
            history: false,
            templates: true,
            drafts: false,
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn first_run_creates_layout_and_default_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aish-home");

        let (layout, config) = init_default_layout(&root).unwrap();

        assert_eq!(layout.root, root);
        assert!(layout.config.exists());
        assert!(layout.history.is_dir());
        assert!(layout.templates.is_dir());
        assert!(layout.secrets.is_dir());
        assert!(layout.logs.is_dir());
        assert_eq!(layout.events, root.join("logs/events.jsonl"));
        assert!(layout.runtime_cache.is_dir());
        assert_eq!(config.storage.home, layout.root);
    }

    #[test]
    fn invalid_config_has_readable_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, "[shell\nbackend = true").unwrap();

        let err = load_config(&path).unwrap_err().to_string();

        assert!(err.contains("invalid config"));
        assert!(err.contains("config.toml"));
    }

    #[test]
    fn completion_tab_accept_empty_normalizes_to_default() {
        let raw = r#"
            [completion]
            tab_accept = ""
        "#;

        let config: Config = toml::from_str(raw).unwrap();

        assert_eq!(config.completion.tab_accept, CompletionTabAccept::Full);
    }

    #[test]
    fn completion_tab_accept_rejects_unsupported_modes() {
        let raw = r#"
            [completion]
            tab_accept = "line"
        "#;

        let err = toml::from_str::<Config>(raw).unwrap_err().to_string();

        assert!(err.contains("invalid value"));
        assert!(err.contains("full"));
        assert!(err.contains("word"));
    }

    #[test]
    fn config_roundtrips_through_json_for_future_jsonl_storage() {
        let config = Config::default();

        let raw = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed, config);
    }

    #[test]
    fn aish_home_environment_overrides_default_root() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", temp.path());
        }

        let root = default_aish_dir();

        unsafe {
            std::env::remove_var("AISH_HOME");
        }
        assert_eq!(root, temp.path());
    }

    #[test]
    fn runtime_aish_dir_rejects_missing_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("AISH_HOME");
            std::env::set_var("HOME", "");
        }

        let err = runtime_aish_dir().unwrap_err().to_string();

        unsafe {
            std::env::remove_var("HOME");
        }
        assert!(err.contains("AISH_HOME or HOME must be set to an absolute path"));
    }

    #[test]
    fn runtime_aish_dir_empty_aish_home_falls_back_to_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", "");
            std::env::set_var("HOME", temp.path());
        }

        let root = runtime_aish_dir().unwrap();

        unsafe {
            std::env::remove_var("AISH_HOME");
            std::env::remove_var("HOME");
        }
        assert_eq!(root, temp.path().join(".aish"));
    }

    #[test]
    fn runtime_aish_dir_rejects_relative_aish_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", "relative-aish");
        }

        let err = runtime_aish_dir().unwrap_err().to_string();

        unsafe {
            std::env::remove_var("AISH_HOME");
        }
        assert!(err.contains("AISH_HOME must be set to an absolute path"));
    }
}
