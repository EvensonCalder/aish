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
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{
    append_encrypted_jsonl, encrypted_path, gpg_program, rewrite_encrypted_jsonl,
};
use crate::history::{
    AiCommandIndex, AiItem, AiSession, DraftEntry, HistoryEntry, JsonlLineError, JsonlLoad,
    NoteEntry, ai_command_indices, append_jsonl,
};
use crate::input::InputBuffer;
use crate::keybindings::{KeyPress, KeybindingConfig};
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

use super::startup_unlock::{
    EncryptedStartupData, EncryptedStartupPaths, EncryptedStartupUnlock, UnlockMode,
    load_encrypted_startup_data,
};
use super::{PendingPrivateOutput, PromptTemplates, configured_encryption_key, unix_timestamp};

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
    pub keybinding_config: KeybindingConfig,
    pub ai_config: AiConfig,
    pub ai_requester: fn(&AiConfig, &str) -> Result<Vec<AiItem>>,
    pub context_config: ContextConfig,
    pub encryption_config: EncryptionConfig,
    pub encrypted_writer: Option<EncryptedWriteQueue>,
    pub last_encrypted_write_error: Option<String>,
    pub encrypted_storage_unlocked: bool,
    pub encrypted_startup_unlock: Option<EncryptedStartupUnlock>,
    pub encrypted_startup_unlock_message: Option<String>,
    pub pending_locked_regular_history: Vec<HistoryEntry>,
    pub pending_locked_draft_history: Vec<DraftEntry>,
    pub pending_locked_ai_sessions: Vec<AiSession>,
    pub pending_locked_notes: Vec<NoteEntry>,
    pub pending_locked_templates: Vec<TemplateEntry>,
    pub sync_config: SyncConfig,
    pub pending_context: Option<PendingContextPrompt>,
    pub pending_private_output: Option<PendingPrivateOutput>,
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
    pub draft_has_paste_preview: bool,
    pub ctrl_x_prefix: bool,
    pub pending_key_prefix: Option<KeyPress>,
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
            keybinding_config: KeybindingConfig::default(),
            ai_config: AiConfig::default(),
            ai_requester: request_ai_items,
            context_config: ContextConfig::default(),
            encryption_config: EncryptionConfig::default(),
            encrypted_writer: None,
            last_encrypted_write_error: None,
            encrypted_storage_unlocked: true,
            encrypted_startup_unlock: None,
            encrypted_startup_unlock_message: None,
            pending_locked_regular_history: Vec::new(),
            pending_locked_draft_history: Vec::new(),
            pending_locked_ai_sessions: Vec::new(),
            pending_locked_notes: Vec::new(),
            pending_locked_templates: Vec::new(),
            sync_config: SyncConfig::default(),
            pending_context: None,
            pending_private_output: None,
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
            draft_has_paste_preview: false,
            ctrl_x_prefix: false,
            pending_key_prefix: None,
            clock: unix_timestamp,
        }
    }
}

impl AppState {
    pub(super) const CONTINUATION_PREFIX: &str = ".. ";

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
        self.draft_has_paste_preview = false;
        self.mode = Mode::Draft;
        self.clear_completion_ui();
        Ok(true)
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
        self.draft_has_paste_preview = false;
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
        self.draft_has_paste_preview = false;
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
        if self.encrypted_storage_is_locked() {
            self.append_locked_jsonl_item(path, entry)?;
            self.pending_locked_templates.push(entry.clone());
            self.templates.push(entry.clone());
            self.invalidate_completion_template_snapshot();
            return Ok(());
        }
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
        if self.encrypted_storage_is_locked() {
            anyhow::bail!("encrypted templates are still unlocking; run #unlock");
        }
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
        if self.encrypted_storage_is_locked() {
            anyhow::bail!("encrypted templates are still unlocking; run #unlock");
        }
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

    pub(crate) fn append_note(&mut self, entry: NoteEntry) -> Result<()> {
        let Some(path) = &self.notes_path else {
            return Ok(());
        };
        if self.encrypted_storage_is_locked() {
            self.append_locked_jsonl_item(path, &entry)?;
            self.pending_locked_notes.push(entry);
            return Ok(());
        }
        self.append_jsonl_item(path, &entry)
    }

    pub(crate) fn append_draft_entry(&mut self, entry: &DraftEntry) -> Result<()> {
        let Some(path) = &self.draft_history_path else {
            return Ok(());
        };
        if self.encrypted_storage_is_locked() {
            self.append_locked_jsonl_item(path, entry)?;
            self.pending_locked_draft_history.push(entry.clone());
            return Ok(());
        }
        self.append_jsonl_item(path, entry)
    }

    fn append_ai_session(&mut self, session: &AiSession) -> Result<()> {
        let Some(path) = &self.ai_history_path else {
            return Ok(());
        };
        if self.encrypted_storage_is_locked() {
            self.append_locked_jsonl_item(path, session)?;
            self.pending_locked_ai_sessions.push(session.clone());
            return Ok(());
        }
        self.append_jsonl_item(path, session)
    }

    pub(crate) fn append_regular_history_entry(&mut self, entry: &HistoryEntry) -> Result<()> {
        let Some(path) = &self.regular_history_path else {
            return Ok(());
        };
        if self.encrypted_storage_is_locked() {
            self.append_locked_jsonl_item(path, entry)?;
            self.pending_locked_regular_history.push(entry.clone());
            return Ok(());
        }
        self.append_jsonl_item(path, entry)
    }

    fn append_locked_jsonl_item<T: serde::Serialize>(&self, path: &Path, item: &T) -> Result<()> {
        if encrypted_path(path).exists() {
            return Ok(());
        }
        self.append_jsonl_item(path, item)
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

    pub fn encrypted_storage_is_locked(&self) -> bool {
        self.encryption_config.enabled && !self.encrypted_storage_unlocked
    }

    pub fn has_pending_locked_writes(&self) -> bool {
        !self.pending_locked_regular_history.is_empty()
            || !self.pending_locked_draft_history.is_empty()
            || !self.pending_locked_ai_sessions.is_empty()
            || !self.pending_locked_notes.is_empty()
            || !self.pending_locked_templates.is_empty()
    }

    pub(crate) fn drain_startup_unlock_event(&mut self) -> Result<bool> {
        let Some(unlock) = &self.encrypted_startup_unlock else {
            return Ok(false);
        };
        let Some(result) = unlock.try_recv() else {
            return Ok(false);
        };
        self.encrypted_startup_unlock = None;
        match result {
            Ok(data) => {
                self.apply_encrypted_startup_data(data)?;
                self.encrypted_startup_unlock_message = Some("encrypted storage unlocked".into());
            }
            Err(error) => {
                self.encrypted_startup_unlock_message = Some(format!(
                    "encrypted storage needs passphrase; run #unlock ({error})"
                ));
            }
        }
        Ok(true)
    }

    pub(crate) fn unlock_encrypted_storage_interactively(&mut self) -> Result<bool> {
        if !self.encryption_config.enabled || self.encrypted_storage_unlocked {
            return Ok(false);
        }
        let Some(paths) = self.encrypted_startup_paths() else {
            anyhow::bail!("encrypted storage paths are not configured");
        };
        self.encrypted_startup_unlock = None;
        let data = self.run_unlock_passthrough(|_| {
            load_encrypted_startup_data(&paths, UnlockMode::Interactive)
        })?;
        self.apply_encrypted_startup_data(data)?;
        self.encrypted_startup_unlock_message = Some("encrypted storage unlocked".into());
        Ok(true)
    }

    fn encrypted_startup_paths(&self) -> Option<EncryptedStartupPaths> {
        Some(EncryptedStartupPaths {
            regular_history: self.regular_history_path.clone()?,
            draft_history: self.draft_history_path.clone()?,
            ai_history: self.ai_history_path.clone()?,
            notes: self.notes_path.clone()?,
            template_store: self.template_store_path.clone()?,
        })
    }

    fn apply_encrypted_startup_data(&mut self, data: EncryptedStartupData) -> Result<()> {
        self.flush_encrypted_writes()?;
        let pending_regular = std::mem::take(&mut self.pending_locked_regular_history);
        let pending_drafts = std::mem::take(&mut self.pending_locked_draft_history);
        let pending_ai_sessions = std::mem::take(&mut self.pending_locked_ai_sessions);
        let pending_notes = std::mem::take(&mut self.pending_locked_notes);
        let pending_templates = std::mem::take(&mut self.pending_locked_templates);

        let mut store = data.store;
        let missing_regular = extend_missing_and_collect(&mut store.regular, pending_regular);
        let missing_drafts = extend_missing_and_collect(&mut store.drafts, pending_drafts);
        let missing_ai_sessions =
            extend_missing_and_collect(&mut store.ai_sessions, pending_ai_sessions);
        let missing_notes = missing_items(&store.notes, pending_notes);
        let mut templates = data.templates.items;
        let missing_templates = extend_missing_and_collect(&mut templates, pending_templates);

        self.regular_history = store.regular;
        self.draft_history = store.drafts;
        self.ai_sessions = store.ai_sessions;
        self.ai_command_indices = ai_command_indices(&self.ai_sessions);
        self.templates = templates;
        self.template_errors = data.templates.errors;
        self.invalidate_completion_history_snapshot();
        self.invalidate_completion_template_snapshot();
        self.encrypted_storage_unlocked = true;
        self.start_encrypted_writer_with_cache(data.encrypted_cache);
        self.persist_missing_locked_writes(
            &missing_regular,
            &missing_drafts,
            &missing_ai_sessions,
            &missing_notes,
            &missing_templates,
        )?;
        Ok(())
    }

    fn persist_missing_locked_writes(
        &self,
        regular: &[HistoryEntry],
        drafts: &[DraftEntry],
        ai_sessions: &[AiSession],
        notes: &[NoteEntry],
        templates: &[TemplateEntry],
    ) -> Result<()> {
        if let Some(path) = self.regular_history_path.clone() {
            for entry in regular {
                self.append_jsonl_item(&path, entry)?;
            }
        }
        if let Some(path) = self.draft_history_path.clone() {
            for entry in drafts {
                self.append_jsonl_item(&path, entry)?;
            }
        }
        if let Some(path) = self.ai_history_path.clone() {
            for session in ai_sessions {
                self.append_jsonl_item(&path, session)?;
            }
        }
        if let Some(path) = self.notes_path.clone() {
            for note in notes {
                self.append_jsonl_item(&path, note)?;
            }
        }
        if let Some(path) = self.template_store_path.clone() {
            for template in templates {
                self.append_jsonl_item(&path, template)?;
            }
        }
        Ok(())
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

    pub fn replace_encrypted_writer_cache(&self, entries: HashMap<PathBuf, Vec<u8>>) -> Result<()> {
        if let Some(writer) = &self.encrypted_writer {
            writer.replace_cache(entries)?;
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

fn extend_missing_and_collect<T: PartialEq + Clone>(items: &mut Vec<T>, pending: Vec<T>) -> Vec<T> {
    let mut missing = Vec::new();
    for item in pending {
        if !items.contains(&item) {
            items.push(item.clone());
            missing.push(item);
        }
    }
    missing
}

fn missing_items<T: PartialEq + Clone>(items: &[T], pending: Vec<T>) -> Vec<T> {
    pending
        .into_iter()
        .filter(|item| !items.contains(item))
        .collect()
}
