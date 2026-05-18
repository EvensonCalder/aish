use anyhow::Result;

use crate::history::{AiItem, AiItemKind, AiSession, HistoryEntry};
use crate::input::InputBuffer;
use crate::keybindings::KeyPress;
use crate::modes::Mode;

use super::AppState;

impl AppState {
    pub fn handle_empty_tab(&mut self) {
        if self.draft.is_empty() {
            self.mode = self.mode.next_primary();
            if self.mode == Mode::History {
                self.select_newest_history_if_available();
            } else if self.mode == Mode::Ai {
                self.select_ai_if_needed();
            } else if self.mode == Mode::Draft {
                self.clear_draft_for_new_draft();
            }
        }
    }

    pub fn select_newest_history_if_available(&mut self) {
        self.selected_history_index = (!self.regular_history.is_empty()).then_some(0);
    }

    pub fn selected_history_command(&self) -> Option<&str> {
        self.selected_history_index
            .and_then(|index| self.regular_history_newest(index))
            .map(|entry| entry.command.as_str())
    }

    pub fn move_history_selection_older(&mut self) -> bool {
        let Some(index) = self.selected_history_index else {
            self.select_newest_history_if_available();
            return self.selected_history_index.is_some();
        };
        if index + 1 >= self.regular_history.len() {
            return false;
        }
        self.selected_history_index = Some(index + 1);
        true
    }

    pub fn move_history_selection_newer(&mut self) -> bool {
        let Some(index) = self.selected_history_index else {
            self.select_newest_history_if_available();
            return self.selected_history_index.is_some();
        };
        if index == 0 {
            return false;
        }
        self.selected_history_index = Some(index - 1);
        true
    }

    pub fn copy_selected_history_to_draft(&mut self) -> bool {
        let Some(command) = self.selected_history_command().map(str::to_string) else {
            return false;
        };
        self.draft = InputBuffer::from(command);
        self.selected_draft_index = None;
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = false;
        self.mode = Mode::Draft;
        true
    }

    pub fn select_first_ai_if_available(&mut self) {
        self.selected_ai_index = (!self.ai_command_indices.is_empty()).then_some(0);
    }

    pub fn select_ai_if_needed(&mut self) {
        let selected_is_valid = self
            .selected_ai_index
            .is_some_and(|index| index < self.ai_command_indices.len());
        if !selected_is_valid {
            self.select_first_ai_if_available();
        }
    }

    pub fn selected_ai_command(&self) -> Option<&str> {
        self.selected_ai_item().map(|(_, item)| item.text.as_str())
    }

    pub fn move_ai_selection_previous(&mut self) -> bool {
        let Some(index) = self.selected_ai_index else {
            self.select_first_ai_if_available();
            return self.selected_ai_index.is_some();
        };
        if index == 0 {
            return false;
        }
        self.selected_ai_index = Some(index - 1);
        true
    }

    pub fn move_ai_selection_next(&mut self) -> bool {
        let Some(index) = self.selected_ai_index else {
            self.select_first_ai_if_available();
            return self.selected_ai_index.is_some();
        };
        if index + 1 >= self.ai_command_indices.len() {
            return false;
        }
        self.selected_ai_index = Some(index + 1);
        true
    }

    pub fn copy_selected_ai_to_draft(&mut self) -> bool {
        let Some(command) = self.selected_ai_command().map(str::to_string) else {
            return false;
        };
        self.draft = InputBuffer::from(command);
        self.selected_draft_index = None;
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = false;
        self.mode = Mode::Draft;
        true
    }

    pub fn copy_read_only_selection_to_draft(&mut self) -> bool {
        match self.mode {
            Mode::History => self.copy_selected_history_to_draft(),
            Mode::Ai => self.copy_selected_ai_to_draft(),
            _ => false,
        }
    }

    pub fn clear_draft_for_new_draft(&mut self) {
        self.clear_draft_preserving_mode();
        self.mode = Mode::Draft;
    }

    pub fn clear_draft_preserving_mode(&mut self) {
        self.draft.clear();
        self.continuation_prompt = None;
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = false;
        self.selected_draft_index = None;
        self.clear_completion_ui();
    }

    pub(crate) fn run_unlock_passthrough<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous_mode = self.mode;
        self.mode = Mode::UnlockPassthrough;
        self.ctrl_x_prefix = false;
        self.pending_key_prefix = None;
        self.cancel_live_completion();
        let result = operation(self);
        self.mode = previous_mode;
        result
    }

    pub(crate) fn has_pending_key_prefix(&self) -> bool {
        self.pending_key_prefix.is_some()
    }

    pub(crate) fn set_pending_key_prefix(&mut self, prefix: KeyPress) {
        self.ctrl_x_prefix = prefix.is_ctrl_x();
        self.pending_key_prefix = Some(prefix);
    }

    pub(crate) fn clear_pending_key_prefix(&mut self) {
        self.ctrl_x_prefix = false;
        self.pending_key_prefix = None;
    }

    pub(crate) fn advance_after_ai_success(&mut self) {
        let Some(current_index) = self.selected_ai_index else {
            self.mode = Mode::Draft;
            return;
        };
        let Some(current_command) = self.ai_command_indices.get(current_index) else {
            self.mode = Mode::Draft;
            return;
        };
        let next_index = current_index + 1;
        let Some(next_command) = self.ai_command_indices.get(next_index) else {
            self.selected_ai_index = None;
            self.mode = Mode::Draft;
            return;
        };
        if next_command.session_index == current_command.session_index {
            self.selected_ai_index = Some(next_index);
            self.mode = Mode::Ai;
        } else {
            self.selected_ai_index = None;
            self.mode = Mode::Draft;
        }
    }

    fn selected_ai_item(&self) -> Option<(&AiSession, &AiItem)> {
        let index = self.ai_command_indices.get(self.selected_ai_index?)?;
        let session = self.ai_sessions.get(index.session_index)?;
        let item = session.items.get(index.item_index)?;
        (item.kind == AiItemKind::Command).then_some((session, item))
    }

    fn regular_history_newest(&self, index: usize) -> Option<&HistoryEntry> {
        self.regular_history
            .len()
            .checked_sub(index + 1)
            .and_then(|regular_index| self.regular_history.get(regular_index))
    }
}
