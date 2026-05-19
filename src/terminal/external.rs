use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

use crate::app::{AppState, execute_draft};
use crate::editor::resolve_editor_command;
use crate::picker::{
    PickerAction, PickerRunResult, env_var_picker_candidates, file_picker_candidates,
    git_branch_picker_candidates, run_fzf_picker, shell_env_var_reference,
};
use crate::pty::PtyBackend;

use super::render::invalidate_render_anchor;

pub fn run_external_editor(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<()> {
    let Some(command) = resolve_editor_command(&state.editor_config) else {
        invalidate_render_anchor(state);
        writeln!(out, "editor.resolved=unavailable")?;
        return Ok(());
    };
    let Some(temp_root) = state.editor_temp_root.clone() else {
        invalidate_render_anchor(state);
        writeln!(out, "editor temp directory is not configured")?;
        return Ok(());
    };

    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }

    let is_ai_prompt_editor = state.should_open_ai_prompt_editor();
    let result = if is_ai_prompt_editor {
        state.run_ai_prompt_editor_roundtrip(&temp_root, &command)
    } else {
        state.run_editor_roundtrip(&temp_root, &command)
    };

    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }

    let result = result?;
    if result.exit_code == Some(0) {
        invalidate_render_anchor(state);
        if state.draft_from_editor {
            writeln!(out, "editor saved draft")?;
        } else {
            writeln!(out, "editor empty; canceled")?;
        }
        if state.editor_config.execute_after_save
            && state.draft_from_editor
            && !state.draft_from_ai_editor
        {
            execute_draft(state, backend, out, command_timeout)?;
        }
    } else {
        invalidate_render_anchor(state);
        writeln!(
            out,
            "editor exited without saving draft: status={}",
            result
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        )?;
    }
    Ok(())
}

pub fn run_file_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let root = state
        .current_cwd
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let candidates = file_picker_candidates(&root)?;
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "file picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_file_picker_result(state, result, out)
}

pub fn run_history_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state.history_picker_candidates();
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "history search has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_history_picker_result(state, result, out)
}

pub fn run_template_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state.template_picker_candidates()?;
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "template picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_template_picker_result(state, result, out)
}

pub fn run_git_branch_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let root = state
        .current_cwd
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let candidates = git_branch_picker_candidates(&root)?;
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "git branch picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_git_branch_picker_result(state, result, out)
}

pub fn run_env_var_picker(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
) -> Result<()> {
    let candidates = env_var_picker_candidates_from_backend(backend);
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "environment variable picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_env_var_picker_result(state, result, out)
}

fn env_var_picker_candidates_from_backend(backend: &mut PtyBackend) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(result) = backend.run_command(" env", Duration::from_secs(2)) {
        names.extend(
            result
                .output
                .lines()
                .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_string())),
        );
    }
    names.extend(env_var_picker_candidates());
    crate::picker::env_var_picker_candidates_from_names(names)
}

pub(super) fn apply_env_var_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "environment variable picker cancelled")?;
        return Ok(());
    };
    let Some(reference) = shell_env_var_reference(&selected) else {
        invalidate_render_anchor(state);
        writeln!(
            out,
            "environment variable picker rejected invalid name: {selected}"
        )?;
        return Ok(());
    };

    if !state.apply_raw_picker_selection(&reference, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
        writeln!(out, "environment variable picker could not update draft")?;
    }
    Ok(())
}

pub(super) fn apply_git_branch_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "git branch picker cancelled")?;
        return Ok(());
    };

    if !state.apply_picker_selection(&selected, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
        writeln!(out, "git branch picker could not update draft")?;
    }
    Ok(())
}

pub(super) fn apply_template_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "template picker cancelled")?;
        return Ok(());
    };

    let id = selected
        .split_whitespace()
        .next()
        .unwrap_or(selected.as_str());
    if state.replace_draft_from_template_picker(&selected)? {
        invalidate_render_anchor(state);
        writeln!(out, "template copied to draft: {id}")?;
    } else {
        invalidate_render_anchor(state);
        writeln!(out, "template not found: {id}")?;
    }
    Ok(())
}

pub(super) fn apply_history_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "history search cancelled")?;
        return Ok(());
    };

    state.replace_draft_from_history_picker(selected);
    Ok(())
}

pub(super) fn apply_file_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "file picker cancelled")?;
        return Ok(());
    };

    if !state.apply_picker_selection(&selected, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
        writeln!(out, "file picker could not update draft")?;
    }
    Ok(())
}

fn run_external_picker(run: impl FnOnce() -> Result<PickerRunResult>) -> Result<PickerRunResult> {
    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }

    let result = run();

    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }

    result
}
