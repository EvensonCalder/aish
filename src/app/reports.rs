use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

use crate::ai::read_api_key_from_env;
use crate::config::CompletionConfig;
use crate::editor::resolve_editor_command;
use crate::encryption::gpg_program;
use crate::keybindings::default_keybindings;

use super::{AppState, configured_encryption_key, prompt_command::write_prompt_config};

pub(super) fn write_doctor_report(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish doctor")?;
    writeln!(out, "mode={}", state.mode.symbol())?;
    writeln!(
        out,
        "last_status={}",
        state
            .last_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "none".to_string())
    )?;
    writeln!(
        out,
        "cwd={}",
        state
            .current_cwd
            .as_ref()
            .map(|cwd| cwd.display().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )?;
    writeln!(out, "draft_persist={}", state.draft_persist)?;
    writeln!(
        out,
        "regular_history_entries={}",
        state.regular_history.len()
    )?;
    writeln!(out, "ai_sessions={}", state.ai_sessions.len())?;
    writeln!(out, "ai_commands={}", state.ai_command_indices.len())?;
    writeln!(out, "output_ring_entries={}", state.output_ring.len())?;
    writeln!(out, "backend_shell={}", backend_shell_value(state))?;
    writeln!(out, "pty=ok")?;
    writeln!(out, "gpg={}", gpg_status(state))?;
    writeln!(out, "git=not_configured")?;
    writeln!(out, "fzf=external")?;
    write_ai_runtime_status(state, out)?;
    write_encryption_sync_status(state, out)?;
    write_editor_resolution(out, state)?;
    write_path_status(out, "regular_history_path", &state.regular_history_path)?;
    write_path_status(out, "notes_path", &state.notes_path)?;
    write_path_status(out, "draft_history_path", &state.draft_history_path)?;
    write_path_status(out, "config_path", &state.config_path)?;
    write_path_status(out, "events_path", &state.events_path)?;
    Ok(())
}

pub(super) fn write_status_report(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish status")?;
    writeln!(out, "mode={}", state.mode.symbol())?;
    writeln!(
        out,
        "last_status={}",
        state
            .last_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "none".to_string())
    )?;
    writeln!(
        out,
        "cwd={}",
        state
            .current_cwd
            .as_ref()
            .map(|cwd| cwd.display().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )?;
    writeln!(out, "shell={}", backend_shell_value(state))?;
    write_ai_runtime_status(state, out)?;
    write_encryption_sync_status(state, out)?;
    writeln!(out, "context.enabled={}", state.context_config.enabled)?;
    writeln!(out, "context.confirm={}", state.context_config.confirm)?;
    writeln!(out, "context.max_bytes={}", state.context_config.max_bytes)?;
    write_completion_config_lines(out, &state.completion_config)?;
    writeln!(out, "keybindings={}", default_keybindings().len())?;
    Ok(())
}

pub(super) fn write_encryption_sync_status(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "encryption={}",
        if state.encryption_config.enabled {
            "on"
        } else {
            "off"
        }
    )?;
    writeln!(
        out,
        "encryption.key_fingerprint={}",
        config_value(&state.encryption_config.key_fingerprint)
    )?;
    if !state.encryption_config.recipient.trim().is_empty() {
        writeln!(
            out,
            "encryption.legacy_recipient={}",
            config_value(&state.encryption_config.recipient)
        )?;
    }
    writeln!(
        out,
        "encryption.writer={}",
        if state.encrypted_writer.is_some() {
            "async"
        } else {
            "sync"
        }
    )?;
    writeln!(
        out,
        "encryption.storage_unlocked={}",
        state.encrypted_storage_unlocked
    )?;
    if let Some(message) = &state.encrypted_startup_unlock_message {
        writeln!(out, "encryption.unlock_status={message}")?;
    }
    writeln!(
        out,
        "encryption.last_write_error={}",
        config_value(state.last_encrypted_write_error.as_deref().unwrap_or(""))
    )?;
    writeln!(out, "sync.enabled={}", state.sync_config.enabled)?;
    writeln!(
        out,
        "sync.remote={}",
        config_value(&state.sync_config.remote)
    )?;
    writeln!(
        out,
        "sync.schedule={}",
        config_value(&state.sync_config.schedule)
    )?;
    writeln!(out, "sync.ai={}", state.sync_config.ai)?;
    writeln!(out, "sync.history={}", state.sync_config.history)?;
    writeln!(out, "sync.templates={}", state.sync_config.templates)?;
    writeln!(out, "sync.drafts={}", state.sync_config.drafts)?;
    Ok(())
}

pub(super) fn write_config_report(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish config")?;
    write_config_path(out, "config_path", &state.config_path)?;
    writeln!(out, "shell.backend={}", backend_shell_value(state))?;
    writeln!(out, "draft.persist={}", state.draft_persist)?;
    writeln!(
        out,
        "editor.execute_after_save={}",
        state.editor_config.execute_after_save
    )?;
    writeln!(
        out,
        "editor.command={}",
        format_editor_command(&state.editor_config.command)
    )?;
    writeln!(out, "paste.multiline={}", state.paste_config.multiline)?;
    writeln!(
        out,
        "paste.confirm_execute={}",
        state.paste_config.confirm_execute
    )?;
    writeln!(out, "paste.preview={}", state.paste_config.preview)?;
    writeln!(
        out,
        "paste.preview_lines={}",
        state.paste_config.preview_lines
    )?;
    writeln!(
        out,
        "paste.preview_bytes={}",
        state.paste_config.preview_bytes
    )?;
    write_prompt_config(out, &state.prompt_templates)?;
    write_completion_config_lines(out, &state.completion_config)?;
    writeln!(out, "ai.model={}", config_value(&state.ai_config.model))?;
    writeln!(
        out,
        "ai.base_url={}",
        config_value(&state.ai_config.base_url)
    )?;
    writeln!(out, "ai.env_key={}", config_value(&state.ai_config.env_key))?;
    writeln!(out, "context.enabled={}", state.context_config.enabled)?;
    writeln!(out, "context.confirm={}", state.context_config.confirm)?;
    writeln!(out, "context.max_bytes={}", state.context_config.max_bytes)?;
    write_encryption_sync_status(state, out)?;
    write_editor_resolution(out, state)?;
    write_config_path(out, "history.regular", &state.regular_history_path)?;
    write_config_path(out, "history.notes", &state.notes_path)?;
    write_config_path(out, "history.draft", &state.draft_history_path)?;
    write_config_path(out, "templates.store", &state.template_store_path)?;
    Ok(())
}

pub(super) fn write_editor_report(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish editor")?;
    writeln!(
        out,
        "execute_after_save={}",
        state.editor_config.execute_after_save
    )?;
    writeln!(
        out,
        "configured={}",
        format_editor_command(&state.editor_config.command)
    )?;
    write_editor_resolution(out, state)?;
    if state.editor_temp_root.is_some() {
        writeln!(out, "external editor launch is wired to Ctrl-X Ctrl-E")?;
    } else {
        writeln!(out, "editor temp directory is not configured")?;
    }
    Ok(())
}

fn write_completion_config_lines(out: &mut impl Write, config: &CompletionConfig) -> Result<()> {
    writeln!(out, "completion.mode={}", config.mode().as_str())?;
    writeln!(out, "completion.enabled={}", config.enabled)?;
    writeln!(out, "completion.max_results={}", config.max_results)?;
    writeln!(out, "completion.coalesce_ms={}", config.coalesce_ms)?;
    writeln!(
        out,
        "completion.display_delay_ms={}",
        config.display_delay_ms
    )?;
    writeln!(out, "completion.ignore_spaces={}", config.ignore_spaces)?;
    writeln!(out, "completion.template_first={}", config.template_first)?;
    writeln!(out, "completion.inline={}", config.inline)?;
    writeln!(out, "completion.fuzzy={}", config.fuzzy)?;
    writeln!(out, "completion.tab_accept={}", config.tab_accept.as_str())?;
    writeln!(
        out,
        "completion.match_threshold_percent={}",
        config.match_threshold_percent
    )?;
    writeln!(
        out,
        "completion.typo_threshold_percent={}",
        config.typo_threshold_percent
    )?;
    Ok(())
}

fn write_ai_runtime_status(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "ai.model={}", config_value(&state.ai_config.model))?;
    writeln!(
        out,
        "ai.final_url={}",
        config_value(&state.ai_config.base_url)
    )?;
    writeln!(out, "ai.key_source={}", ai_key_source(state))?;
    Ok(())
}

fn ai_key_source(state: &AppState) -> &'static str {
    if read_api_key_from_env(&state.ai_config.env_key).is_ok() {
        "env"
    } else if state
        .secret_key_path
        .as_ref()
        .is_some_and(|path| path.exists())
    {
        "gpg"
    } else {
        "unconfigured"
    }
}

fn backend_shell_value(state: &AppState) -> &str {
    state.backend_shell.as_deref().unwrap_or("unknown")
}

fn gpg_status(state: &AppState) -> &'static str {
    if configured_encryption_key(&state.encryption_config).is_empty() {
        return "not_configured";
    }
    match Command::new(gpg_program()).arg("--version").output() {
        Ok(output) if output.status.success() => "available",
        _ => "unavailable",
    }
}

fn config_value(value: &str) -> &str {
    if value.is_empty() {
        "unconfigured"
    } else {
        value
    }
}

fn write_editor_resolution(out: &mut impl Write, state: &AppState) -> Result<()> {
    match resolve_editor_command(&state.editor_config) {
        Some(command) => {
            writeln!(
                out,
                "editor.resolved={}",
                format_editor_command(&command.argv)
            )?;
        }
        None => {
            writeln!(out, "editor.resolved=unavailable")?;
        }
    }
    Ok(())
}

fn format_editor_command(command: &[String]) -> String {
    if command.is_empty() {
        "unconfigured".to_string()
    } else {
        command.join(" ")
    }
}

fn write_config_path(out: &mut impl Write, name: &str, path: &Option<PathBuf>) -> Result<()> {
    match path {
        Some(path) => writeln!(out, "{name}={}", path.display())?,
        None => writeln!(out, "{name}=unconfigured")?,
    }
    Ok(())
}

fn write_path_status(out: &mut impl Write, name: &str, path: &Option<PathBuf>) -> Result<()> {
    match path {
        Some(path) => writeln!(out, "{name}={} exists={}", path.display(), path.exists())?,
        None => writeln!(out, "{name}=unconfigured")?,
    }
    Ok(())
}
