use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub shell: ShellConfig,
    pub prompt: PromptConfig,
    pub storage: StorageConfig,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_spec_basics() {
        let config = Config::default();
        assert_eq!(config.shell.backend, "auto");
        assert_eq!(config.prompt.draft, "{user}@{host} {cwd} > ");
        assert_eq!(config.prompt.history, "{user}@{host} {cwd} $ ");
        assert_eq!(config.prompt.ai, "{user}@{host} {cwd} % ");
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
        };

        normalize_config(&mut config);

        assert_eq!(config, Config::default());
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
    fn config_roundtrips_through_json_for_future_jsonl_storage() {
        let config = Config::default();

        let raw = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed, config);
    }

    #[test]
    fn aish_home_environment_overrides_default_root() {
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
}
