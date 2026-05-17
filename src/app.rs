use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

use crate::ai::request_ai_items;
use crate::commands::{ParsedLine, parse_line};
use crate::completion::CompletionCandidate;
use crate::completion_worker::CompletionWorker;
#[cfg(test)]
use crate::config::PromptConfig;
use crate::config::{
    self, AiConfig, CompletionConfig, ContextConfig, EditorConfig, EncryptionConfig, PasteConfig,
    SyncConfig,
};
use crate::context::{
    build_contextual_ai_prompt, is_dangerous_context_command, run_context_command,
};
use crate::editor::{
    EditorCommand, EditorRunResult, PreparedEditorSession, prepare_editor_file, read_editor_file,
    run_editor_command,
};
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{
    append_encrypted_jsonl, gpg_program, load_encrypted_jsonl, load_encrypted_jsonl_with_bytes,
    rewrite_encrypted_jsonl,
};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, HistorySource,
    HistoryStore, JsonlLineError, JsonlLoad, NoteEntry, ai_command_indices, append_jsonl,
    load_jsonl, newest_first_indices, trim_combined_history,
};
use crate::input::InputBuffer;
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event, format_recent_events, load_events};
use crate::modes::Mode;
use crate::picker::{
    PickerAction, ai_history_picker_candidates, apply_picker_result, apply_raw_picker_result,
    combined_history_picker_candidates, regular_history_picker_candidates,
    template_picker_candidates,
};
use crate::pty::{PtyBackend, PtyCommandEvent};
use crate::shell_integration::{is_interactive_passthrough_command, passthrough_key_bytes};
#[cfg(test)]
use crate::templates::template_id;
use crate::templates::{
    TemplateEntry, TemplateRemoval, append_template, find_template_by_id, load_templates,
    remove_templates_by_id, replace_template_by_id, template_placeholders,
};

mod completion_runtime;
mod config_commands;
mod encryption_commands;
mod help;
mod private_commands;
mod prompt;
mod prompt_command;
mod reports;
mod sync_commands;

use config_commands::{update_ai_config_field, update_completion_config, update_context_config};
#[cfg(test)]
use encryption_commands::{StoredApiKey, write_history_rewrite_script};
use encryption_commands::{
    ai_config_for_request, clear_stored_key, configured_encryption_key, parse_key_command,
    set_stored_key, update_encryption_config,
};
pub use prompt::PromptTemplates;
use reports::{write_config_report, write_doctor_report, write_editor_report, write_status_report};
#[cfg(test)]
use sync_commands::write_last_sync_attempt;
use sync_commands::{
    run_manual_sync_push, run_startup_sync_check, set_sync_remote, set_sync_schedule,
};

const OUTPUT_RING_CAPACITY: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputEntry {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingContextPrompt {
    pub prompt: String,
    pub command: String,
    pub dangerous: bool,
}

#[derive(Debug)]
pub struct AppState {
    pub mode: Mode,
    pub draft: InputBuffer,
    pub last_status: Option<i32>,
    pub current_cwd: Option<PathBuf>,
    pub backend_shell: Option<String>,
    pub exit_requested: bool,
    pub regular_history_path: Option<PathBuf>,
    pub ai_history_path: Option<PathBuf>,
    pub notes_path: Option<PathBuf>,
    pub draft_history_path: Option<PathBuf>,
    pub events_path: Option<PathBuf>,
    pub template_store_path: Option<PathBuf>,
    pub secret_key_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub draft_persist: bool,
    pub draft_history: Vec<DraftEntry>,
    pub selected_draft_index: Option<usize>,
    pub regular_history: Vec<HistoryEntry>,
    pub selected_history_index: Option<usize>,
    pub ai_sessions: Vec<AiSession>,
    pub ai_command_indices: Vec<AiCommandIndex>,
    pub selected_ai_index: Option<usize>,
    pub templates: Vec<TemplateEntry>,
    pub template_errors: Vec<JsonlLineError>,
    pub output_ring: VecDeque<OutputEntry>,
    pub prompt_templates: PromptTemplates,
    pub editor_config: EditorConfig,
    pub editor_temp_root: Option<PathBuf>,
    pub paste_config: PasteConfig,
    pub completion_config: CompletionConfig,
    pub ai_config: AiConfig,
    pub ai_requester: fn(&AiConfig, &str) -> Result<Vec<AiItem>>,
    pub context_config: ContextConfig,
    pub encryption_config: EncryptionConfig,
    pub encrypted_writer: Option<EncryptedWriteQueue>,
    pub last_encrypted_write_error: Option<String>,
    pub sync_config: SyncConfig,
    pub pending_context: Option<PendingContextPrompt>,
    pub completion_panel: Vec<String>,
    pub completion_inline: Option<InlineCompletion>,
    pub completion_worker: Option<CompletionWorker>,
    pub completion_generation: u64,
    pub pending_completion: Option<PendingCompletion>,
    pub pending_completion_update: Option<PendingCompletionUpdate>,
    pub completion_history_snapshot: Arc<Vec<HistoryEntry>>,
    pub completion_history_snapshot_len: usize,
    pub completion_template_snapshot: Arc<Vec<TemplateEntry>>,
    pub completion_template_snapshot_len: usize,
    pub completion_display_not_before: Option<Instant>,
    pub last_rendered_lines: usize,
    pub last_rendered_cursor_row: usize,
    pub render_anchor_saved: bool,
    pub continuation_prompt: Option<String>,
    pub draft_from_editor: bool,
    pub draft_from_ai_editor: bool,
    pub draft_from_template: bool,
    pub ctrl_x_prefix: bool,
    pub clock: fn() -> i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineCompletion {
    pub candidate: CompletionCandidate,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCompletion {
    pub id: u64,
    pub line: String,
    pub cursor: usize,
    pub candidates: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone)]
pub struct PendingCompletionUpdate {
    pub id: u64,
    pub line: String,
    pub cursor: usize,
    pub candidates: Vec<CompletionCandidate>,
    pub first_seen: Instant,
    pub final_tier_seen: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Draft,
            draft: InputBuffer::new(),
            last_status: None,
            current_cwd: None,
            backend_shell: None,
            exit_requested: false,
            regular_history_path: None,
            ai_history_path: None,
            notes_path: None,
            draft_history_path: None,
            events_path: None,
            template_store_path: None,
            secret_key_path: None,
            config_path: None,
            draft_persist: true,
            draft_history: Vec::new(),
            selected_draft_index: None,
            regular_history: Vec::new(),
            selected_history_index: None,
            ai_sessions: Vec::new(),
            ai_command_indices: Vec::new(),
            selected_ai_index: None,
            templates: Vec::new(),
            template_errors: Vec::new(),
            output_ring: VecDeque::new(),
            prompt_templates: PromptTemplates::default(),
            editor_config: EditorConfig::default(),
            editor_temp_root: None,
            paste_config: PasteConfig::default(),
            completion_config: CompletionConfig::default(),
            ai_config: AiConfig::default(),
            ai_requester: request_ai_items,
            context_config: ContextConfig::default(),
            encryption_config: EncryptionConfig::default(),
            encrypted_writer: None,
            last_encrypted_write_error: None,
            sync_config: SyncConfig::default(),
            pending_context: None,
            completion_panel: Vec::new(),
            completion_inline: None,
            completion_worker: None,
            completion_generation: 0,
            pending_completion: None,
            pending_completion_update: None,
            completion_history_snapshot: Arc::new(Vec::new()),
            completion_history_snapshot_len: 0,
            completion_template_snapshot: Arc::new(Vec::new()),
            completion_template_snapshot_len: usize::MAX,
            completion_display_not_before: None,
            last_rendered_lines: 1,
            last_rendered_cursor_row: 0,
            render_anchor_saved: false,
            continuation_prompt: None,
            draft_from_editor: false,
            draft_from_ai_editor: false,
            draft_from_template: false,
            ctrl_x_prefix: false,
            clock: unix_timestamp,
        }
    }
}

impl AppState {
    const CONTINUATION_PREFIX: &str = ".. ";

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
        self.selected_draft_index = None;
        self.clear_completion_ui();
    }

    pub fn save_current_draft_if_needed(&mut self) -> Result<bool> {
        if !self.draft_persist || self.draft.is_empty() {
            return Ok(false);
        }
        let text = self.draft.as_str().to_string();
        if self
            .selected_draft_index
            .and_then(|index| self.draft_history.get(index))
            .is_some_and(|entry| entry.text == text)
        {
            return Ok(false);
        }

        let entry = DraftEntry {
            t: (self.clock)(),
            text,
        };
        self.append_draft_entry(&entry)?;
        self.draft_history.push(entry);
        self.selected_draft_index = self.draft_history.len().checked_sub(1);
        Ok(true)
    }

    pub fn move_draft_selection_older(&mut self) -> Result<bool> {
        if !self.draft_persist || self.draft_from_editor {
            return Ok(false);
        }
        self.save_current_draft_if_needed()?;
        let Some(target) = (match self.selected_draft_index {
            Some(index) if index > 0 => Some(index - 1),
            Some(index) => Some(index),
            None => self.draft_history.len().checked_sub(1),
        }) else {
            return Ok(false);
        };
        self.copy_saved_draft_to_current(target)
    }

    pub fn move_draft_selection_newer(&mut self) -> Result<bool> {
        if !self.draft_persist || self.draft_from_editor {
            return Ok(false);
        }
        self.save_current_draft_if_needed()?;
        let Some(index) = self.selected_draft_index else {
            return Ok(false);
        };
        if index + 1 < self.draft_history.len() {
            return self.copy_saved_draft_to_current(index + 1);
        }
        self.clear_draft_for_new_draft();
        Ok(true)
    }

    fn copy_saved_draft_to_current(&mut self, index: usize) -> Result<bool> {
        let Some(entry) = self.draft_history.get(index) else {
            return Ok(false);
        };
        self.draft = InputBuffer::from(entry.text.clone());
        self.selected_draft_index = Some(index);
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
        self.clear_completion_ui();
        Ok(true)
    }

    pub fn prepare_editor_session(
        &mut self,
        temp_root: &std::path::Path,
    ) -> Result<PreparedEditorSession> {
        self.copy_read_only_selection_to_draft();
        self.mode = Mode::Draft;
        prepare_editor_file(temp_root, self.draft.as_str())
    }

    pub fn should_open_ai_prompt_editor(&self) -> bool {
        self.draft_from_ai_editor || draft_is_ai_prompt_or_empty_editor_trigger(self.draft.as_str())
    }

    pub fn prepare_ai_prompt_editor_session(
        &mut self,
        temp_root: &std::path::Path,
    ) -> Result<PreparedEditorSession> {
        self.mode = Mode::Draft;
        let initial_text = if self.draft_from_ai_editor {
            self.draft.as_str().to_string()
        } else {
            ai_editor_initial_text(self.draft.as_str()).unwrap_or_default()
        };
        prepare_editor_file(temp_root, &initial_text)
    }

    pub fn replace_draft_from_editor_session(
        &mut self,
        session: &PreparedEditorSession,
    ) -> Result<()> {
        let content = normalize_editor_draft_content(&read_editor_file(session)?);
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
        Ok(())
    }

    pub fn replace_draft_from_ai_prompt_editor_session(
        &mut self,
        session: &PreparedEditorSession,
    ) -> Result<()> {
        let content = normalize_editor_draft_content(&read_editor_file(session)?);
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = !self.draft.is_empty();
        self.draft_from_template = false;
        self.mode = Mode::Draft;
        Ok(())
    }

    pub fn run_editor_roundtrip(
        &mut self,
        temp_root: &std::path::Path,
        command: &EditorCommand,
    ) -> Result<EditorRunResult> {
        let session = self.prepare_editor_session(temp_root)?;
        let result = run_editor_command(command, &session)?;
        if result.exit_code == Some(0) {
            self.replace_draft_from_editor_session(&session)?;
        }
        Ok(result)
    }

    pub fn run_ai_prompt_editor_roundtrip(
        &mut self,
        temp_root: &std::path::Path,
        command: &EditorCommand,
    ) -> Result<EditorRunResult> {
        let session = self.prepare_ai_prompt_editor_session(temp_root)?;
        let result = run_editor_command(command, &session)?;
        if result.exit_code == Some(0) {
            self.replace_draft_from_ai_prompt_editor_session(&session)?;
        }
        Ok(result)
    }

    pub fn replace_draft_from_editor_text(&mut self, content: impl Into<String>) {
        let content = normalize_editor_draft_content(&content.into());
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
    }

    pub fn apply_picker_selection(&mut self, value: &str, action: PickerAction) -> bool {
        if self.mode != Mode::Draft || self.draft_from_editor {
            return false;
        }
        let edit = apply_picker_result(self.draft.as_str(), self.draft.cursor(), value, action);
        if self.draft.replace(edit.line, edit.cursor) {
            self.draft_from_template = false;
            true
        } else {
            false
        }
    }

    pub fn apply_raw_picker_selection(&mut self, value: &str, action: PickerAction) -> bool {
        if self.mode != Mode::Draft || self.draft_from_editor {
            return false;
        }
        let edit = apply_raw_picker_result(self.draft.as_str(), self.draft.cursor(), value, action);
        if self.draft.replace(edit.line, edit.cursor) {
            self.draft_from_template = false;
            true
        } else {
            false
        }
    }

    pub fn history_picker_candidates(&self) -> Vec<String> {
        match self.mode {
            Mode::History => regular_history_picker_candidates(&self.regular_history),
            Mode::Ai => ai_history_picker_candidates(&self.ai_sessions),
            _ => combined_history_picker_candidates(&self.regular_history, &self.ai_sessions),
        }
    }

    pub fn replace_draft_from_history_picker(&mut self, command: impl Into<String>) {
        self.draft = InputBuffer::from(command.into());
        self.selected_draft_index = None;
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
    }

    pub fn template_picker_candidates(&self) -> Result<Vec<String>> {
        let loaded = self.load_templates()?;
        Ok(template_picker_candidates(&loaded.items))
    }

    pub fn replace_draft_from_template_picker(&mut self, selected: &str) -> Result<bool> {
        let id = selected.split_whitespace().next().unwrap_or(selected);
        let loaded = self.find_template_by_id(id)?;
        let Some(template) = loaded.items.first() else {
            return Ok(false);
        };
        self.draft = InputBuffer::from(template.body.clone());
        self.selected_draft_index = None;
        self.draft_from_editor = false;
        self.draft_from_ai_editor = false;
        self.draft_from_template = true;
        self.mode = Mode::Draft;
        Ok(true)
    }

    fn load_templates(&self) -> Result<JsonlLoad<TemplateEntry>> {
        let Some(path) = &self.template_store_path else {
            return Ok(JsonlLoad {
                items: Vec::new(),
                errors: Vec::new(),
            });
        };
        if !self.templates.is_empty() || self.encryption_config.enabled {
            Ok(JsonlLoad {
                items: self.templates.clone(),
                errors: self.template_errors.clone(),
            })
        } else {
            load_templates(path)
        }
    }

    fn append_template(&mut self, entry: &TemplateEntry) -> Result<()> {
        let Some(path) = &self.template_store_path else {
            return Ok(());
        };
        if self.encryption_config.enabled {
            if let Some(writer) = &self.encrypted_writer {
                writer.enqueue_append_jsonl(path, entry)?;
            } else {
                append_encrypted_jsonl(
                    gpg_program(),
                    configured_encryption_key(&self.encryption_config),
                    path,
                    entry,
                )?;
            }
        } else {
            append_template(path, entry)?;
        }
        self.templates.push(entry.clone());
        self.invalidate_completion_template_snapshot();
        Ok(())
    }

    fn find_template_by_id(&self, id: &str) -> Result<JsonlLoad<TemplateEntry>> {
        let Some(path) = &self.template_store_path else {
            return Ok(JsonlLoad {
                items: Vec::new(),
                errors: Vec::new(),
            });
        };
        if !self.templates.is_empty() || self.encryption_config.enabled {
            let mut loaded = self.load_templates()?;
            loaded.items = loaded
                .items
                .into_iter()
                .rev()
                .find(|template| template.id() == id)
                .into_iter()
                .collect();
            Ok(loaded)
        } else {
            find_template_by_id(path, id)
        }
    }

    fn remove_templates_by_id(&mut self, id: &str) -> Result<Option<TemplateRemoval>> {
        let Some(path) = &self.template_store_path else {
            return Ok(None);
        };
        if !self.encryption_config.enabled {
            let removal = remove_templates_by_id(path, id)?;
            self.templates = removal.remaining.clone();
            self.template_errors = removal.errors.clone();
            return Ok(Some(removal));
        }
        let loaded = self.load_templates()?;
        let before = loaded.items.len();
        let remaining: Vec<_> = loaded
            .items
            .into_iter()
            .filter(|template| template.id() != id)
            .collect();
        let removed = before - remaining.len();
        if let Some(writer) = &self.encrypted_writer {
            writer.enqueue_rewrite_jsonl(path, &remaining)?;
        } else {
            rewrite_encrypted_jsonl(
                gpg_program(),
                configured_encryption_key(&self.encryption_config),
                path,
                &remaining,
            )?;
        }
        let removal = TemplateRemoval {
            removed,
            remaining,
            errors: loaded.errors,
        };
        self.templates = removal.remaining.clone();
        self.template_errors = removal.errors.clone();
        self.invalidate_completion_template_snapshot();
        Ok(Some(removal))
    }

    fn replace_template_by_id(
        &mut self,
        existing_id: &str,
        entry: TemplateEntry,
    ) -> Result<Option<TemplateRemoval>> {
        let Some(path) = &self.template_store_path else {
            return Ok(None);
        };
        if !self.encryption_config.enabled {
            let removal = replace_template_by_id(path, existing_id, entry)?;
            self.templates = removal.remaining.clone();
            self.template_errors = removal.errors.clone();
            return Ok(Some(removal));
        }
        let loaded = self.load_templates()?;
        let before = loaded.items.len();
        let mut remaining: Vec<_> = loaded
            .items
            .into_iter()
            .filter(|template| template.id() != existing_id)
            .collect();
        let removed = before - remaining.len();
        remaining.push(entry);
        if let Some(writer) = &self.encrypted_writer {
            writer.enqueue_rewrite_jsonl(path, &remaining)?;
        } else {
            rewrite_encrypted_jsonl(
                gpg_program(),
                configured_encryption_key(&self.encryption_config),
                path,
                &remaining,
            )?;
        }
        let removal = TemplateRemoval {
            removed,
            remaining,
            errors: loaded.errors,
        };
        self.templates = removal.remaining.clone();
        self.template_errors = removal.errors.clone();
        self.invalidate_completion_template_snapshot();
        Ok(Some(removal))
    }

    fn append_note(&self, entry: NoteEntry) -> Result<()> {
        let Some(path) = &self.notes_path else {
            return Ok(());
        };
        self.append_jsonl_item(path, &entry)
    }

    fn append_draft_entry(&self, entry: &DraftEntry) -> Result<()> {
        let Some(path) = &self.draft_history_path else {
            return Ok(());
        };
        self.append_jsonl_item(path, entry)
    }

    fn append_ai_session(&self, session: &AiSession) -> Result<()> {
        let Some(path) = &self.ai_history_path else {
            return Ok(());
        };
        self.append_jsonl_item(path, session)
    }

    fn append_regular_history_entry(&self, entry: &HistoryEntry) -> Result<()> {
        let Some(path) = &self.regular_history_path else {
            return Ok(());
        };
        self.append_jsonl_item(path, entry)
    }

    fn append_jsonl_item<T: serde::Serialize>(&self, path: &Path, item: &T) -> Result<()> {
        if self.encryption_config.enabled {
            if let Some(writer) = &self.encrypted_writer {
                writer.enqueue_append_jsonl(path, item)
            } else {
                append_encrypted_jsonl(
                    gpg_program(),
                    configured_encryption_key(&self.encryption_config),
                    path,
                    item,
                )
            }
        } else {
            append_jsonl(path, item)
        }
    }

    pub fn store_ai_session_from_items(
        &mut self,
        prompt: &str,
        model: &str,
        items: Vec<AiItem>,
    ) -> Result<bool> {
        let session = AiSession {
            id: format!("ai-{}-{}", (self.clock)(), self.ai_sessions.len() + 1),
            t: (self.clock)(),
            prompt: prompt.to_string(),
            ctx: false,
            model: model.to_string(),
            items,
        };
        self.append_ai_session(&session)?;
        let new_session_index = self.ai_sessions.len();
        self.ai_sessions.push(session);
        self.ai_command_indices = ai_command_indices(&self.ai_sessions);
        self.selected_ai_index = self
            .ai_command_indices
            .iter()
            .position(|index| index.session_index == new_session_index);
        if self.selected_ai_index.is_some() {
            self.mode = Mode::Ai;
            Ok(true)
        } else {
            self.mode = Mode::Draft;
            Ok(false)
        }
    }

    fn selected_ai_item(&self) -> Option<(&AiSession, &AiItem)> {
        let index = self.ai_command_indices.get(self.selected_ai_index?)?;
        let session = self.ai_sessions.get(index.session_index)?;
        let item = session.items.get(index.item_index)?;
        (item.kind == AiItemKind::Command).then_some((session, item))
    }

    fn advance_after_ai_success(&mut self) {
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

    fn regular_history_newest(&self, index: usize) -> Option<&HistoryEntry> {
        self.regular_history
            .len()
            .checked_sub(index + 1)
            .and_then(|regular_index| self.regular_history.get(regular_index))
    }

    fn push_output_entry(&mut self, entry: OutputEntry) {
        if self.output_ring.len() == OUTPUT_RING_CAPACITY {
            self.output_ring.pop_front();
        }
        self.output_ring.push_back(entry);
    }

    fn append_event(&self, level: EventLevel, msg: &str) -> Result<()> {
        if let Some(path) = &self.events_path {
            append_event(path, (self.clock)(), level, msg, DEFAULT_MAX_EVENTS)?;
        }
        Ok(())
    }

    pub fn start_encrypted_writer_with_cache(&mut self, initial_cache: HashMap<PathBuf, Vec<u8>>) {
        if !self.encryption_config.enabled {
            self.encrypted_writer = None;
            return;
        }
        let recipient = configured_encryption_key(&self.encryption_config).to_string();
        if recipient.is_empty() {
            self.encrypted_writer = None;
            return;
        }
        self.encrypted_writer = Some(EncryptedWriteQueue::start(
            gpg_program(),
            recipient,
            initial_cache,
        ));
        self.last_encrypted_write_error = None;
    }

    pub fn stop_encrypted_writer(&mut self) {
        self.encrypted_writer = None;
    }

    pub fn flush_encrypted_writes(&self) -> Result<()> {
        if let Some(writer) = &self.encrypted_writer {
            writer.flush().context("pending encrypted writes failed")?;
        }
        Ok(())
    }

    pub fn invalidate_encrypted_writer_cache(&self, paths: Vec<PathBuf>) -> Result<()> {
        if let Some(writer) = &self.encrypted_writer {
            writer.invalidate(paths)?;
        }
        Ok(())
    }

    pub fn drain_encrypted_write_events(&mut self) -> bool {
        let Some(writer) = &self.encrypted_writer else {
            return false;
        };
        let events = writer.drain_events();
        if events.is_empty() {
            return false;
        }
        for event in events {
            if let Some(error) = event.error {
                self.last_encrypted_write_error = Some(format!(
                    "{} failed for {}: {error}",
                    event.operation.as_str(),
                    event.path.display()
                ));
                let _ = self.append_event(EventLevel::Error, "encrypted write failed");
            }
        }
        true
    }
}

fn normalize_editor_draft_content(content: &str) -> String {
    content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end_matches('\n')
        .to_string()
}

pub fn draft_is_ai_prompt_or_empty_editor_trigger(text: &str) -> bool {
    if text
        .strip_prefix("# ")
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        return true;
    }
    matches!(
        parse_line(text),
        ParsedLine::AiPrompt(_) | ParsedLine::AiPromptWithContext { .. }
    )
}

fn ai_editor_initial_text(text: &str) -> Option<String> {
    if !draft_is_ai_prompt_or_empty_editor_trigger(text) {
        return None;
    }
    text.strip_prefix("# ")
        .map(|prompt| prompt.trim_start().to_string())
}

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::runtime_aish_dir()?)?;
    let mut encrypted_cache = HashMap::new();
    let store = load_history_store(&layout, &config.encryption, &mut encrypted_cache)?;
    let templates = load_template_store(&layout, &config.encryption, &mut encrypted_cache)?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        current_cwd: backend.initial_cwd().map(PathBuf::from),
        backend_shell: Some(backend.shell_program().to_string()),
        regular_history_path: Some(layout.regular_history),
        ai_history_path: Some(layout.ai_history),
        notes_path: Some(layout.notes),
        draft_history_path: Some(layout.draft_history),
        events_path: Some(layout.events),
        template_store_path: Some(layout.template_store),
        secret_key_path: Some(layout.secrets.join("key.json.gpg")),
        config_path: Some(layout.config),
        draft_persist: config.draft.persist,
        draft_history: store.drafts,
        regular_history: store.regular,
        ai_sessions: store.ai_sessions,
        ai_command_indices: store.ai_command_indices,
        templates: templates.items,
        template_errors: templates.errors,
        prompt_templates: config.prompt.into(),
        editor_config: config.editor,
        paste_config: config.paste,
        completion_config: config.completion,
        ai_config: config.ai,
        context_config: config.context,
        encryption_config: config.encryption,
        sync_config: config.sync,
        editor_temp_root: Some(layout.runtime_cache.join("editor")),
        ..AppState::default()
    };
    state.start_encrypted_writer_with_cache(encrypted_cache);
    run_startup_sync_check(&mut state, &layout.root, &mut io::stdout())?;
    crate::terminal::run(
        &mut state,
        &mut backend,
        &mut io::stdout(),
        Duration::from_secs(60),
    )
}

fn load_template_store(
    layout: &config::DirectoryLayout,
    encryption: &EncryptionConfig,
    encrypted_cache: &mut HashMap<PathBuf, Vec<u8>>,
) -> Result<JsonlLoad<TemplateEntry>> {
    if encryption.enabled {
        let (loaded, bytes) = load_encrypted_jsonl_with_bytes::<TemplateEntry>(
            gpg_program(),
            &layout.template_store,
        )?;
        encrypted_cache.insert(layout.template_store.clone(), bytes);
        Ok(loaded)
    } else {
        load_templates(&layout.template_store)
    }
}

fn load_history_store(
    layout: &config::DirectoryLayout,
    encryption: &EncryptionConfig,
    encrypted_cache: &mut HashMap<PathBuf, Vec<u8>>,
) -> Result<HistoryStore> {
    if !encryption.enabled {
        return HistoryStore::load(layout);
    }

    let program = gpg_program();
    let (regular, regular_bytes) =
        load_encrypted_jsonl_with_bytes::<HistoryEntry>(&program, &layout.regular_history)?;
    let (drafts, draft_bytes) =
        load_encrypted_jsonl_with_bytes::<DraftEntry>(&program, &layout.draft_history)?;
    let (ai_sessions, ai_bytes) =
        load_encrypted_jsonl_with_bytes::<AiSession>(&program, &layout.ai_history)?;
    let (notes, note_bytes) =
        load_encrypted_jsonl_with_bytes::<NoteEntry>(&program, &layout.notes)?;
    encrypted_cache.insert(layout.regular_history.clone(), regular_bytes);
    encrypted_cache.insert(layout.draft_history.clone(), draft_bytes);
    encrypted_cache.insert(layout.ai_history.clone(), ai_bytes);
    encrypted_cache.insert(layout.notes.clone(), note_bytes);
    let regular_newest_indices = newest_first_indices(regular.items.len());
    let ai_command_indices = ai_command_indices(&ai_sessions.items);

    let mut errors = Vec::new();
    errors.extend(regular.errors);
    errors.extend(drafts.errors);
    errors.extend(ai_sessions.errors);
    errors.extend(notes.errors);

    Ok(HistoryStore {
        regular: regular.items,
        regular_newest_indices,
        drafts: drafts.items,
        ai_sessions: ai_sessions.items,
        ai_command_indices,
        notes: notes.items,
        errors,
    })
}

pub fn execute_draft(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    state.cancel_live_completion();
    if state.pending_context.is_some() {
        writeln!(out, "context confirmation is pending; answer Y or n")?;
        state.mode = Mode::Draft;
        return Ok(());
    }

    if state.draft.is_empty() && state.mode == Mode::History {
        state.copy_selected_history_to_draft();
    }
    let executing_ai = state.draft.is_empty() && state.mode == Mode::Ai;
    if executing_ai {
        state.copy_selected_ai_to_draft();
    }

    if state.draft.is_empty() {
        return Ok(());
    }

    let command = state.draft.as_str().to_string();
    if state.draft_from_ai_editor {
        let prompt = command.trim();
        if prompt.is_empty() {
            state.clear_draft_for_new_draft();
            return Ok(());
        }
        let ai_line = format!("# {prompt}");
        match parse_line(&ai_line) {
            ParsedLine::AiPrompt(prompt) => submit_ai_prompt(state, prompt, out)?,
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, out, timeout)?;
            }
            _ => unreachable!("AI editor drafts are submitted as AI prompts"),
        }
        state.clear_draft_preserving_mode();
        return Ok(());
    }
    if state.draft_from_template {
        let unresolved = template_placeholders(&command);
        if !unresolved.is_empty() {
            writeln!(
                out,
                "cannot execute unresolved template placeholders: {}",
                unresolved.join(", ")
            )?;
            state.mode = Mode::Draft;
            return Ok(());
        }
    }
    if !state.draft_from_editor {
        match parse_line(&command) {
            ParsedLine::Ordinary(_) => {}
            ParsedLine::EmptyPrivate => {
                writeln!(out, "empty Aish command")?;
                state.clear_draft_for_new_draft();
                return Ok(());
            }
            ParsedLine::Note { tag, text } => {
                state.append_note(NoteEntry {
                    tag,
                    text: text.to_string(),
                })?;
                writeln!(out, "note stored")?;
                state.clear_draft_for_new_draft();
                return Ok(());
            }
            ParsedLine::Private { name, args } => {
                private_commands::execute_private_command(state, out, name, args)?;
                return Ok(());
            }
            ParsedLine::AiPrompt(prompt) => {
                submit_ai_prompt(state, prompt, out)?;
                state.clear_draft_preserving_mode();
                return Ok(());
            }
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, out, timeout)?;
                state.clear_draft_preserving_mode();
                return Ok(());
            }
        }
    }

    if !state.draft_from_editor {
        let continuation = backend.input_needs_more_lines(&command)?;
        if continuation.needs_more {
            state.continuation_prompt = continuation.prompt;
            state.draft.insert_str("\n");
            state.mode = Mode::Draft;
            return Ok(());
        }
        state.save_current_draft_if_needed()?;
    }

    state.mode = Mode::CommandRunning;
    if !state.draft_from_editor && is_interactive_passthrough_command(&command) {
        let exit_code = run_foreground_interactive_command(state, backend, &command)?;
        record_completed_command(state, command, String::new(), exit_code, executing_ai)?;
        return Ok(());
    }

    let result = backend.run_command_with_event_callback(&command, timeout, |backend, event| {
        handle_command_running_event(backend, out, event)
    })?;
    record_completed_command(
        state,
        result.command.clone(),
        result.output.clone(),
        result.exit_code,
        executing_ai,
    )?;
    if let Some(cwd) = result.cwd {
        state.current_cwd = Some(PathBuf::from(cwd));
    }
    Ok(())
}

fn record_completed_command(
    state: &mut AppState,
    command: String,
    output: String,
    exit_code: i32,
    executing_ai: bool,
) -> Result<()> {
    state.push_output_entry(OutputEntry {
        command: command.clone(),
        output: output.clone(),
        exit_code,
    });
    if state.regular_history_path.is_some() {
        let entry = HistoryEntry {
            command,
            t: (state.clock)(),
            exit_code: Some(exit_code),
            source: if executing_ai {
                HistorySource::Ai
            } else {
                HistorySource::User
            },
        };
        state.append_regular_history_entry(&entry)?;
        state.regular_history.push(entry);
    }
    state.last_status = Some(exit_code);
    state.continuation_prompt = None;
    if executing_ai {
        state.draft.clear();
        state.selected_draft_index = None;
        state.draft_from_editor = false;
        state.draft_from_ai_editor = false;
        state.draft_from_template = false;
    } else {
        state.clear_draft_for_new_draft();
    }
    if executing_ai && exit_code == 0 {
        state.advance_after_ai_success();
    } else if executing_ai {
        state.mode = Mode::Ai;
    }
    Ok(())
}

fn run_foreground_interactive_command(
    state: &AppState,
    backend: &PtyBackend,
    command: &str,
) -> Result<i32> {
    let shell = backend.shell_program();
    let args = foreground_shell_args(shell, command);
    let cwd = state
        .current_cwd
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }
    let child = Command::new(shell)
        .args(&args)
        .current_dir(cwd)
        .spawn()
        .with_context(|| format!("failed to run interactive command `{command}`"));
    let status = match child {
        Ok(mut child) => {
            let _sigint_guard = SigintIgnoreGuard::ignore();
            child
                .wait()
                .with_context(|| format!("failed to wait for interactive command `{command}`"))
        }
        Err(err) => Err(err),
    };
    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }
    Ok(status?.code().unwrap_or(1))
}

fn forward_terminal_input_to_backend(backend: &mut PtyBackend) -> Result<bool> {
    if !is_raw_mode_enabled().unwrap_or(false) {
        return Ok(false);
    }

    let mut marker_may_need_reissue = false;
    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Key(key) => {
                if matches!(
                    (key.modifiers, key.code),
                    (
                        crossterm::event::KeyModifiers::CONTROL,
                        crossterm::event::KeyCode::Char('c' | 'd')
                    )
                ) {
                    marker_may_need_reissue = true;
                }
                if let Some(bytes) = passthrough_key_bytes(key) {
                    backend.write_raw(&bytes)?;
                }
            }
            Event::Paste(text) => {
                backend.write_raw(&text)?;
            }
            Event::Resize(cols, rows) => {
                backend.resize(crate::pty::pty_size(cols, rows))?;
            }
            _ => {}
        }
    }
    Ok(marker_may_need_reissue)
}

fn handle_command_running_event(
    backend: &mut PtyBackend,
    out: &mut impl Write,
    event: PtyCommandEvent<'_>,
) -> Result<bool> {
    match event {
        PtyCommandEvent::Output(chunk) => {
            write_command_output_bytes(out, chunk)?;
            out.flush()?;
            Ok(false)
        }
        PtyCommandEvent::PollInput | PtyCommandEvent::Idle => {
            forward_terminal_input_to_backend(backend)
        }
    }
}

#[cfg(unix)]
struct SigintIgnoreGuard {
    previous: SignalHandler,
}

#[cfg(unix)]
type SignalHandler = usize;

#[cfg(unix)]
impl SigintIgnoreGuard {
    fn ignore() -> Self {
        unsafe extern "C" {
            fn signal(signum: i32, handler: SignalHandler) -> SignalHandler;
        }

        const SIGINT: i32 = 2;
        const SIG_IGN: SignalHandler = 1;

        let previous = unsafe { signal(SIGINT, SIG_IGN) };
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for SigintIgnoreGuard {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn signal(signum: i32, handler: SignalHandler) -> SignalHandler;
        }

        const SIGINT: i32 = 2;
        let _ = unsafe { signal(SIGINT, self.previous) };
    }
}

#[cfg(not(unix))]
struct SigintIgnoreGuard;

#[cfg(not(unix))]
impl SigintIgnoreGuard {
    fn ignore() -> Self {
        Self
    }
}

fn foreground_shell_args(shell: &str, command: &str) -> Vec<String> {
    let shell_name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match shell_name {
        "fish" => vec!["-c".to_string(), command.to_string()],
        _ => vec!["-lc".to_string(), command.to_string()],
    }
}

#[cfg(test)]
fn write_command_output(out: &mut impl Write, output: &str) -> Result<()> {
    write_command_output_bytes(out, output.as_bytes())
}

fn write_command_output_bytes(out: &mut impl Write, output: &[u8]) -> Result<()> {
    // PTY output is already terminal protocol. Adding display framing here can
    // corrupt commands like `clear`: ESC[H ESC[2J followed by an Aish-added LF
    // moves the prompt to row 1, leaving a blank first row.
    out.write_all(output)?;
    if output_clears_visible_screen_bytes(output) {
        write!(out, "\x1b[H")?;
    }
    Ok(())
}

fn output_clears_visible_screen_bytes(output: &[u8]) -> bool {
    output_clears_visible_screen(&String::from_utf8_lossy(output))
}

fn output_clears_visible_screen(output: &str) -> bool {
    output.contains("\x1b[2J")
        || output.contains("\x1bc")
        || (output_contains_cursor_home(output) && output.contains("\x1b[J"))
}

fn output_contains_cursor_home(output: &str) -> bool {
    output.contains("\x1b[H") || output.contains("\x1b[;H") || output.contains("\x1b[1;1H")
}

pub fn answer_context_confirmation(
    state: &mut AppState,
    accepted: bool,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    let Some(pending) = state.pending_context.take() else {
        return Ok(());
    };
    state.mode = Mode::Draft;
    if !accepted {
        writeln!(out, "context command skipped: {}", pending.command)?;
        state.append_event(EventLevel::Info, "context command skipped")?;
        return Ok(());
    }
    state.append_event(EventLevel::Info, "context command confirmed")?;
    submit_confirmed_ai_prompt_with_context(state, &pending.prompt, &pending.command, out, timeout)
}

fn submit_ai_prompt(state: &mut AppState, prompt: &str, out: &mut impl Write) -> Result<()> {
    let request_config = match ai_config_for_request(state) {
        Ok(config) => config,
        Err(error) => {
            state.append_event(EventLevel::Error, "AI request failed")?;
            writeln!(out, "AI request failed: {error}")?;
            state.mode = Mode::Draft;
            return Ok(());
        }
    };
    match (state.ai_requester)(&request_config, prompt) {
        Ok(items) => {
            let item_count = items.len();
            let model = request_config.model.clone();
            if state.store_ai_session_from_items(prompt, &model, items)? {
                state.append_event(
                    EventLevel::Info,
                    &format!("AI generated {item_count} item(s)"),
                )?;
                writeln!(out, "AI items generated: {}", item_count)?;
            } else {
                state.append_event(EventLevel::Warn, "AI response contained no command items")?;
                writeln!(out, "AI response contained no command items")?;
            }
        }
        Err(error) => {
            state.append_event(EventLevel::Error, "AI request failed")?;
            writeln!(out, "AI request failed: {error}")?;
            state.mode = Mode::Draft;
        }
    }
    Ok(())
}

fn submit_ai_prompt_with_context(
    state: &mut AppState,
    prompt: &str,
    command: &str,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    if !state.context_config.enabled {
        writeln!(
            out,
            "context collection is disabled; context command not executed: {command}"
        )?;
        state.append_event(
            EventLevel::Warn,
            "context command skipped because context is disabled",
        )?;
        state.mode = Mode::Draft;
        return Ok(());
    }
    if is_dangerous_context_command(command) {
        writeln!(
            out,
            "dangerous context command requires confirmation: {command}"
        )?;
        state.pending_context = Some(PendingContextPrompt {
            prompt: prompt.to_string(),
            command: command.to_string(),
            dangerous: true,
        });
        state.append_event(
            EventLevel::Warn,
            "dangerous context command requires confirmation",
        )?;
        state.mode = Mode::Draft;
        return Ok(());
    }
    if state.context_config.confirm {
        writeln!(out, "aish will run this command to collect context:")?;
        writeln!(out)?;
        writeln!(out, "  {command}")?;
        writeln!(out)?;
        writeln!(out, "Run context command? [Y/n]")?;
        writeln!(out, "answer Y to run context command or n to skip")?;
        state.pending_context = Some(PendingContextPrompt {
            prompt: prompt.to_string(),
            command: command.to_string(),
            dangerous: false,
        });
        state.append_event(EventLevel::Warn, "context command requires confirmation")?;
        state.mode = Mode::Draft;
        return Ok(());
    }

    submit_confirmed_ai_prompt_with_context(state, prompt, command, out, timeout)
}

fn submit_confirmed_ai_prompt_with_context(
    state: &mut AppState,
    prompt: &str,
    command: &str,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    let result = run_context_command(
        command,
        state.current_cwd.as_deref(),
        state.context_config.max_bytes,
        timeout,
    )?;
    state.append_event(EventLevel::Info, "context command captured output")?;
    if result.truncated {
        state.append_event(EventLevel::Warn, "context output truncated")?;
        writeln!(
            out,
            "context output truncated to {} bytes",
            state.context_config.max_bytes
        )?;
    }
    let contextual_prompt = build_contextual_ai_prompt(prompt, command, &result);
    submit_ai_prompt(state, &contextual_prompt, out)
}

fn show_event_log(state: &AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let count = args.parse::<usize>();
    match (count, &state.events_path) {
        (Ok(count), Some(path)) => {
            let loaded = load_events(path)?;
            for line in format_recent_events(&loaded.items, count) {
                writeln!(out, "{line}")?;
            }
            if loaded.items.is_empty() {
                writeln!(out, "no events logged")?;
            }
            if !loaded.errors.is_empty() {
                writeln!(out, "skipped {} bad event line(s)", loaded.errors.len())?;
            }
        }
        (Ok(_), None) => writeln!(out, "event log storage is not configured")?,
        (Err(_), _) => writeln!(out, "usage: #log <count>")?,
    }
    Ok(())
}

fn trim_history_for_state(
    state: &AppState,
    count: usize,
) -> Result<crate::history::TrimHistoryLoad> {
    let Some(regular_path) = &state.regular_history_path else {
        anyhow::bail!("history storage is not configured");
    };
    let Some(ai_path) = &state.ai_history_path else {
        anyhow::bail!("history storage is not configured");
    };
    if !state.encryption_config.enabled {
        return trim_combined_history(regular_path, ai_path, count);
    }

    state.flush_encrypted_writes()?;
    let regular = load_encrypted_jsonl::<HistoryEntry>(gpg_program(), regular_path)?;
    let ai_sessions = load_encrypted_jsonl::<AiSession>(gpg_program(), ai_path)?;

    let keep_from = regular.items.len().saturating_sub(count);
    let trimmed_regular = regular.items[keep_from..].to_vec();

    let mut remaining_ai_commands = count.saturating_sub(trimmed_regular.len());
    let mut trimmed_ai_sessions = Vec::new();
    for session in ai_sessions.items.iter().rev() {
        let mut kept_items = Vec::new();
        let mut kept_command = false;
        for item in session.items.iter().rev() {
            if item.kind == AiItemKind::Command {
                if remaining_ai_commands == 0 {
                    continue;
                }
                remaining_ai_commands -= 1;
                kept_command = true;
            }
            kept_items.push(item.clone());
        }
        kept_items.reverse();
        if kept_command {
            let mut trimmed_session = session.clone();
            trimmed_session.items = kept_items;
            trimmed_ai_sessions.push(trimmed_session);
        }
    }
    trimmed_ai_sessions.reverse();

    rewrite_encrypted_jsonl(
        gpg_program(),
        configured_encryption_key(&state.encryption_config),
        regular_path,
        &trimmed_regular,
    )?;
    rewrite_encrypted_jsonl(
        gpg_program(),
        configured_encryption_key(&state.encryption_config),
        ai_path,
        &trimmed_ai_sessions,
    )?;
    state.invalidate_encrypted_writer_cache(vec![regular_path.clone(), ai_path.clone()])?;

    Ok(crate::history::TrimHistoryLoad {
        regular,
        ai_sessions,
    })
}

fn load_ai_sessions_for_state(state: &AppState) -> Result<Vec<AiSession>> {
    let Some(ai_path) = &state.ai_history_path else {
        return Ok(Vec::new());
    };
    if state.encryption_config.enabled {
        Ok(load_encrypted_jsonl::<AiSession>(gpg_program(), ai_path)?.items)
    } else {
        Ok(load_jsonl::<AiSession>(ai_path)?.items)
    }
}

fn parse_template_body(args: &str) -> Option<&str> {
    let body = args.trim();
    (!body.is_empty()).then_some(body)
}

fn parse_template_find_query(args: &str) -> Option<&str> {
    let query = args.trim_start().strip_prefix("find")?.trim_start();
    (!query.is_empty()).then_some(query)
}

fn parse_template_id_and_body(args: &str) -> Option<(&str, &str)> {
    let args = args.trim();
    let split_at = args.find(char::is_whitespace)?;
    let (id, body) = args.split_at(split_at);
    let body = body.trim_start();
    (!id.is_empty() && !body.is_empty()).then_some((id, body))
}

fn parse_template_subcommand_args(args: &str) -> Option<(&str, &str)> {
    let rest = args.trim_start().strip_prefix("replace")?.trim_start();
    parse_template_id_and_body(rest)
}

fn parse_template_values(args: &str) -> HashMap<String, String> {
    let tokens = split_template_tokens(args);
    let mut parts = tokens.iter().map(String::as_str);
    let _subcommand = parts.next();
    let _name = parts.next();

    parts
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            (!key.is_empty()).then_some((key.to_string(), trim_matching_quotes(value).to_string()))
        })
        .collect()
}

fn split_template_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        match quote {
            Some(active) if ch == active => {
                quote = None;
                current.push(ch);
            }
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn trim_matching_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn template_usage() -> &'static str {
    "usage: #template find <query> | #template show <id> | #template use <id> [key=value...] | #template rm <id> | #template replace <id> <body>"
}

pub fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

pub fn save_draft_if_configured(state: &AppState) -> Result<bool> {
    if !state.draft_persist || state.draft.is_empty() {
        return Ok(false);
    }
    if state.draft_history_path.is_none() {
        return Ok(false);
    }

    state.append_draft_entry(&DraftEntry {
        t: (state.clock)(),
        text: state.draft.as_str().to_string(),
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests;
