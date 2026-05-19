use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::AppState;
use crate::config::CompletionMode;
use crate::keybindings::{KeyBindingAction, KeyBindingMatch, match_keybinding};
use crate::shell_integration::passthrough_key_bytes;
use crate::templates::template_placeholder_spans;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Exit,
    ClearScreen,
    HistorySearch,
    ExternalEditor,
    FilePicker,
    TemplatePicker,
    GitBranchPicker,
    EnvVarPicker,
    AdvancedKeyPlaceholder(&'static str),
    Submit,
    ConfirmContext(bool),
    ConfirmPrivateOutput(bool),
    CompleteOrShow,
    AcceptCompletion,
    PreviousDraft,
    NextDraft,
    ForwardToBackend(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteAction {
    Continue,
    Submit,
}

pub fn apply_paste_to_state(text: &str, state: &mut AppState) -> PasteAction {
    let text = normalize_paste_newlines(text);
    if !text.contains('\n') {
        state.copy_read_only_selection_to_draft();
        if state.draft.is_empty() {
            state.draft_from_editor = false;
            state.draft_from_ai_editor = false;
            state.draft_from_template = false;
            state.draft_has_paste_preview = false;
        }
        state.draft.insert_str(&text);
        return PasteAction::Continue;
    }

    match state.paste_config.multiline.as_str() {
        "editor" | "execute" if state.paste_config.confirm_execute => {
            state.replace_draft_from_paste_text(text);
            PasteAction::Continue
        }
        "execute" => {
            state.replace_draft_from_paste_text(text);
            PasteAction::Submit
        }
        _ => PasteAction::Continue,
    }
}

pub(crate) fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end_matches('\n')
        .to_string()
}

pub fn apply_key_to_state(key: KeyEvent, state: &mut AppState) -> KeyAction {
    if matches!(
        state.mode,
        crate::modes::Mode::Passthrough | crate::modes::Mode::UnlockPassthrough
    ) {
        return passthrough_key_bytes(key)
            .map(KeyAction::ForwardToBackend)
            .unwrap_or(KeyAction::Continue);
    }

    if state.pending_context.is_some() {
        return match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => KeyAction::ConfirmContext(true),
            (_, KeyCode::Char('y' | 'Y')) => KeyAction::ConfirmContext(true),
            (_, KeyCode::Char('n' | 'N')) => KeyAction::ConfirmContext(false),
            (_, KeyCode::Esc) => KeyAction::ConfirmContext(false),
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => KeyAction::ConfirmContext(false),
            _ => KeyAction::Continue,
        };
    }

    if state.pending_private_output.is_some() {
        return match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => KeyAction::ConfirmPrivateOutput(true),
            (_, KeyCode::Char('y' | 'Y')) => KeyAction::ConfirmPrivateOutput(true),
            (_, KeyCode::Char('n' | 'N')) => KeyAction::ConfirmPrivateOutput(false),
            (_, KeyCode::Esc) => KeyAction::ConfirmPrivateOutput(false),
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => KeyAction::ConfirmPrivateOutput(false),
            _ => KeyAction::Continue,
        };
    }

    let binding_match = match_keybinding(
        &state.keybinding_config,
        state.pending_key_prefix.as_ref(),
        key,
    );
    let action = match binding_match {
        KeyBindingMatch::Action(action) => {
            state.clear_pending_key_prefix();
            Some(action)
        }
        KeyBindingMatch::Prefix(prefix) => {
            state.clear_completion_ui();
            state.set_pending_key_prefix(prefix);
            return KeyAction::Continue;
        }
        KeyBindingMatch::UnmatchedPending => {
            state.clear_pending_key_prefix();
            state.clear_completion_ui();
            return KeyAction::Continue;
        }
        KeyBindingMatch::None => {
            state.clear_pending_key_prefix();
            None
        }
    };
    if let Some(action) = action {
        if !matches!(
            action,
            KeyBindingAction::CompleteOrCycle | KeyBindingAction::MoveRightOrAcceptCompletion
        ) {
            state.clear_completion_ui();
        }
        return apply_bound_key_action(action, state);
    }

    state.clear_completion_ui();
    let is_editor_draft = state.mode == crate::modes::Mode::Draft && state.draft_from_editor;
    match key.code {
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !is_editor_draft =>
        {
            state.copy_read_only_selection_to_draft();
            if state.draft.is_empty() {
                state.draft_from_editor = false;
                state.draft_from_ai_editor = false;
                state.draft_from_template = false;
                state.draft_has_paste_preview = false;
            }
            expand_template_draft_if_inside_placeholder(state);
            state.draft.insert_char(ch);
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn apply_bound_key_action(action: KeyBindingAction, state: &mut AppState) -> KeyAction {
    let is_read_only_mode = matches!(
        state.mode,
        crate::modes::Mode::History | crate::modes::Mode::Ai
    );
    let is_editor_draft = state.mode == crate::modes::Mode::Draft && state.draft_from_editor;
    match action {
        KeyBindingAction::ClearOrCancel => {
            state.clear_draft_for_new_draft();
            KeyAction::Continue
        }
        KeyBindingAction::ExitOrDelete if state.draft.is_empty() => KeyAction::Exit,
        KeyBindingAction::ExitOrDelete if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::ExitOrDelete => {
            if !delete_template_placeholder_after_cursor(state) {
                state.draft.delete();
            }
            if state.draft.is_empty() {
                state.selected_draft_index = None;
            }
            KeyAction::Continue
        }
        KeyBindingAction::ClearScreen => KeyAction::ClearScreen,
        KeyBindingAction::HistorySearch => KeyAction::HistorySearch,
        KeyBindingAction::ExternalEditor => KeyAction::ExternalEditor,
        KeyBindingAction::FilePicker => KeyAction::FilePicker,
        KeyBindingAction::TemplatePicker => KeyAction::TemplatePicker,
        KeyBindingAction::GitBranchPicker => KeyAction::GitBranchPicker,
        KeyBindingAction::EnvVarPicker => KeyAction::EnvVarPicker,
        KeyBindingAction::MoveStart => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_start();
            }
            KeyAction::Continue
        }
        KeyBindingAction::MoveEnd => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_end();
            }
            KeyAction::Continue
        }
        KeyBindingAction::DeleteToStart if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeleteToStart => {
            state.copy_read_only_selection_to_draft();
            state.draft.delete_to_start();
            KeyAction::Continue
        }
        KeyBindingAction::DeleteToEnd if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeleteToEnd => {
            state.copy_read_only_selection_to_draft();
            state.draft.delete_to_end();
            KeyAction::Continue
        }
        KeyBindingAction::DeletePreviousWord if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeletePreviousWord => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_before_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete_previous_word();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        KeyBindingAction::DeleteNextWord if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeleteNextWord => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_after_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete_next_word();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        KeyBindingAction::MovePreviousWord => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_previous_word();
            }
            KeyAction::Continue
        }
        KeyBindingAction::MoveNextWord => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_next_word();
            }
            KeyAction::Continue
        }
        KeyBindingAction::PreviousItem if state.mode == crate::modes::Mode::History => {
            state.move_history_selection_older();
            KeyAction::Continue
        }
        KeyBindingAction::PreviousItem if state.mode == crate::modes::Mode::Ai => {
            state.move_ai_selection_previous();
            KeyAction::Continue
        }
        KeyBindingAction::PreviousItem if state.mode == crate::modes::Mode::Draft => {
            KeyAction::PreviousDraft
        }
        KeyBindingAction::PreviousItem => KeyAction::Continue,
        KeyBindingAction::NextItem if state.mode == crate::modes::Mode::History => {
            state.move_history_selection_newer();
            KeyAction::Continue
        }
        KeyBindingAction::NextItem if state.mode == crate::modes::Mode::Ai => {
            state.move_ai_selection_next();
            KeyAction::Continue
        }
        KeyBindingAction::NextItem
            if state.mode == crate::modes::Mode::Draft && !is_editor_draft =>
        {
            KeyAction::NextDraft
        }
        KeyBindingAction::NextItem => KeyAction::Continue,
        KeyBindingAction::MoveLeft => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_left();
            }
            KeyAction::Continue
        }
        KeyBindingAction::MoveRightOrAcceptCompletion => {
            if !is_read_only_mode && !is_editor_draft {
                if state.mode == crate::modes::Mode::Draft
                    && state.draft.cursor() == state.draft.as_str().len()
                    && !state.draft.is_empty()
                    && state.completion_config.mode() != CompletionMode::Off
                    && (state.completion_inline.is_some()
                        || state
                            .cached_live_completion_candidates_with_max_results(1)
                            .is_some_and(|candidates| !candidates.is_empty()))
                {
                    return KeyAction::AcceptCompletion;
                }
                state.clear_completion_ui();
                state.draft.move_right();
            }
            KeyAction::Continue
        }
        KeyBindingAction::DeletePreviousChar if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeletePreviousChar => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_before_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.backspace();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        KeyBindingAction::DeleteNextChar if is_editor_draft => KeyAction::Continue,
        KeyBindingAction::DeleteNextChar => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_after_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        KeyBindingAction::Cancel => {
            state.clear_draft_for_new_draft();
            KeyAction::Continue
        }
        KeyBindingAction::CompleteOrCycle => {
            if !state.draft.is_empty() && state.mode == crate::modes::Mode::Draft {
                if state.completion_config.mode() != CompletionMode::Off {
                    return KeyAction::CompleteOrShow;
                }
                state.clear_completion_ui();
                return KeyAction::Continue;
            }
            state.clear_completion_ui();
            state.handle_empty_tab();
            KeyAction::Continue
        }
        KeyBindingAction::Submit => KeyAction::Submit,
    }
}

fn clear_draft_metadata_if_empty(state: &mut AppState) {
    if state.draft.is_empty() {
        state.selected_draft_index = None;
        state.draft_from_editor = false;
        state.draft_from_ai_editor = false;
        state.draft_from_template = false;
        state.draft_has_paste_preview = false;
    }
}

fn delete_template_placeholder_before_cursor(state: &mut AppState) -> bool {
    if !state.draft_from_template {
        return false;
    }
    let cursor = state.draft.cursor();
    for span in template_placeholder_spans(state.draft.as_str()) {
        if span.end == cursor {
            return state.draft.drain_range(span.start, span.end);
        }
    }
    false
}

fn delete_template_placeholder_after_cursor(state: &mut AppState) -> bool {
    if !state.draft_from_template {
        return false;
    }
    let cursor = state.draft.cursor();
    for span in template_placeholder_spans(state.draft.as_str()) {
        if span.start == cursor {
            return state.draft.drain_range(span.start, span.end);
        }
    }
    false
}

fn expand_template_draft_if_inside_placeholder(state: &mut AppState) {
    if !state.draft_from_template {
        return;
    }
    let cursor = state.draft.cursor();
    if template_placeholder_spans(state.draft.as_str())
        .into_iter()
        .any(|span| span.start < cursor && cursor < span.end)
    {
        state.draft_from_template = false;
    }
}
