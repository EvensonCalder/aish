use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

use crate::ai::{normalize_chat_completions_url, read_api_key_from_env, request_ai_items};
use crate::commands::{ParsedLine, parse_line};
use crate::completion::{
    CompletionCandidate, CompletionOptions, CompletionSource, complete_first_token_with_options,
    complete_non_first_token_for_line_with_options, complete_private_command_line,
    current_token_context, dedupe_completion_candidates, rank_completion_candidates,
};
use crate::completion_worker::{CompletionJob, CompletionTier, CompletionWorker};
#[cfg(test)]
use crate::config::PromptConfig;
use crate::config::{
    self, AiConfig, CompletionConfig, CompletionMode, CompletionTabAccept, ContextConfig,
    EditorConfig, EncryptionConfig, PasteConfig, SyncConfig,
};
use crate::context::{
    build_contextual_ai_prompt, is_dangerous_context_command, run_context_command,
};
use crate::editor::{
    EditorCommand, EditorRunResult, PreparedEditorSession, prepare_editor_file, read_editor_file,
    resolve_editor_command, run_editor_command,
};
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{
    append_encrypted_jsonl, atomic_gpg_encrypt_bytes, encrypted_path,
    encryption_git_history_warning, existing_jsonl_bytes, gpg_decrypt_file, gpg_program,
    load_encrypted_jsonl, load_encrypted_jsonl_with_bytes, migrate_gpg_jsonl_to_plaintext,
    migrate_plaintext_jsonl_to_gpg, pause_terminal_raw_mode_for_gpg, prepare_gpg_terminal_env,
    reencrypt_gpg_jsonl, resolve_gpg_key_fingerprint, rewrite_encrypted_jsonl,
};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, HistorySource,
    HistoryStore, JsonlLineError, JsonlLoad, NoteEntry, ai_command_indices, append_jsonl,
    load_jsonl, newest_first_indices, trim_combined_history,
};
use crate::input::InputBuffer;
use crate::keybindings::default_keybindings;
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event, format_recent_events, load_events};
use crate::modes::Mode;
use crate::picker::{
    PickerAction, ai_history_picker_candidates, apply_picker_result, apply_raw_picker_result,
    combined_history_picker_candidates, regular_history_picker_candidates,
    template_picker_candidates,
};
use crate::pty::{PtyBackend, PtyCommandEvent};
use crate::shell_integration::{is_interactive_passthrough_command, passthrough_key_bytes};
use crate::sync::{
    GitCommandPlan, StartupSyncDecision, SyncFailureKind, SyncLock, SyncStepOutcome,
    classify_git_sync_step, conservative_sync_plan_for_existing_paths_with_encryption,
    init_repo_plan, log_sync_failure, maintain_managed_gitignore, startup_sync_decision,
    tracked_managed_files_warning,
};
#[cfg(test)]
use crate::templates::template_id;
use crate::templates::{
    TemplateEntry, TemplateRemoval, append_template, find_template_by_id, load_templates,
    remove_templates_by_id, replace_template_by_id, template_placeholders,
};

mod private_commands;
mod prompt;
mod prompt_command;

pub use prompt::PromptTemplates;
use prompt_command::write_prompt_config;

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

    pub fn completion_candidates(&self) -> Result<Vec<CompletionCandidate>> {
        self.completion_candidates_with_max_results(usize::MAX)
    }

    pub fn completion_panel_candidates(&self) -> Result<Vec<CompletionCandidate>> {
        self.completion_candidates_with_max_results(self.completion_config.max_results)
    }

    pub fn completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled || self.mode != Mode::Draft || self.draft_from_editor {
            return Ok(Vec::new());
        }
        let line = self.draft.as_str();
        let token = current_token_context(line, self.draft.cursor());
        if line.starts_with('#') {
            return Ok(complete_private_command_line(
                line,
                self.draft.cursor(),
                max_results,
            ));
        }
        let templates = self.templates_for_completion()?;
        let history_newest_first: Vec<_> = self.regular_history.iter().rev().cloned().collect();
        let options = CompletionOptions {
            max_results,
            ignore_spaces: self.completion_config.ignore_spaces,
            fuzzy_enabled: self.completion_config.fuzzy,
            match_threshold_percent: self.completion_config.match_threshold_percent,
            typo_threshold_percent: self.completion_config.typo_threshold_percent,
        };

        if token.is_first_token && !token.path_like {
            Ok(complete_first_token_with_options(
                &token.text,
                &templates,
                &history_newest_first,
                &path_dirs(),
                options,
            ))
        } else {
            Ok(complete_non_first_token_for_line_with_options(
                self.draft.as_str(),
                self.draft.cursor(),
                &completion_cwd(&self.current_cwd),
                &history_newest_first,
                &templates,
                options,
            ))
        }
    }

    pub fn start_live_completion_request(
        &mut self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        let now = Instant::now();
        let line = self.draft.as_str().to_string();
        let cursor = self.draft.cursor();
        let candidates = self.immediate_completion_candidates_with_max_results(max_results)?;
        self.pending_completion = None;
        self.pending_completion_update = None;
        let should_enqueue_async = self.should_enqueue_async_completion(&line, cursor);
        let display_deferred = !self.completion_display_ready(now) && !candidates.is_empty();
        let defer_initial_ui = should_enqueue_async
            && self.should_defer_initial_completion_ui(&line, cursor, &candidates);
        let hide_initial_ui = display_deferred || defer_initial_ui;
        if should_enqueue_async || display_deferred {
            self.completion_generation = self.completion_generation.wrapping_add(1).max(1);
            let id = self.completion_generation;
            self.pending_completion = Some(PendingCompletion {
                id,
                line: line.clone(),
                cursor,
                candidates: candidates.clone(),
            });
            if hide_initial_ui && !candidates.is_empty() {
                self.queue_completion_update(
                    id,
                    line.clone(),
                    cursor,
                    candidates.clone(),
                    !should_enqueue_async,
                    now,
                );
            }
            if should_enqueue_async {
                let history_newest_first = self.completion_history_snapshot();
                let templates = self.completion_template_snapshot()?;
                let job = CompletionJob {
                    id,
                    line,
                    cursor,
                    cwd: completion_cwd(&self.current_cwd),
                    path_dirs: Arc::new(path_dirs()),
                    history_newest_first,
                    templates,
                    options: self.completion_options(usize::MAX),
                };
                self.ensure_completion_worker().enqueue(job)?;
            }
        }
        Ok(if hide_initial_ui {
            Vec::new()
        } else {
            candidates
        })
    }

    pub fn drain_live_completion_events(&mut self) -> Option<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled {
            self.pending_completion = None;
            self.pending_completion_update = None;
            return None;
        }
        let events = self
            .completion_worker
            .as_ref()
            .map(|worker| worker.drain_events())
            .unwrap_or_default();
        let now = Instant::now();
        let line = self.draft.as_str().to_string();
        let cursor = self.draft.cursor();
        let fuzzy_enabled = self.completion_config.fuzzy;
        let Some(pending) = self.pending_completion.as_mut() else {
            self.pending_completion_update = None;
            return None;
        };
        if pending.line != line || pending.cursor != cursor {
            self.pending_completion = None;
            self.pending_completion_update = None;
            return None;
        }
        let mut changed = false;
        let mut final_tier_seen = false;
        for event in events {
            if event.id != pending.id {
                continue;
            }
            final_tier_seen |= completion_tier_is_final(event.tier, fuzzy_enabled);
            let previous_candidates = pending.candidates.clone();
            pending.candidates.extend(event.candidates);
            dedupe_completion_candidates(&mut pending.candidates);
            rank_completion_candidates(&mut pending.candidates);
            changed |= pending.candidates != previous_candidates;
        }
        let pending_id = pending.id;
        let pending_line = pending.line.clone();
        let pending_cursor = pending.cursor;
        let pending_candidates = pending.candidates.clone();
        if changed {
            self.queue_completion_update(
                pending_id,
                pending_line,
                pending_cursor,
                pending_candidates,
                final_tier_seen,
                now,
            );
        } else if final_tier_seen
            && let Some(update) = self.pending_completion_update.as_mut()
            && update.id == pending_id
            && update.line == pending_line
            && update.cursor == pending_cursor
        {
            update.final_tier_seen = true;
        }
        self.ready_completion_update(now)
    }

    fn queue_completion_update(
        &mut self,
        id: u64,
        line: String,
        cursor: usize,
        candidates: Vec<CompletionCandidate>,
        final_tier_seen: bool,
        now: Instant,
    ) {
        match self.pending_completion_update.as_mut() {
            Some(update) if update.id == id && update.line == line && update.cursor == cursor => {
                update.candidates = candidates;
                update.final_tier_seen |= final_tier_seen;
            }
            _ => {
                self.pending_completion_update = Some(PendingCompletionUpdate {
                    id,
                    line,
                    cursor,
                    candidates,
                    first_seen: now,
                    final_tier_seen,
                });
            }
        }
    }

    fn ready_completion_update(&mut self, now: Instant) -> Option<Vec<CompletionCandidate>> {
        let (update_id, update_line, update_cursor, first_seen, final_tier_seen) = {
            let update = self.pending_completion_update.as_ref()?;
            (
                update.id,
                update.line.clone(),
                update.cursor,
                update.first_seen,
                update.final_tier_seen,
            )
        };
        let current_line = self.draft.as_str();
        let current_cursor = self.draft.cursor();
        let pending_matches = self.pending_completion.as_ref().is_some_and(|pending| {
            pending.id == update_id
                && pending.line == update_line
                && pending.cursor == update_cursor
                && update_line == current_line
                && update_cursor == current_cursor
        });
        if !pending_matches {
            self.pending_completion_update = None;
            return None;
        }
        let coalesce_ms = self.completion_config.coalesce_ms;
        let display_ready = self.completion_display_ready(now);
        let ready = display_ready
            && (coalesce_ms == 0
                || final_tier_seen
                || now.saturating_duration_since(first_seen) >= Duration::from_millis(coalesce_ms));
        ready.then(|| {
            self.completion_display_not_before = None;
            self.pending_completion_update.take().unwrap().candidates
        })
    }

    pub fn cached_live_completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Option<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled {
            return None;
        }
        let pending = self.pending_completion.as_ref()?;
        if pending.line != self.draft.as_str() || pending.cursor != self.draft.cursor() {
            return None;
        }
        Some(crate::completion::limit_candidates(
            pending.candidates.clone(),
            max_results,
        ))
    }

    pub fn live_completion_candidates_with_max_results(
        &mut self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if let Some(candidates) =
            self.cached_live_completion_candidates_with_max_results(max_results)
        {
            return Ok(candidates);
        }
        let candidates = self.start_live_completion_request(usize::MAX)?;
        Ok(crate::completion::limit_candidates(candidates, max_results))
    }

    pub fn immediate_completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled || self.mode != Mode::Draft || self.draft_from_editor {
            return Ok(Vec::new());
        }
        let line = self.draft.as_str();
        let cursor = self.draft.cursor();
        let token = current_token_context(line, cursor);
        if line.starts_with('#') {
            return Ok(complete_private_command_line(line, cursor, max_results));
        }

        let options = self.completion_options(max_results);
        if token.is_first_token && !token.path_like {
            return Ok(Vec::new());
        }
        Ok(complete_non_first_token_for_line_with_options(
            line,
            cursor,
            &completion_cwd(&self.current_cwd),
            &[],
            &[],
            options,
        ))
    }

    fn completion_options(&self, max_results: usize) -> CompletionOptions {
        CompletionOptions {
            max_results,
            ignore_spaces: self.completion_config.ignore_spaces,
            fuzzy_enabled: self.completion_config.fuzzy,
            match_threshold_percent: self.completion_config.match_threshold_percent,
            typo_threshold_percent: self.completion_config.typo_threshold_percent,
        }
    }

    fn ensure_completion_worker(&mut self) -> &CompletionWorker {
        if self.completion_worker.is_none() {
            self.completion_worker = Some(CompletionWorker::start());
        }
        self.completion_worker.as_ref().unwrap()
    }

    fn should_enqueue_async_completion(&self, line: &str, cursor: usize) -> bool {
        if !self.completion_config.enabled
            || line.trim().is_empty()
            || line.starts_with('#')
            || cursor != line.len()
        {
            return false;
        }
        !current_token_context(line, cursor).path_like
    }

    fn should_defer_initial_completion_ui(
        &self,
        line: &str,
        cursor: usize,
        candidates: &[CompletionCandidate],
    ) -> bool {
        if self.completion_config.coalesce_ms == 0 || candidates.is_empty() {
            return false;
        }
        let token = current_token_context(line, cursor);
        token.is_first_token
            && !token.path_like
            && candidates
                .iter()
                .all(|candidate| candidate.source == CompletionSource::Executable)
    }

    fn completion_history_snapshot(&mut self) -> Arc<Vec<HistoryEntry>> {
        if self.completion_history_snapshot_len != self.regular_history.len() {
            self.completion_history_snapshot =
                Arc::new(self.regular_history.iter().rev().cloned().collect());
            self.completion_history_snapshot_len = self.regular_history.len();
        }
        Arc::clone(&self.completion_history_snapshot)
    }

    fn invalidate_completion_history_snapshot(&mut self) {
        self.completion_history_snapshot_len = usize::MAX;
    }

    fn completion_template_snapshot(&mut self) -> Result<Arc<Vec<TemplateEntry>>> {
        let memory_backed = !self.templates.is_empty() || self.encryption_config.enabled;
        if self.completion_template_snapshot_len == usize::MAX
            || (memory_backed && self.completion_template_snapshot_len != self.templates.len())
        {
            let templates = self.templates_for_completion()?;
            self.completion_template_snapshot_len = if memory_backed {
                self.templates.len()
            } else {
                templates.len()
            };
            self.completion_template_snapshot = Arc::new(templates);
        }
        Ok(Arc::clone(&self.completion_template_snapshot))
    }

    fn invalidate_completion_template_snapshot(&mut self) {
        self.completion_template_snapshot_len = usize::MAX;
    }

    pub(crate) fn defer_completion_display(&mut self, now: Instant) {
        if self.completion_config.display_delay_ms == 0 {
            self.completion_display_not_before = None;
            return;
        }
        self.completion_display_not_before =
            Some(now + Duration::from_millis(self.completion_config.display_delay_ms));
    }

    pub(crate) fn clear_completion_display_delay(&mut self) -> bool {
        self.completion_display_not_before.take().is_some()
    }

    fn completion_display_ready(&self, now: Instant) -> bool {
        self.completion_display_not_before
            .is_none_or(|deadline| now >= deadline)
    }

    pub fn clear_completion_ui(&mut self) {
        self.completion_panel.clear();
        self.completion_inline = None;
    }

    pub(crate) fn cancel_live_completion(&mut self) {
        self.clear_completion_ui();
        self.pending_completion = None;
        self.pending_completion_update = None;
        self.completion_display_not_before = None;
    }

    fn templates_for_completion(&self) -> Result<Vec<TemplateEntry>> {
        if !self.templates.is_empty() || self.encryption_config.enabled {
            return Ok(self.templates.clone());
        }
        let Some(path) = &self.template_store_path else {
            return Ok(Vec::new());
        };
        Ok(load_templates(path)?.items)
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

fn write_doctor_report(state: &AppState, out: &mut impl Write) -> Result<()> {
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

fn write_status_report(state: &AppState, out: &mut impl Write) -> Result<()> {
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
    writeln!(
        out,
        "completion.mode={}",
        state.completion_config.mode().as_str()
    )?;
    writeln!(
        out,
        "completion.enabled={}",
        state.completion_config.enabled
    )?;
    writeln!(
        out,
        "completion.max_results={}",
        state.completion_config.max_results
    )?;
    writeln!(
        out,
        "completion.coalesce_ms={}",
        state.completion_config.coalesce_ms
    )?;
    writeln!(
        out,
        "completion.display_delay_ms={}",
        state.completion_config.display_delay_ms
    )?;
    writeln!(
        out,
        "completion.ignore_spaces={}",
        state.completion_config.ignore_spaces
    )?;
    writeln!(
        out,
        "completion.template_first={}",
        state.completion_config.template_first
    )?;
    writeln!(out, "completion.inline={}", state.completion_config.inline)?;
    writeln!(out, "completion.fuzzy={}", state.completion_config.fuzzy)?;
    writeln!(
        out,
        "completion.tab_accept={}",
        state.completion_config.tab_accept.as_str()
    )?;
    writeln!(
        out,
        "completion.match_threshold_percent={}",
        state.completion_config.match_threshold_percent
    )?;
    writeln!(
        out,
        "completion.typo_threshold_percent={}",
        state.completion_config.typo_threshold_percent
    )?;
    writeln!(out, "keybindings={}", default_keybindings().len())?;
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

fn write_encryption_sync_status(state: &AppState, out: &mut impl Write) -> Result<()> {
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

fn gpg_status(state: &AppState) -> &'static str {
    if configured_encryption_key(&state.encryption_config).is_empty() {
        return "not_configured";
    }
    match Command::new(gpg_program()).arg("--version").output() {
        Ok(output) if output.status.success() => "available",
        _ => "unavailable",
    }
}

fn configured_encryption_key(config: &EncryptionConfig) -> &str {
    let fingerprint = config.key_fingerprint.trim();
    if !fingerprint.is_empty() {
        fingerprint
    } else {
        config.recipient.trim()
    }
}

fn write_config_report(state: &AppState, out: &mut impl Write) -> Result<()> {
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
    write_prompt_config(out, &state.prompt_templates)?;
    writeln!(
        out,
        "completion.mode={}",
        state.completion_config.mode().as_str()
    )?;
    writeln!(
        out,
        "completion.enabled={}",
        state.completion_config.enabled
    )?;
    writeln!(
        out,
        "completion.max_results={}",
        state.completion_config.max_results
    )?;
    writeln!(
        out,
        "completion.coalesce_ms={}",
        state.completion_config.coalesce_ms
    )?;
    writeln!(
        out,
        "completion.display_delay_ms={}",
        state.completion_config.display_delay_ms
    )?;
    writeln!(
        out,
        "completion.ignore_spaces={}",
        state.completion_config.ignore_spaces
    )?;
    writeln!(
        out,
        "completion.template_first={}",
        state.completion_config.template_first
    )?;
    writeln!(out, "completion.inline={}", state.completion_config.inline)?;
    writeln!(out, "completion.fuzzy={}", state.completion_config.fuzzy)?;
    writeln!(
        out,
        "completion.tab_accept={}",
        state.completion_config.tab_accept.as_str()
    )?;
    writeln!(
        out,
        "completion.match_threshold_percent={}",
        state.completion_config.match_threshold_percent
    )?;
    writeln!(
        out,
        "completion.typo_threshold_percent={}",
        state.completion_config.typo_threshold_percent
    )?;
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

fn config_value(value: &str) -> &str {
    if value.is_empty() {
        "unconfigured"
    } else {
        value
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredApiKey {
    env_key: String,
    value: String,
}

fn parse_key_command(args: &str) -> Option<&str> {
    let mut parts = args.split_whitespace();
    let command = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(command)
}

fn write_editor_report(state: &AppState, out: &mut impl Write) -> Result<()> {
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

fn write_editor_resolution(out: &mut impl Write, state: &AppState) -> Result<()> {
    match resolve_editor_command(&state.editor_config) {
        Some(command) => writeln!(out, "editor.resolved={}", command.argv.join(" "))?,
        None => writeln!(out, "editor.resolved=unavailable")?,
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

fn set_stored_key(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(path) = &state.secret_key_path else {
        writeln!(out, "key storage is not configured; no key stored")?;
        return Ok(());
    };
    let key = configured_encryption_key(&state.encryption_config);
    if key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }
    let value = match read_api_key_from_env(&state.ai_config.env_key) {
        Ok(value) => value,
        Err(err) => {
            writeln!(out, "{err}")?;
            return Ok(());
        }
    };
    let record = StoredApiKey {
        env_key: state.ai_config.env_key.clone(),
        value,
    };
    let plaintext =
        serde_json::to_vec(&record).context("failed to serialize encrypted API key record")?;
    atomic_gpg_encrypt_bytes(gpg_program(), key, path, &plaintext)?;
    state.append_event(EventLevel::Info, "stored key encrypted")?;
    writeln!(out, "stored key encrypted")?;
    Ok(())
}

fn clear_stored_key(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(path) = &state.secret_key_path else {
        writeln!(out, "key storage is not configured; no key removed")?;
        return Ok(());
    };

    match std::fs::remove_file(path) {
        Ok(()) => {
            state.append_event(EventLevel::Info, "stored key cleared")?;
            writeln!(out, "stored key cleared")?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            writeln!(out, "no stored key to clear")?;
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn load_stored_api_key(state: &AppState) -> Result<Option<String>> {
    let Some(path) = &state.secret_key_path else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let bytes = gpg_decrypt_file(gpg_program(), path)?;
    let record: StoredApiKey =
        serde_json::from_slice(&bytes).context("stored API key record is not valid JSON")?;
    if record.value.trim().is_empty() {
        anyhow::bail!("stored API key is empty");
    }
    Ok(Some(record.value))
}

fn ai_config_for_request(state: &AppState) -> Result<AiConfig> {
    let mut config = state.ai_config.clone();
    config.api_key_override = None;
    if read_api_key_from_env(&config.env_key).is_ok() {
        return Ok(config);
    }
    if let Some(api_key) = load_stored_api_key(state)? {
        config.api_key_override = Some(api_key);
    }
    Ok(config)
}

fn update_encryption_config(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let parts: Vec<_> = args.split_whitespace().collect();
    match parts.as_slice() {
        ["on"] => enable_encryption(state, out, None),
        ["on", key_selector] => enable_encryption(state, out, Some(key_selector)),
        ["rotate", key_selector] => rotate_encryption_key(state, out, Some(key_selector)),
        ["rewrite-history", "plan"] => plan_encryption_history_rewrite(state, out),
        ["rewrite-history", "run", key_selector, "--confirm-rewrite-history"] => {
            run_encryption_history_rewrite(state, out, key_selector)
        }
        ["off"] => disable_encryption(state, out),
        _ => writeln!(
            out,
            "usage: #encrypt on [key-fingerprint|unique-email] | #encrypt rotate <key-fingerprint|unique-email> | #encrypt rewrite-history plan | #encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history | #encrypt off"
        )
        .map_err(Into::into),
    }
}

fn enable_encryption(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector_arg: Option<&str>,
) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }
    let selector = encryption_key_selector(state, key_selector_arg);
    if selector.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), &selector)?;
    state.flush_encrypted_writes()?;
    let encrypted_cache = encrypted_writer_cache_from_storage(state)?;
    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    let summary = rewrite_storage_for_encryption_key(state, &current_key, &fingerprint)?;
    set_encryption_config(state, |config| {
        config.encryption.enabled = true;
        config.encryption.key_fingerprint = fingerprint.clone();
        config.encryption.recipient.clear();
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "{}", encryption_git_history_warning())?;
    writeln!(out, "encryption=on")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &summary)?;
    Ok(())
}

fn encryption_key_selector(state: &AppState, key_selector_arg: Option<&str>) -> String {
    key_selector_arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| configured_encryption_key(&state.encryption_config))
        .to_string()
}

fn rotate_encryption_key(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector_arg: Option<&str>,
) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }
    let Some(selector) = key_selector_arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        writeln!(out, "usage: #encrypt rotate <key-fingerprint|unique-email>")?;
        return Ok(());
    };

    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), selector)?;
    state.flush_encrypted_writes()?;
    let encrypted_cache = encrypted_writer_cache_from_storage(state)?;
    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    let summary = rewrite_storage_for_encryption_key(state, &current_key, &fingerprint)?;
    set_encryption_config(state, |config| {
        config.encryption.enabled = true;
        config.encryption.key_fingerprint = fingerprint.clone();
        config.encryption.recipient.clear();
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "encryption=on")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &summary)?;
    Ok(())
}

fn disable_encryption(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }

    state.flush_encrypted_writes()?;
    state.stop_encrypted_writer();
    migrate_storage_to_plaintext(state)?;
    set_encryption_config(state, |config| {
        config.encryption.enabled = false;
    })?;
    writeln!(out, "encryption=off")?;
    writeln!(
        out,
        "plaintext history and templates will be written from now on"
    )?;
    Ok(())
}

fn set_encryption_config(
    state: &mut AppState,
    update: impl FnOnce(&mut config::Config),
) -> Result<()> {
    let Some(path) = &state.config_path else {
        anyhow::bail!("config path is not configured; #encrypt not saved");
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config);
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.encryption_config = config.encryption;
    state.append_event(EventLevel::Info, "encryption config changed")?;
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct EncryptionRewriteSummary {
    plaintext_encrypted: usize,
    reencrypted: usize,
    already_encrypted: usize,
    missing: usize,
}

fn rewrite_storage_for_encryption_key(
    state: &AppState,
    old_key: &str,
    new_key: &str,
) -> Result<EncryptionRewriteSummary> {
    let mut summary = EncryptionRewriteSummary::default();
    for path in encrypted_storage_paths(state) {
        let encrypted = encrypted_path(&path);
        match (path.exists(), encrypted.exists()) {
            (true, true) => {
                anyhow::bail!(
                    "both plaintext and encrypted storage exist for {}; resolve this before changing encryption keys",
                    path.display()
                );
            }
            (true, false) => {
                if migrate_plaintext_jsonl_to_gpg(gpg_program(), new_key, &path)? {
                    summary.plaintext_encrypted += 1;
                }
            }
            (false, true) if old_key != new_key => {
                if reencrypt_gpg_jsonl(gpg_program(), new_key, &path)? {
                    summary.reencrypted += 1;
                }
            }
            (false, true) => {
                summary.already_encrypted += 1;
            }
            (false, false) => {
                summary.missing += 1;
            }
        }
    }
    Ok(summary)
}

fn write_encryption_rewrite_summary(
    out: &mut impl Write,
    summary: &EncryptionRewriteSummary,
) -> Result<()> {
    writeln!(
        out,
        "encrypted_plaintext_files={}",
        summary.plaintext_encrypted
    )?;
    writeln!(out, "reencrypted_files={}", summary.reencrypted)?;
    writeln!(out, "already_encrypted_files={}", summary.already_encrypted)?;
    Ok(())
}

fn plan_encryption_history_rewrite(state: &AppState, out: &mut impl Write) -> Result<()> {
    let Some(config_path) = &state.config_path else {
        writeln!(
            out,
            "config path is not configured; cannot plan history rewrite"
        )?;
        return Ok(());
    };
    let Some(root) = config_path.parent() else {
        writeln!(
            out,
            "config path has no parent; cannot plan history rewrite"
        )?;
        return Ok(());
    };
    let key = configured_encryption_key(&state.encryption_config);
    if key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    writeln!(out, "history rewrite plan")?;
    writeln!(out, "repo={}", root.display())?;
    writeln!(out, "target_key_fingerprint={key}")?;
    writeln!(
        out,
        "risk=rewrites commit ids and requires a force push for any shared remote"
    )?;
    writeln!(
        out,
        "scope=current branch; managed history, draft, note, AI, and template storage paths"
    )?;
    writeln!(
        out,
        "next=#encrypt rewrite-history run <key-fingerprint> --confirm-rewrite-history"
    )?;
    writeln!(
        out,
        "note=the run command must decrypt old encrypted blobs with the old private key, then encrypt each rewritten blob for the target key"
    )?;
    Ok(())
}

fn run_encryption_history_rewrite(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector: &str,
) -> Result<()> {
    let Some(config_path) = &state.config_path else {
        writeln!(out, "config path is not configured; cannot rewrite history")?;
        return Ok(());
    };
    let Some(root) = config_path.parent().map(Path::to_path_buf) else {
        writeln!(out, "config path has no parent; cannot rewrite history")?;
        return Ok(());
    };
    if !root.join(".git").is_dir() {
        writeln!(
            out,
            "git repository is not initialized; run #push before rewriting history"
        )?;
        return Ok(());
    }
    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    if current_key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    state.flush_encrypted_writes()?;
    let encrypted_cache = encrypted_writer_cache_from_storage(state)?;
    let clean = run_git_command(
        &root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["status".to_string(), "--porcelain".to_string()],
        },
    )?;
    if !clean.success || !clean.stdout.trim().is_empty() {
        writeln!(
            out,
            "history rewrite requires a clean git worktree; commit, stash, or discard changes first"
        )?;
        return Ok(());
    }

    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), key_selector)?;
    let script_path = write_history_rewrite_script(&root, state)?;
    let backup_ref = format!(
        "aish/rewrite-backup/{}-{}",
        (state.clock)(),
        std::process::id()
    );
    let backup = run_git_command(
        &root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["branch".to_string(), backup_ref.clone(), "HEAD".to_string()],
        },
    )?;
    if !backup.success {
        let _ = fs::remove_file(&script_path);
        anyhow::bail!(
            "failed to create rewrite backup branch: {}",
            backup.combined_output()
        );
    }

    let filter_result = run_git_filter_branch_reencrypt(&root, &script_path, &fingerprint);
    let _ = fs::remove_file(&script_path);
    let filter_result = filter_result?;
    if !filter_result.success {
        anyhow::bail!(
            "git history rewrite failed: {}",
            filter_result.combined_output()
        );
    }

    let untracked =
        rewrite_untracked_storage_for_encryption_key(state, &root, &current_key, &fingerprint)?;
    set_encryption_config(state, |config| {
        config.encryption.enabled = true;
        config.encryption.key_fingerprint = fingerprint.clone();
        config.encryption.recipient.clear();
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "history rewrite completed")?;
    writeln!(out, "backup_branch={backup_ref}")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &untracked)?;
    writeln!(
        out,
        "next=verify the rewritten history, push with --force-with-lease if appropriate, then remove backup refs and expire reflogs only after an external backup"
    )?;
    Ok(())
}

fn write_history_rewrite_script(root: &Path, state: &AppState) -> Result<PathBuf> {
    let script_dir = root.join("cache/runtime");
    fs::create_dir_all(&script_dir).with_context(|| {
        format!(
            "failed to create rewrite script directory {}",
            script_dir.display()
        )
    })?;
    let script_path = script_dir.join("encrypt-rewrite-history.sh");
    let mut script = String::from(
        "#!/bin/sh\nset -eu\ngpg_program=${AISH_REWRITE_GPG:-gpg}\nrecipient=${AISH_REWRITE_RECIPIENT:?}\nreencrypt_file() {\n  plain=$1\n  enc=$plain.gpg\n  if [ -f \"$plain\" ] && [ -f \"$enc\" ]; then\n    printf '%s\\n' \"both plaintext and encrypted files exist: $plain\" >&2\n    exit 3\n  fi\n  if [ -f \"$plain\" ]; then\n    \"$gpg_program\" --batch --yes --no-tty --trust-model always --encrypt --recipient \"$recipient\" --output \"$enc.tmp\" \"$plain\"\n    mv \"$enc.tmp\" \"$enc\"\n    rm -f \"$plain\"\n  elif [ -f \"$enc\" ]; then\n    tmp=\"$enc.plain.$$\"\n    \"$gpg_program\" --yes --decrypt \"$enc\" > \"$tmp\"\n    \"$gpg_program\" --batch --yes --no-tty --trust-model always --encrypt --recipient \"$recipient\" --output \"$enc.tmp\" \"$tmp\"\n    rm -f \"$tmp\"\n    mv \"$enc.tmp\" \"$enc\"\n  fi\n}\n",
    );
    for relative in managed_relative_storage_paths(root, state)? {
        script.push_str("reencrypt_file ");
        script.push_str(&shell_single_quote(&relative));
        script.push('\n');
    }
    fs::write(&script_path, script)
        .with_context(|| format!("failed to write rewrite script {}", script_path.display()))?;
    Ok(script_path)
}

fn run_git_filter_branch_reencrypt(
    root: &Path,
    script_path: &Path,
    fingerprint: &str,
) -> Result<GitStepResult> {
    let filter_command = format!(
        "sh {}",
        shell_single_quote(&script_path.display().to_string())
    );
    let _raw_mode_pause = pause_terminal_raw_mode_for_gpg()?;
    let mut command = Command::new("git");
    command
        .args([
            "filter-branch",
            "-f",
            "--tree-filter",
            &filter_command,
            "--",
            "HEAD",
        ])
        .current_dir(root)
        .env("FILTER_BRANCH_SQUELCH_WARNING", "1")
        .env("AISH_REWRITE_GPG", gpg_program())
        .env("AISH_REWRITE_RECIPIENT", fingerprint);
    prepare_gpg_terminal_env(&mut command);
    let output = command
        .output()
        .context("failed to run git filter-branch")?;
    Ok(GitStepResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn rewrite_untracked_storage_for_encryption_key(
    state: &AppState,
    root: &Path,
    old_key: &str,
    new_key: &str,
) -> Result<EncryptionRewriteSummary> {
    let mut summary = EncryptionRewriteSummary::default();
    for path in encrypted_storage_paths(state) {
        let relative = match path.strip_prefix(root) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        if git_path_is_tracked(root, relative)? {
            continue;
        }
        let encrypted = encrypted_path(&path);
        match (path.exists(), encrypted.exists()) {
            (true, true) => {
                anyhow::bail!(
                    "both plaintext and encrypted storage exist for {}; resolve this before changing encryption keys",
                    path.display()
                );
            }
            (true, false) => {
                if migrate_plaintext_jsonl_to_gpg(gpg_program(), new_key, &path)? {
                    summary.plaintext_encrypted += 1;
                }
            }
            (false, true) if old_key != new_key => {
                if reencrypt_gpg_jsonl(gpg_program(), new_key, &path)? {
                    summary.reencrypted += 1;
                }
            }
            (false, true) => summary.already_encrypted += 1,
            (false, false) => summary.missing += 1,
        }
    }
    Ok(summary)
}

fn git_path_is_tracked(root: &Path, relative: &Path) -> Result<bool> {
    let result = run_git_command(
        root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "ls-files".to_string(),
                "--error-unmatch".to_string(),
                "--".to_string(),
                relative.display().to_string(),
            ],
        },
    )?;
    Ok(result.success)
}

fn managed_relative_storage_paths(root: &Path, state: &AppState) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for path in encrypted_storage_paths(state) {
        let relative = path.strip_prefix(root).with_context(|| {
            format!(
                "managed storage path is outside git root: {}",
                path.display()
            )
        })?;
        paths.push(relative.display().to_string());
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn migrate_storage_to_plaintext(state: &AppState) -> Result<()> {
    for path in encrypted_storage_paths(state) {
        migrate_gpg_jsonl_to_plaintext(gpg_program(), path)?;
    }
    Ok(())
}

fn encrypted_storage_paths(state: &AppState) -> Vec<PathBuf> {
    [
        &state.regular_history_path,
        &state.ai_history_path,
        &state.draft_history_path,
        &state.notes_path,
        &state.template_store_path,
    ]
    .into_iter()
    .filter_map(|path| path.clone())
    .collect()
}

fn encrypted_writer_cache_from_storage(state: &AppState) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let program = gpg_program();
    let mut cache = HashMap::new();
    for path in encrypted_storage_paths(state) {
        let bytes = existing_jsonl_bytes(&program, &path)?;
        cache.insert(path, bytes);
    }
    Ok(cache)
}

fn set_sync_remote(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let remote = args.trim();
    if remote.is_empty() {
        writeln!(out, "usage: #set-remote <git-url>")?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; sync config not saved")?;
        return Ok(());
    }

    update_sync_config(state, |config| {
        config.sync.remote = remote.to_string();
    })?;
    writeln!(out, "sync.remote={remote}")?;
    writeln!(out, "no git command run")?;
    Ok(())
}

fn set_sync_schedule(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let args = args.trim();
    if args.is_empty() {
        write_encryption_sync_status(state, out)?;
        writeln!(out, "no git command run")?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; sync config not saved")?;
        return Ok(());
    }
    if args == "off" {
        update_sync_config(state, |config| {
            config.sync.enabled = false;
            config.sync.schedule.clear();
        })?;
        writeln!(out, "sync.enabled=false")?;
        writeln!(out, "no scheduler file created")?;
        return Ok(());
    }
    if let Some((category, enabled)) = parse_sync_category_toggle(args) {
        update_sync_config(state, |config| match category {
            "ai" => config.sync.ai = enabled,
            "history" => config.sync.history = enabled,
            "templates" => config.sync.templates = enabled,
            "drafts" => config.sync.drafts = enabled,
            _ => unreachable!("validated category"),
        })?;
        writeln!(out, "sync.{category}={enabled}")?;
        writeln!(out, "no git command run")?;
        return Ok(());
    }
    if is_malformed_sync_category_toggle(args) {
        writeln!(out, "usage: #sync ai|history|templates|drafts on|off")?;
        return Ok(());
    }

    update_sync_config(state, |config| {
        config.sync.enabled = true;
        config.sync.schedule = args.to_string();
    })?;
    writeln!(out, "sync.enabled=true")?;
    writeln!(out, "sync.schedule={args}")?;
    writeln!(out, "no scheduler file created")?;
    Ok(())
}

fn run_manual_sync_push(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let remote = state.sync_config.remote.trim();
    if remote.is_empty() {
        writeln!(
            out,
            "sync remote is not configured; run #set-remote <git-url> first"
        )?;
        return Ok(());
    }
    let Some(root) = sync_root(state) else {
        writeln!(out, "config path is not configured; sync push cannot run")?;
        return Ok(());
    };
    state.flush_encrypted_writes()?;
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };

    maintain_managed_gitignore(root.join(".gitignore"))?;
    let mut initialized_repo = false;
    if root.join(".git").is_dir() {
        warn_tracked_managed_paths(&root, out)?;
    } else if let Some(plan) = init_repo_plan(remote) {
        for command in &plan.commands {
            run_sync_git_step(state, out, &root, command)?;
        }
        initialized_repo = true;
    }

    for command in conservative_sync_plan_for_existing_paths_with_encryption(
        &root,
        &state.sync_config,
        state.encryption_config.enabled,
    )
    .commands
    {
        if initialized_repo && is_pull_rebase_command(&command) {
            writeln!(
                out,
                "sync step skipped: git pull --rebase for new repository"
            )?;
            continue;
        }
        if is_commit_command(&command) {
            let result = run_git_command(&root, &command)?;
            if result.success || git_output_is_nothing_to_commit(&result.combined_output()) {
                if result.success {
                    writeln!(out, "sync step ok: git commit")?;
                } else {
                    writeln!(out, "sync step skipped: nothing to commit")?;
                }
                continue;
            }
            handle_failed_sync_step(state, out, &command, result)?;
            return Ok(());
        }
        if !run_sync_git_step(state, out, &root, &command)? {
            return Ok(());
        }
    }
    state.append_event(EventLevel::Info, "sync push completed")?;
    writeln!(out, "sync push completed")?;
    Ok(())
}

fn run_startup_sync_check(state: &mut AppState, root: &Path, out: &mut impl Write) -> Result<()> {
    let last_attempt_path = root.join("cache/runtime/sync.last_attempt");
    let now = (state.clock)();
    match startup_sync_decision(
        &state.sync_config,
        now,
        read_last_sync_attempt(&last_attempt_path)?,
    ) {
        StartupSyncDecision::Due => {
            write_last_sync_attempt(&last_attempt_path, now)?;
            writeln!(out, "startup sync due; running #push")?;
            run_manual_sync_push(state, out)?;
        }
        StartupSyncDecision::UnsupportedSchedule(schedule) => {
            state.append_event(
                EventLevel::Warn,
                &format!("startup sync unsupported schedule: {schedule}"),
            )?;
        }
        StartupSyncDecision::Disabled
        | StartupSyncDecision::MissingRemote
        | StartupSyncDecision::MissingSchedule
        | StartupSyncDecision::NotDue { .. } => {}
    }
    Ok(())
}

fn read_last_sync_attempt(path: &Path) -> Result<Option<i64>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(raw.trim().parse::<i64>().ok()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err)
            .with_context(|| format!("failed to read startup sync timestamp {}", path.display())),
    }
}

fn write_last_sync_attempt(path: &Path, value: i64) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create startup sync timestamp directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(path, format!("{value}\n"))
        .with_context(|| format!("failed to write startup sync timestamp {}", path.display()))
}

fn sync_root(state: &AppState) -> Option<PathBuf> {
    state.config_path.as_ref()?.parent().map(Path::to_path_buf)
}

fn warn_tracked_managed_paths(root: &Path, out: &mut impl Write) -> Result<()> {
    let plan = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["ls-files".to_string()],
    };
    let result = run_git_command(root, &plan)?;
    if let Some(warning) = tracked_managed_files_warning(result.stdout.lines()) {
        writeln!(out, "{}", warning.message)?;
        for path in warning.paths {
            writeln!(out, "tracked: {path}")?;
        }
    }
    Ok(())
}

fn run_sync_git_step(
    state: &AppState,
    out: &mut impl Write,
    root: &Path,
    command: &GitCommandPlan,
) -> Result<bool> {
    let result = run_git_command(root, command)?;
    if result.success {
        writeln!(out, "sync step ok: {}", describe_git_command(command))?;
        return Ok(true);
    }
    handle_failed_sync_step(state, out, command, result)?;
    Ok(false)
}

fn handle_failed_sync_step(
    state: &AppState,
    out: &mut impl Write,
    command: &GitCommandPlan,
    result: GitStepResult,
) -> Result<()> {
    let detail = result.combined_output();
    match classify_git_sync_step(false, &result.stdout, &result.stderr) {
        SyncStepOutcome::AbortConflict { .. } => {
            writeln!(
                out,
                "sync aborted on conflict: {}",
                describe_git_command(command)
            )?;
            if let Some(path) = &state.events_path {
                log_sync_failure(path, (state.clock)(), SyncFailureKind::Conflict, &detail)?;
            }
        }
        SyncStepOutcome::AbortFailure { .. } => {
            writeln!(out, "sync failed: {}", describe_git_command(command))?;
            if let Some(path) = &state.events_path {
                log_sync_failure(path, (state.clock)(), SyncFailureKind::Failure, &detail)?;
            }
        }
        SyncStepOutcome::Continue => unreachable!("failed git step cannot continue"),
    }
    let detail = detail.trim();
    if !detail.is_empty() {
        writeln!(out, "{detail}")?;
    }
    Ok(())
}

#[derive(Debug)]
struct GitStepResult {
    success: bool,
    stdout: String,
    stderr: String,
}

impl GitStepResult {
    fn combined_output(&self) -> String {
        let stdout = self.stdout.trim();
        let stderr = self.stderr.trim();
        match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => String::new(),
            (false, true) => stdout.to_string(),
            (true, false) => stderr.to_string(),
            (false, false) => format!("{stdout}\n{stderr}"),
        }
    }
}

fn run_git_command(root: &Path, command: &GitCommandPlan) -> Result<GitStepResult> {
    let output = Command::new(&command.program)
        .args(&command.args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run {}", describe_git_command(command)))?;
    Ok(GitStepResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn is_commit_command(command: &GitCommandPlan) -> bool {
    command.program == "git" && command.args.first().is_some_and(|arg| arg == "commit")
}

fn is_pull_rebase_command(command: &GitCommandPlan) -> bool {
    command.program == "git"
        && command
            .args
            .iter()
            .map(String::as_str)
            .eq(["pull", "--rebase"])
}

fn git_output_is_nothing_to_commit(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("nothing to commit") || lower.contains("no changes added to commit")
}

fn describe_git_command(command: &GitCommandPlan) -> String {
    let mut parts = Vec::with_capacity(command.args.len() + 1);
    parts.push(command.program.as_str());
    parts.extend(command.args.iter().map(String::as_str));
    parts.join(" ")
}

fn parse_sync_category_toggle(args: &str) -> Option<(&str, bool)> {
    let mut parts = args.split_whitespace();
    let category = parts.next()?;
    let value = parts.next()?;
    if parts.next().is_some() || !is_sync_category(category) {
        return None;
    }
    match value {
        "on" => Some((category, true)),
        "off" => Some((category, false)),
        _ => None,
    }
}

fn is_malformed_sync_category_toggle(args: &str) -> bool {
    let mut parts = args.split_whitespace();
    let Some(category) = parts.next() else {
        return false;
    };
    is_sync_category(category)
}

fn is_sync_category(value: &str) -> bool {
    matches!(value, "ai" | "history" | "templates" | "drafts")
}

fn update_sync_config(
    state: &mut AppState,
    update: impl FnOnce(&mut config::Config),
) -> Result<()> {
    let Some(path) = &state.config_path else {
        anyhow::bail!("config path is not configured; sync config not saved");
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config);
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.sync_config = config.sync;
    state.append_event(EventLevel::Info, "sync config changed")?;
    Ok(())
}

fn update_ai_config_field(
    state: &mut AppState,
    out: &mut impl Write,
    name: &str,
    args: &str,
) -> Result<()> {
    let value = args.trim();
    if value.is_empty() {
        write_ai_config_value(out, name, state)?;
        return Ok(());
    }
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #{name} not saved")?;
        return Ok(());
    };

    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    match name {
        "model" => config.ai.model = value.to_string(),
        "base-url" => config.ai.base_url = normalize_chat_completions_url(value)?,
        "env-key" => config.ai.env_key = value.to_string(),
        _ => unreachable!("unknown AI config field"),
    }
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.ai_config = config.ai;
    write_ai_config_value(out, name, state)
}

fn write_ai_config_value(out: &mut impl Write, name: &str, state: &AppState) -> Result<()> {
    let value = match name {
        "model" => &state.ai_config.model,
        "base-url" => &state.ai_config.base_url,
        "env-key" => &state.ai_config.env_key,
        _ => unreachable!("unknown AI config field"),
    };
    if value.is_empty() {
        writeln!(out, "#{name}=unconfigured")?;
    } else {
        writeln!(out, "#{name}={value}")?;
    }
    Ok(())
}

fn update_context_config(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => write_context_config(out, &state.context_config),
        (Some("on"), None, None) => set_context_config(state, out, |config| {
            config.context.enabled = true;
            Ok(())
        }),
        (Some("off"), None, None) => set_context_config(state, out, |config| {
            config.context.enabled = false;
            Ok(())
        }),
        (Some("confirm"), Some("on"), None) => set_context_config(state, out, |config| {
            config.context.confirm = true;
            Ok(())
        }),
        (Some("confirm"), Some("off"), None) => set_context_config(state, out, |config| {
            config.context.confirm = false;
            Ok(())
        }),
        (Some(bytes), None, None) => {
            let max_bytes = bytes.parse::<usize>()?;
            if max_bytes == 0 {
                writeln!(out, "context max bytes must be greater than 0")?;
                return Ok(());
            }
            set_context_config(state, out, |config| {
                config.context.max_bytes = max_bytes;
                Ok(())
            })
        }
        _ => writeln!(
            out,
            "usage: #context [on|off|confirm on|confirm off|<bytes>]"
        )
        .map_err(Into::into),
    }
}

fn set_context_config(
    state: &mut AppState,
    out: &mut impl Write,
    update: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<()> {
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #context not saved")?;
        return Ok(());
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config)?;
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.context_config = config.context;
    write_context_config(out, &state.context_config)
}

fn write_context_config(out: &mut impl Write, config: &ContextConfig) -> Result<()> {
    writeln!(
        out,
        "context.enabled={} context.confirm={} context.max_bytes={}",
        config.enabled, config.confirm, config.max_bytes
    )?;
    Ok(())
}

fn update_completion_config(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => write_completion_config(out, &state.completion_config),
        (Some(value @ ("on" | "off")), None, None) => {
            let mode = if value == "on" {
                CompletionMode::Auto
            } else {
                CompletionMode::Off
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(mode);
                Ok(())
            })
        }
        (Some("mode"), Some(value), None) => {
            let Some(mode) = parse_completion_mode(value) else {
                writeln!(out, "usage: #completion mode auto|tab|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(mode);
                Ok(())
            })
        }
        (Some("max"), Some(count), None) => {
            let max_results = count.parse::<usize>();
            let Ok(max_results) = max_results else {
                writeln!(out, "usage: #completion max <count>")?;
                return Ok(());
            };
            if max_results == 0 {
                writeln!(out, "completion max results must be greater than 0")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.max_results = max_results;
                Ok(())
            })
        }
        (Some("coalesce" | "coalesce-ms"), Some(value), None) => {
            let Ok(coalesce_ms) = value.parse::<u64>() else {
                writeln!(out, "usage: #completion coalesce-ms <0-1000>")?;
                return Ok(());
            };
            if coalesce_ms > 1_000 {
                writeln!(out, "completion coalesce ms must be between 0 and 1000")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.coalesce_ms = coalesce_ms;
                Ok(())
            })
        }
        (Some("display-delay" | "display-delay-ms"), Some(value), None) => {
            let Ok(display_delay_ms) = value.parse::<u64>() else {
                writeln!(out, "usage: #completion display-delay-ms <0-1000>")?;
                return Ok(());
            };
            if display_delay_ms > 1_000 {
                writeln!(out, "completion display delay ms must be between 0 and 1000")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.display_delay_ms = display_delay_ms;
                Ok(())
            })
        }
        (Some("inline"), Some(value), None) => {
            let Some(inline) = parse_on_off(value) else {
                writeln!(out, "usage: #completion inline on|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(if inline {
                    CompletionMode::Auto
                } else {
                    CompletionMode::Tab
                });
                Ok(())
            })
        }
        (Some("fuzzy"), Some(value), None) => {
            let Some(fuzzy) = parse_on_off(value) else {
                writeln!(out, "usage: #completion fuzzy on|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.fuzzy = fuzzy;
                Ok(())
            })
        }
        (Some("tab-accept"), Some(value), None) => {
            let Some(tab_accept) = parse_completion_tab_accept(value) else {
                writeln!(out, "usage: #completion tab-accept full|word")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.tab_accept = tab_accept;
                Ok(())
            })
        }
        (Some("match-threshold"), Some(value), None) => {
            let Ok(percent) = value.parse::<usize>() else {
                writeln!(out, "usage: #completion match-threshold <0-100>")?;
                return Ok(());
            };
            if percent > 100 {
                writeln!(out, "completion match threshold must be between 0 and 100")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.match_threshold_percent = percent;
                Ok(())
            })
        }
        (Some("typo-threshold"), Some(value), None) => {
            let Ok(percent) = value.parse::<usize>() else {
                writeln!(out, "usage: #completion typo-threshold <0-100>")?;
                return Ok(());
            };
            if percent > 100 {
                writeln!(out, "completion typo threshold must be between 0 and 100")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.typo_threshold_percent = percent;
                Ok(())
            })
        }
        _ => writeln!(
            out,
            "usage: #completion on|off|mode auto|tab|off|max <count>|coalesce-ms <0-1000>|display-delay-ms <0-1000>|inline on|off|fuzzy on|off|tab-accept full|word|match-threshold <0-100>|typo-threshold <0-100>"
        )
        .map_err(Into::into),
    }
}

fn set_completion_config(
    state: &mut AppState,
    out: &mut impl Write,
    update: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<()> {
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #completion not saved")?;
        return Ok(());
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config)?;
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.completion_config = config.completion;
    write_completion_config(out, &state.completion_config)
}

fn write_completion_config(out: &mut impl Write, config: &CompletionConfig) -> Result<()> {
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

fn parse_completion_mode(value: &str) -> Option<CompletionMode> {
    match value {
        "auto" => Some(CompletionMode::Auto),
        "tab" => Some(CompletionMode::Tab),
        "off" => Some(CompletionMode::Off),
        _ => None,
    }
}

fn parse_completion_tab_accept(value: &str) -> Option<CompletionTabAccept> {
    match value {
        "full" => Some(CompletionTabAccept::Full),
        "word" => Some(CompletionTabAccept::Word),
        _ => None,
    }
}

fn parse_on_off(value: &str) -> Option<bool> {
    match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn completion_tier_is_final(tier: CompletionTier, fuzzy_enabled: bool) -> bool {
    matches!(tier, CompletionTier::Typo)
        || !fuzzy_enabled && matches!(tier, CompletionTier::History)
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

fn completion_cwd(current_cwd: &Option<PathBuf>) -> PathBuf {
    current_cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

fn write_path_status(out: &mut impl Write, name: &str, path: &Option<PathBuf>) -> Result<()> {
    match path {
        Some(path) => writeln!(out, "{name}={} exists={}", path.display(), path.exists())?,
        None => writeln!(out, "{name}=unconfigured")?,
    }
    Ok(())
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
