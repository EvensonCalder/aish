use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};

use crate::ai::request_ai_items;
use crate::completion::CompletionCandidate;
use crate::completion_worker::CompletionWorker;
use crate::config::{
    AiConfig, CompletionConfig, ContextConfig, EditorConfig, EncryptionConfig, PasteConfig,
    SyncConfig,
};
use crate::editor::{
    EditorCommand, EditorRunResult, PreparedEditorSession, prepare_editor_file, read_editor_file,
    run_editor_command,
};
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{append_encrypted_jsonl, gpg_program, rewrite_encrypted_jsonl};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, JsonlLineError,
    JsonlLoad, NoteEntry, ai_command_indices, append_jsonl,
};
use crate::input::InputBuffer;
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event};
use crate::modes::Mode;
use crate::picker::{
    PickerAction, ai_history_picker_candidates, apply_picker_result, apply_raw_picker_result,
    combined_history_picker_candidates, regular_history_picker_candidates,
    template_picker_candidates,
};
use crate::templates::{
    TemplateEntry, TemplateRemoval, append_template, find_template_by_id, load_templates,
    remove_templates_by_id, replace_template_by_id,
};

use super::{
    PromptTemplates, ai_editor_initial_text, configured_encryption_key,
    draft_is_ai_prompt_or_empty_editor_trigger, normalize_editor_draft_content, unix_timestamp,
};

pub(crate) const OUTPUT_RING_CAPACITY: usize = 100;

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
    pub(super) const CONTINUATION_PREFIX: &str = ".. ";

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

    pub(crate) fn load_templates(&self) -> Result<JsonlLoad<TemplateEntry>> {
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

    pub(crate) fn append_template(&mut self, entry: &TemplateEntry) -> Result<()> {
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

    pub(crate) fn find_template_by_id(&self, id: &str) -> Result<JsonlLoad<TemplateEntry>> {
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

    pub(crate) fn remove_templates_by_id(&mut self, id: &str) -> Result<Option<TemplateRemoval>> {
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

    pub(crate) fn replace_template_by_id(
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

    pub(crate) fn append_note(&self, entry: NoteEntry) -> Result<()> {
        let Some(path) = &self.notes_path else {
            return Ok(());
        };
        self.append_jsonl_item(path, &entry)
    }

    pub(crate) fn append_draft_entry(&self, entry: &DraftEntry) -> Result<()> {
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

    pub(crate) fn append_regular_history_entry(&self, entry: &HistoryEntry) -> Result<()> {
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

    fn regular_history_newest(&self, index: usize) -> Option<&HistoryEntry> {
        self.regular_history
            .len()
            .checked_sub(index + 1)
            .and_then(|regular_index| self.regular_history.get(regular_index))
    }

    pub(crate) fn push_output_entry(&mut self, entry: OutputEntry) {
        if self.output_ring.len() == OUTPUT_RING_CAPACITY {
            self.output_ring.pop_front();
        }
        self.output_ring.push_back(entry);
    }

    pub(crate) fn append_event(&self, level: EventLevel, msg: &str) -> Result<()> {
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
