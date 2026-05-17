use std::io::Write;

use anyhow::Result;

use crate::app::{AppState, InlineCompletion};
use crate::completion::{
    CompletionCandidate, accept_completion_with_mode, current_token_context,
    ghost_completion_suffix, limit_candidates, render_completion_candidates_for_width,
    truncate_with_ellipsis,
};
use crate::config::CompletionMode;

use super::KeyAction;

pub fn write_completion_candidates(state: &AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state
        .cached_live_completion_candidates_with_max_results(state.completion_config.max_results)
        .map(Ok)
        .unwrap_or_else(|| state.completion_panel_candidates())?;
    if candidates.is_empty() {
        return Ok(());
    }
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let width = super::terminal_display_width();
    let content_start_col = super::completion_panel_content_start_col(state, width);
    for line in render_completion_candidates_for_width(
        &candidates,
        state.draft.as_str(),
        &token,
        content_start_col,
        width,
    ) {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

pub fn complete_or_show_candidates(state: &mut AppState) -> Result<()> {
    complete_or_show_candidates_for_width(state, super::terminal_display_width())
}

pub fn complete_or_show_candidates_for_width(state: &mut AppState, width: usize) -> Result<()> {
    let had_display_delay = state.clear_completion_display_delay();
    match state.completion_config.mode() {
        CompletionMode::Auto => {
            complete_or_show_auto_candidates_for_width(state, width, had_display_delay)
        }
        CompletionMode::Tab => complete_or_show_tab_candidates_for_width(state, width),
        CompletionMode::Off => {
            state.clear_completion_ui();
            Ok(())
        }
    }
}

fn complete_or_show_auto_candidates_for_width(
    state: &mut AppState,
    width: usize,
    had_display_delay: bool,
) -> Result<()> {
    if accept_inline_completion(state)? {
        refresh_live_completion_ui_for_width(state, width)?;
        return Ok(());
    }
    let had_panel_without_inline =
        state.completion_inline.is_none() && !state.completion_panel.is_empty();
    let had_no_visible_completion =
        state.completion_inline.is_none() && state.completion_panel.is_empty();
    let candidates = state.live_completion_candidates_with_max_results(usize::MAX)?;
    if candidates.is_empty() {
        state.clear_completion_ui();
        return Ok(());
    }
    if had_display_delay && had_no_visible_completion {
        let Some(candidate) = candidates.into_iter().next() else {
            return Ok(());
        };
        accept_completion_candidate(state, candidate)?;
        refresh_live_completion_ui_for_width(state, width)?;
        return Ok(());
    }
    if had_panel_without_inline {
        let Some(candidate) = candidates.into_iter().next() else {
            return Ok(());
        };
        accept_completion_candidate(state, candidate)?;
        refresh_live_completion_ui_for_width(state, width)?;
        return Ok(());
    }
    set_completion_ui_from_candidates(state, candidates, width);
    Ok(())
}

fn complete_or_show_tab_candidates_for_width(state: &mut AppState, width: usize) -> Result<()> {
    if accept_visible_completion(state)? {
        refresh_live_completion_ui_for_width(state, width)?;
        return Ok(());
    }
    state.clear_completion_ui();
    let candidates = state.start_live_completion_request(usize::MAX)?;
    if !candidates.is_empty() {
        set_completion_ui_from_candidates(state, candidates, width);
    }
    Ok(())
}

pub(super) fn refresh_live_completion_ui(state: &mut AppState) -> Result<()> {
    refresh_live_completion_ui_for_width(state, super::terminal_display_width())
}

pub(super) fn refresh_live_completion_ui_for_width(
    state: &mut AppState,
    width: usize,
) -> Result<()> {
    state.clear_completion_ui();
    if !should_refresh_live_completion(state) {
        return Ok(());
    }
    let candidates = state.start_live_completion_request(usize::MAX)?;
    if !candidates.is_empty() {
        set_completion_ui_from_candidates(state, candidates, width);
    }
    Ok(())
}

fn should_refresh_live_completion(state: &AppState) -> bool {
    state.completion_config.enabled
        && state.completion_config.mode() == CompletionMode::Auto
        && state.pending_context.is_none()
        && !state.ctrl_x_prefix
        && state.mode == crate::modes::Mode::Draft
        && !state.draft_from_editor
        && !state.draft.is_empty()
        && state.draft.cursor() == state.draft.as_str().len()
}

pub(super) fn live_completion_input_key(state: &AppState) -> Option<(String, usize)> {
    should_refresh_live_completion(state)
        .then(|| (state.draft.as_str().to_string(), state.draft.cursor()))
}

pub(super) fn refresh_should_defer_completion_display(
    state: &AppState,
    previous: &Option<(String, usize)>,
    action: &KeyAction,
) -> bool {
    if !matches!(action, KeyAction::Continue) {
        return false;
    }
    let current = live_completion_input_key(state);
    if &current == previous {
        return false;
    }
    let Some((line, _cursor)) = current else {
        return false;
    };
    !line.trim().is_empty() && !line.starts_with('#')
}

fn accept_visible_completion(state: &mut AppState) -> Result<bool> {
    if state.completion_inline.is_none() && state.completion_panel.is_empty() {
        return Ok(false);
    }
    if accept_inline_completion(state)? {
        return Ok(true);
    }
    let candidates = state
        .cached_live_completion_candidates_with_max_results(usize::MAX)
        .map(Ok)
        .unwrap_or_else(|| state.completion_candidates())?;
    let Some(candidate) = candidates.into_iter().next() else {
        state.clear_completion_ui();
        return Ok(false);
    };
    accept_completion_candidate(state, candidate)
}

pub(super) fn set_completion_ui_from_candidates(
    state: &mut AppState,
    candidates: Vec<CompletionCandidate>,
    width: usize,
) {
    set_completion_ui_from_candidates_with_panel(state, candidates, width, true);
}

fn set_completion_ui_from_candidates_with_panel(
    state: &mut AppState,
    candidates: Vec<CompletionCandidate>,
    width: usize,
    show_panel: bool,
) {
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let inline_index = if candidates
        .first()
        .is_some_and(completion_candidate_replaces_whole_line)
    {
        None
    } else {
        candidates
            .iter()
            .position(|candidate| inline_completion_from_candidate(&token, candidate).is_some())
    };
    state.completion_inline = inline_index.and_then(|index| {
        candidates
            .get(index)
            .and_then(|candidate| inline_completion_from_candidate(&token, candidate))
    });
    if !show_panel {
        state.completion_panel.clear();
        return;
    }
    let panel_candidates: Vec<_> = if let Some(inline_index) = inline_index {
        candidates
            .into_iter()
            .enumerate()
            .filter_map(|(index, candidate)| (index != inline_index).then_some(candidate))
            .collect()
    } else {
        candidates
    };
    let panel_candidates = panel_candidates
        .into_iter()
        .filter(|candidate| candidate.replacement != token.text)
        .collect();
    let panel_candidates = limit_candidates(panel_candidates, state.completion_config.max_results);
    let content_start_col = super::completion_panel_content_start_col(state, width);
    state.completion_panel = render_completion_candidates_for_width(
        &panel_candidates,
        state.draft.as_str(),
        &token,
        content_start_col,
        width,
    );
}

pub(super) fn replace_completion_ui_from_candidates(
    state: &mut AppState,
    candidates: Vec<CompletionCandidate>,
    width: usize,
    show_panel: bool,
) -> bool {
    let previous_inline = state.completion_inline.clone();
    let previous_panel = state.completion_panel.clone();
    state.clear_completion_ui();
    if !candidates.is_empty() {
        set_completion_ui_from_candidates_with_panel(state, candidates, width, show_panel);
    }
    state.completion_inline != previous_inline || state.completion_panel != previous_panel
}

fn inline_completion_from_candidate(
    token: &crate::completion::TokenContext,
    candidate: &CompletionCandidate,
) -> Option<InlineCompletion> {
    ghost_completion_suffix(token, candidate).map(|suffix| InlineCompletion {
        candidate: candidate.clone(),
        suffix,
    })
}

fn completion_candidate_replaces_whole_line(candidate: &CompletionCandidate) -> bool {
    matches!(
        candidate.source,
        crate::completion::CompletionSource::TemplateTypo
            | crate::completion::CompletionSource::HistoryTypo
    )
}

fn accept_inline_completion(state: &mut AppState) -> Result<bool> {
    if !state.completion_config.enabled {
        state.clear_completion_ui();
        return Ok(false);
    }
    let Some(inline) = state.completion_inline.clone() else {
        return Ok(false);
    };
    state.clear_completion_ui();
    accept_completion_candidate(state, inline.candidate)
}

pub(super) fn render_inline_completion_suffix(state: &AppState, width: usize) -> Option<String> {
    if state.mode != crate::modes::Mode::Draft
        || state.draft_from_editor
        || state.draft.cursor() != state.draft.as_str().len()
    {
        return None;
    }
    let suffix = &state.completion_inline.as_ref()?.suffix;
    let (_, cursor_col) = super::terminal_cursor_position_for_width(state, width);
    let remaining = width.saturating_sub(cursor_col as usize);
    let suffix = truncate_with_ellipsis(suffix, remaining);
    (!suffix.is_empty()).then_some(suffix)
}

pub(super) fn write_inline_completion_suffix(out: &mut impl Write, suffix: &str) -> Result<()> {
    if std::env::var_os("NO_COLOR").is_some() {
        write!(out, "{suffix}")?;
    } else {
        write!(out, "\x1b[2m{suffix}\x1b[0m")?;
    }
    Ok(())
}

pub fn accept_first_completion(state: &mut AppState) -> Result<bool> {
    if accept_inline_completion(state)? {
        return Ok(true);
    }
    let candidates = state
        .cached_live_completion_candidates_with_max_results(usize::MAX)
        .map(Ok)
        .unwrap_or_else(|| state.completion_candidates())?;
    let Some(candidate) = candidates.into_iter().next() else {
        return Ok(false);
    };
    accept_completion_candidate(state, candidate)
}

fn accept_completion_candidate(
    state: &mut AppState,
    candidate: crate::completion::CompletionCandidate,
) -> Result<bool> {
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let accepted = accept_completion_with_mode(
        state.draft.as_str(),
        &token,
        &candidate,
        state.completion_config.tab_accept,
    );
    if state.draft.replace(accepted.line, accepted.cursor) {
        state.draft_from_template = matches!(
            candidate.source,
            crate::completion::CompletionSource::Template
                | crate::completion::CompletionSource::TemplateTypo
                | crate::completion::CompletionSource::TemplatePlaceholder
        );
        state.clear_completion_ui();
        Ok(true)
    } else {
        Ok(false)
    }
}
