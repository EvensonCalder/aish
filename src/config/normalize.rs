use std::collections::HashSet;

use crate::git_remote::{sanitize_git_remote, valid_template_remote_name};

use super::{CompletionConfig, Config, ContextConfig, PasteConfig, PromptConfig, default_aish_dir};

pub fn normalize_config(config: &mut Config) {
    config.shell.backend = config.shell.backend.trim().to_string();
    if config.shell.backend.is_empty() {
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
    if config.paste.preview_lines == 0 || config.paste.preview_lines > 20 {
        config.paste.preview_lines = PasteConfig::default().preview_lines;
    }
    if config.paste.preview_bytes == 0 || config.paste.preview_bytes > 4_096 {
        config.paste.preview_bytes = PasteConfig::default().preview_bytes;
    }
    if config.completion.max_results == 0 {
        config.completion.max_results = CompletionConfig::default().max_results;
    }
    if config.completion.coalesce_ms > 1_000 {
        config.completion.coalesce_ms = CompletionConfig::default().coalesce_ms;
    }
    if config.completion.display_delay_ms > 1_000 {
        config.completion.display_delay_ms = CompletionConfig::default().display_delay_ms;
    }
    if config.completion.match_threshold_percent > 100 {
        config.completion.match_threshold_percent =
            CompletionConfig::default().match_threshold_percent;
    }
    if config.completion.typo_threshold_percent > 100 {
        config.completion.typo_threshold_percent =
            CompletionConfig::default().typo_threshold_percent;
    }
    if let Some(mode) = config.completion.mode {
        config.completion.set_mode(mode);
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
    config.sync.remote = sanitize_git_remote(&config.sync.remote).unwrap_or_default();
    config.sync.schedule = config.sync.schedule.trim().to_string();
    for remote in &mut config.template_sharing.remotes {
        remote.name = remote.name.trim().to_string();
        remote.remote = sanitize_git_remote(&remote.remote).unwrap_or_default();
    }
    config.template_sharing.remotes.retain(|remote| {
        !remote.name.is_empty()
            && valid_template_remote_name(&remote.name)
            && !remote.remote.is_empty()
    });
    let mut names = HashSet::new();
    config
        .template_sharing
        .remotes
        .retain(|remote| names.insert(remote.name.clone()));
}
