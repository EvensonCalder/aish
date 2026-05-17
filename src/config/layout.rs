use std::path::PathBuf;

use anyhow::Result;

use super::create_private_dir_all;

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
            create_private_dir_all(dir)?;
        }
        Ok(())
    }
}
