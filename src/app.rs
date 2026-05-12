use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::ai::{normalize_chat_completions_url, request_ai_items};
use crate::commands::{
    IMPLEMENTED_PRIVATE_COMMANDS, ParsedLine, parse_line, suggest_private_command,
};
use crate::completion::{
    CompletionCandidate, CompletionOptions, complete_first_token_with_options,
    complete_non_first_token_with_options, current_token_context,
};
use crate::config::{
    self, AiConfig, CompletionConfig, ContextConfig, EditorConfig, PasteConfig, PromptConfig,
};
use crate::context::{
    build_contextual_ai_prompt, is_dangerous_context_command, run_context_command,
};
use crate::editor::{
    EditorCommand, EditorRunResult, PreparedEditorSession, prepare_editor_file, read_editor_file,
    resolve_editor_command, run_editor_command,
};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, HistorySource,
    HistoryStore, NoteEntry, ai_command_indices, append_jsonl, load_jsonl, trim_combined_history,
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
use crate::pty::PtyBackend;
use crate::templates::{
    TemplateEntry, append_template, apply_template_values_with_usage, find_template_by_name,
    load_templates, remove_templates_by_name, replace_template, template_placeholders,
};

const OUTPUT_RING_CAPACITY: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputEntry {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplates {
    pub draft: String,
    pub history: String,
    pub ai: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingContextPrompt {
    pub prompt: String,
    pub command: String,
    pub dangerous: bool,
}

impl Default for PromptTemplates {
    fn default() -> Self {
        Self {
            draft: "{mode} ".to_string(),
            history: "{mode} ".to_string(),
            ai: "{mode} ".to_string(),
        }
    }
}

impl From<PromptConfig> for PromptTemplates {
    fn from(config: PromptConfig) -> Self {
        Self {
            draft: config.draft,
            history: config.history,
            ai: config.ai,
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub mode: Mode,
    pub draft: InputBuffer,
    pub last_status: Option<i32>,
    pub current_cwd: Option<PathBuf>,
    pub exit_requested: bool,
    pub regular_history_path: Option<PathBuf>,
    pub ai_history_path: Option<PathBuf>,
    pub notes_path: Option<PathBuf>,
    pub draft_history_path: Option<PathBuf>,
    pub events_path: Option<PathBuf>,
    pub template_store_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub draft_persist: bool,
    pub regular_history: Vec<HistoryEntry>,
    pub selected_history_index: Option<usize>,
    pub ai_sessions: Vec<AiSession>,
    pub ai_command_indices: Vec<AiCommandIndex>,
    pub selected_ai_index: Option<usize>,
    pub output_ring: VecDeque<OutputEntry>,
    pub prompt_templates: PromptTemplates,
    pub editor_config: EditorConfig,
    pub editor_temp_root: Option<PathBuf>,
    pub paste_config: PasteConfig,
    pub completion_config: CompletionConfig,
    pub ai_config: AiConfig,
    pub context_config: ContextConfig,
    pub pending_context: Option<PendingContextPrompt>,
    pub draft_from_editor: bool,
    pub draft_from_template: bool,
    pub ctrl_x_prefix: bool,
    pub clock: fn() -> i64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Draft,
            draft: InputBuffer::new(),
            last_status: None,
            current_cwd: None,
            exit_requested: false,
            regular_history_path: None,
            ai_history_path: None,
            notes_path: None,
            draft_history_path: None,
            events_path: None,
            template_store_path: None,
            config_path: None,
            draft_persist: true,
            regular_history: Vec::new(),
            selected_history_index: None,
            ai_sessions: Vec::new(),
            ai_command_indices: Vec::new(),
            selected_ai_index: None,
            output_ring: VecDeque::new(),
            prompt_templates: PromptTemplates::default(),
            editor_config: EditorConfig::default(),
            editor_temp_root: None,
            paste_config: PasteConfig::default(),
            completion_config: CompletionConfig::default(),
            ai_config: AiConfig::default(),
            context_config: ContextConfig::default(),
            pending_context: None,
            draft_from_editor: false,
            draft_from_template: false,
            ctrl_x_prefix: false,
            clock: unix_timestamp,
        }
    }
}

impl AppState {
    pub fn handle_empty_tab(&mut self) {
        if self.draft.is_empty() {
            self.mode = self.mode.next_primary();
            if self.mode == Mode::History {
                self.select_newest_history_if_available();
            } else if self.mode == Mode::Ai {
                self.select_first_ai_if_available();
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
        self.draft_from_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
        true
    }

    pub fn select_first_ai_if_available(&mut self) {
        self.selected_ai_index = (!self.ai_command_indices.is_empty()).then_some(0);
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
        self.draft_from_editor = false;
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

    pub fn prepare_editor_session(
        &mut self,
        temp_root: &std::path::Path,
    ) -> Result<PreparedEditorSession> {
        self.copy_read_only_selection_to_draft();
        self.mode = Mode::Draft;
        prepare_editor_file(temp_root, self.draft.as_str())
    }

    pub fn replace_draft_from_editor_session(
        &mut self,
        session: &PreparedEditorSession,
    ) -> Result<()> {
        let content = read_editor_file(session)?;
        self.draft = InputBuffer::from(content);
        self.draft_from_editor = true;
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

    pub fn replace_draft_from_editor_text(&mut self, content: impl Into<String>) {
        self.draft = InputBuffer::from(content.into());
        self.draft_from_editor = true;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
    }

    pub fn completion_candidates(&self) -> Result<Vec<CompletionCandidate>> {
        if self.mode != Mode::Draft || self.draft_from_editor {
            return Ok(Vec::new());
        }
        let token = current_token_context(self.draft.as_str(), self.draft.cursor());
        let templates = match &self.template_store_path {
            Some(path) => load_templates(path)?.items,
            None => Vec::new(),
        };
        let history_newest_first: Vec<_> = self.regular_history.iter().rev().cloned().collect();
        let options = CompletionOptions {
            max_results: self.completion_config.max_results,
            ignore_spaces: self.completion_config.ignore_spaces,
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
            Ok(complete_non_first_token_with_options(
                &token.text,
                &completion_cwd(&self.current_cwd),
                &history_newest_first,
                &templates,
                options,
            ))
        }
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
        self.draft_from_editor = false;
        self.draft_from_template = false;
        self.mode = Mode::Draft;
    }

    pub fn template_picker_candidates(&self) -> Result<Vec<String>> {
        let Some(path) = &self.template_store_path else {
            return Ok(Vec::new());
        };
        let loaded = load_templates(path)?;
        Ok(template_picker_candidates(&loaded.items))
    }

    pub fn replace_draft_from_template_picker(&mut self, name: &str) -> Result<bool> {
        let Some(path) = &self.template_store_path else {
            return Ok(false);
        };
        let loaded = find_template_by_name(path, name)?;
        let Some(template) = loaded.items.first() else {
            return Ok(false);
        };
        self.draft = InputBuffer::from(template.body.clone());
        self.draft_from_editor = false;
        self.draft_from_template = true;
        self.mode = Mode::Draft;
        Ok(true)
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
        if let Some(path) = &self.ai_history_path {
            append_jsonl(path, &session)?;
        }
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

    pub fn prompt_prefix(&self) -> String {
        let template = match self.mode {
            Mode::History => &self.prompt_templates.history,
            Mode::Ai => &self.prompt_templates.ai,
            _ => &self.prompt_templates.draft,
        };
        self.render_prompt_template(template)
    }

    fn render_prompt_template(&self, template: &str) -> String {
        let mode = self.mode.symbol().to_string();
        let cwd = self
            .current_cwd
            .as_ref()
            .map(|cwd| display_cwd(cwd))
            .unwrap_or_default();
        let basename = self
            .current_cwd
            .as_ref()
            .and_then(|cwd| cwd.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        template
            .replace("{user}", &prompt_user())
            .replace("{host}", &prompt_host())
            .replace("{cwd}", &cwd)
            .replace("{basename}", basename)
            .replace("{mode}", &mode)
            .replace(
                "{last_status}",
                &self
                    .last_status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            )
    }

    pub fn render_prompt_line(&self) -> String {
        if let Some(pending) = &self.pending_context {
            let marker = if pending.dangerous {
                "[dangerous context confirmation: Y/n]"
            } else {
                "[context confirmation: Y/n]"
            };
            return format!("{}{}", self.prompt_prefix(), marker);
        }
        let text = match self.mode {
            Mode::History => self.selected_history_command().unwrap_or(""),
            Mode::Ai => self.selected_ai_command().unwrap_or(""),
            Mode::Draft if self.draft_from_editor => {
                return format!("{}{}", self.prompt_prefix(), self.editor_draft_summary());
            }
            _ => self.draft.as_str(),
        };
        format!("{}{}", self.prompt_prefix(), text)
    }

    pub fn terminal_cursor_column(&self) -> u16 {
        if let Some(pending) = &self.pending_context {
            let marker = if pending.dangerous {
                "[dangerous context confirmation: Y/n]"
            } else {
                "[context confirmation: Y/n]"
            };
            return (self.prompt_prefix().len() + marker.len()).min(u16::MAX as usize) as u16;
        }
        let cursor = match self.mode {
            Mode::History => self.selected_history_command().unwrap_or("").len(),
            Mode::Ai => self.selected_ai_command().unwrap_or("").len(),
            Mode::Draft if self.draft_from_editor => self.editor_draft_summary().len(),
            _ => self.draft.cursor(),
        };
        let column = self.prompt_prefix().len() + cursor;
        column.min(u16::MAX as usize) as u16
    }

    fn editor_draft_summary(&self) -> String {
        let bytes = self.draft.as_str().len();
        let lines = self.draft.as_str().lines().count().max(1);
        format!(
            "[editor draft: {lines} line(s), {bytes} byte(s); Ctrl-X Ctrl-E to edit, Enter to run]"
        )
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
}

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::default_aish_dir())?;
    let store = HistoryStore::load(&layout)?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        current_cwd: backend.initial_cwd().map(PathBuf::from),
        regular_history_path: Some(layout.regular_history),
        ai_history_path: Some(layout.ai_history),
        notes_path: Some(layout.notes),
        draft_history_path: Some(layout.draft_history),
        events_path: Some(layout.events),
        template_store_path: Some(layout.template_store),
        config_path: Some(layout.config),
        draft_persist: config.draft.persist,
        regular_history: store.regular,
        ai_sessions: store.ai_sessions,
        ai_command_indices: store.ai_command_indices,
        prompt_templates: config.prompt.into(),
        editor_config: config.editor,
        paste_config: config.paste,
        completion_config: config.completion,
        ai_config: config.ai,
        context_config: config.context,
        editor_temp_root: Some(layout.runtime_cache.join("editor")),
        ..AppState::default()
    };
    crate::terminal::run(
        &mut state,
        &mut backend,
        &mut io::stdout(),
        Duration::from_secs(60),
    )
}

pub fn execute_draft(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
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
                state.draft.clear();
                state.mode = Mode::Draft;
                return Ok(());
            }
            ParsedLine::Note { tag, text } => {
                if let Some(path) = &state.notes_path {
                    append_jsonl(
                        path,
                        &NoteEntry {
                            tag,
                            text: text.to_string(),
                        },
                    )?;
                }
                writeln!(out, "note stored")?;
                state.draft.clear();
                state.mode = Mode::Draft;
                return Ok(());
            }
            ParsedLine::Private { name, args } => {
                match name {
                    "exit" | "quit" => {
                        state.exit_requested = true;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "help" => {
                        writeln!(
                            out,
                            "Aish private commands: {}",
                            IMPLEMENTED_PRIVATE_COMMANDS
                                .iter()
                                .map(|name| format!("#{name}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )?;
                        writeln!(out, "Default keybindings:")?;
                        for binding in default_keybindings() {
                            let status = if binding.implemented {
                                "implemented"
                            } else {
                                "reserved"
                            };
                            writeln!(out, "{} [{}] - {}", binding.key, status, binding.action)?;
                        }
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "status" => {
                        writeln!(
                            out,
                            "mode={} last_status={} cwd={} keybindings={}",
                            state.mode.symbol(),
                            state
                                .last_status
                                .map(|status| status.to_string())
                                .unwrap_or_else(|| "none".to_string()),
                            state
                                .current_cwd
                                .as_ref()
                                .map(|cwd| cwd.display().to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                            default_keybindings().len()
                        )?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "config" => {
                        write_config_report(state, out)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "doctor" => {
                        write_doctor_report(state, out)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "model" => {
                        update_ai_config_field(state, out, "model", args)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "base-url" => {
                        update_ai_config_field(state, out, "base-url", args)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "env-key" => {
                        update_ai_config_field(state, out, "env-key", args)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "key" => {
                        match args.split_whitespace().next() {
                            Some("set") => {
                                writeln!(out, "#key set is not implemented yet; no key stored")?
                            }
                            Some("clear") => {
                                writeln!(out, "#key clear is not implemented yet; no key removed")?
                            }
                            _ => writeln!(out, "usage: #key set | #key clear")?,
                        }
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "context" => {
                        update_context_config(state, out, args)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "completion" => {
                        writeln!(out, "completion.enabled=false")?;
                        writeln!(out, "completion engine is not implemented yet")?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "log" => {
                        show_event_log(state, out, args)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "editor" => {
                        write_editor_report(state, out)?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "history" => {
                        let count = args.parse::<usize>();
                        match (count, &state.regular_history_path, &state.ai_history_path) {
                            (Ok(count), Some(regular_path), Some(ai_path)) => {
                                let loaded = trim_combined_history(regular_path, ai_path, count)?;
                                let keep_from = loaded.regular.items.len().saturating_sub(count);
                                state.regular_history = loaded.regular.items[keep_from..].to_vec();
                                state.ai_sessions = load_jsonl::<AiSession>(ai_path)?.items;
                                state.ai_command_indices = ai_command_indices(&state.ai_sessions);
                                state.selected_history_index = None;
                                state.selected_ai_index = None;
                                writeln!(
                                    out,
                                    "history trimmed to {count}; skipped {} bad regular line(s), {} bad ai line(s)",
                                    loaded.regular.errors.len(),
                                    loaded.ai_sessions.errors.len()
                                )?;
                            }
                            (Ok(_), _, _) => writeln!(out, "history storage is not configured")?,
                            (Err(_), _, _) => writeln!(out, "usage: #history <count>")?,
                        }
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "mt" => {
                        match parse_template_args(args) {
                            Some((name, body)) => match &state.template_store_path {
                                Some(path) => {
                                    append_template(
                                        path,
                                        &TemplateEntry {
                                            name: name.to_string(),
                                            body: body.to_string(),
                                        },
                                    )?;
                                    writeln!(out, "template stored: {name}")?;
                                }
                                None => writeln!(out, "template storage is not configured")?,
                            },
                            None => writeln!(out, "usage: #mt <name> <body>")?,
                        }
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "template" => {
                        let mut keep_draft = false;
                        match args.split_whitespace().next() {
                            Some("list") => match &state.template_store_path {
                                Some(path) => {
                                    let loaded = load_templates(path)?;
                                    if loaded.items.is_empty() {
                                        writeln!(out, "no templates stored")?;
                                    } else {
                                        for template in loaded.items {
                                            writeln!(out, "{}", template.name)?;
                                        }
                                    }
                                    if !loaded.errors.is_empty() {
                                        writeln!(
                                            out,
                                            "skipped {} bad template line(s)",
                                            loaded.errors.len()
                                        )?;
                                    }
                                }
                                None => writeln!(out, "template storage is not configured")?,
                            },
                            Some("rm") => match args.split_whitespace().nth(1) {
                                Some(name) => match &state.template_store_path {
                                    Some(path) => {
                                        let removal = remove_templates_by_name(path, name)?;
                                        writeln!(
                                            out,
                                            "template removed: {name} ({})",
                                            removal.removed
                                        )?;
                                        if !removal.errors.is_empty() {
                                            writeln!(
                                                out,
                                                "skipped {} bad template line(s)",
                                                removal.errors.len()
                                            )?;
                                        }
                                    }
                                    None => writeln!(out, "template storage is not configured")?,
                                },
                                None => writeln!(out, "{}", template_usage())?,
                            },
                            Some("replace") => match parse_template_subcommand_args(args) {
                                Some((name, body)) => match &state.template_store_path {
                                    Some(path) => {
                                        let removal = replace_template(
                                            path,
                                            TemplateEntry {
                                                name: name.to_string(),
                                                body: body.to_string(),
                                            },
                                        )?;
                                        writeln!(
                                            out,
                                            "template replaced: {name} (removed {})",
                                            removal.removed
                                        )?;
                                        if !removal.errors.is_empty() {
                                            writeln!(
                                                out,
                                                "skipped {} bad template line(s)",
                                                removal.errors.len()
                                            )?;
                                        }
                                    }
                                    None => writeln!(out, "template storage is not configured")?,
                                },
                                None => writeln!(out, "{}", template_usage())?,
                            },
                            Some("show") => match args.split_whitespace().nth(1) {
                                Some(name) => match &state.template_store_path {
                                    Some(path) => {
                                        let loaded = find_template_by_name(path, name)?;
                                        match loaded.items.first() {
                                            Some(template) => {
                                                writeln!(out, "template: {}", template.name)?;
                                                writeln!(out, "{}", template.body)?;
                                            }
                                            None => writeln!(out, "template not found: {name}")?,
                                        }
                                        if !loaded.errors.is_empty() {
                                            writeln!(
                                                out,
                                                "skipped {} bad template line(s)",
                                                loaded.errors.len()
                                            )?;
                                        }
                                    }
                                    None => writeln!(out, "template storage is not configured")?,
                                },
                                None => writeln!(out, "{}", template_usage())?,
                            },
                            Some("use") => match args.split_whitespace().nth(1) {
                                Some(name) => match &state.template_store_path {
                                    Some(path) => {
                                        let loaded = find_template_by_name(path, name)?;
                                        match loaded.items.first() {
                                            Some(template) => {
                                                let values = parse_template_values(args);
                                                let (rendered, used_keys) =
                                                    apply_template_values_with_usage(
                                                        &template.body,
                                                        &values,
                                                    );
                                                state.draft = InputBuffer::from(rendered);
                                                state.draft_from_editor = false;
                                                state.draft_from_template = true;
                                                keep_draft = true;
                                                writeln!(out, "template copied to draft: {name}")?;
                                                let placeholders =
                                                    template_placeholders(&template.body);
                                                if !placeholders.is_empty() {
                                                    writeln!(
                                                        out,
                                                        "template placeholders: {}",
                                                        placeholders.join(", ")
                                                    )?;
                                                }
                                                let mut unresolved =
                                                    template_placeholders(state.draft.as_str());
                                                unresolved.sort();
                                                if !unresolved.is_empty() {
                                                    writeln!(
                                                        out,
                                                        "unresolved template placeholders: {}",
                                                        unresolved.join(", ")
                                                    )?;
                                                }
                                                let mut unused_keys: Vec<_> = values
                                                    .keys()
                                                    .filter(|key| {
                                                        !used_keys.iter().any(|used| used == *key)
                                                    })
                                                    .cloned()
                                                    .collect();
                                                unused_keys.sort();
                                                if !unused_keys.is_empty() {
                                                    writeln!(
                                                        out,
                                                        "unused template values: {}",
                                                        unused_keys.join(", ")
                                                    )?;
                                                }
                                            }
                                            None => writeln!(out, "template not found: {name}")?,
                                        }
                                        if !loaded.errors.is_empty() {
                                            writeln!(
                                                out,
                                                "skipped {} bad template line(s)",
                                                loaded.errors.len()
                                            )?;
                                        }
                                    }
                                    None => writeln!(out, "template storage is not configured")?,
                                },
                                None => writeln!(out, "{}", template_usage())?,
                            },
                            _ => writeln!(out, "{}", template_usage())?,
                        }
                        if !keep_draft {
                            state.draft.clear();
                        }
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "encrypt" => {
                        writeln!(out, "encryption is not implemented yet; no data changed")?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "set-remote" => {
                        writeln!(
                            out,
                            "sync remote configuration is not implemented yet; no remote changed"
                        )?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "push" => {
                        writeln!(out, "sync push is not implemented yet; no git command run")?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    "sync" => {
                        writeln!(out, "sync is not implemented yet; no git command run")?;
                        state.draft.clear();
                        state.mode = Mode::Draft;
                        return Ok(());
                    }
                    _ => {}
                }
                match suggest_private_command(name) {
                    Some(suggestion) => writeln!(
                        out,
                        "Aish command not implemented yet: #{name}. Did you mean #{suggestion}?"
                    )?,
                    None => writeln!(out, "Aish command not implemented yet: #{name}")?,
                }
                state.draft.clear();
                state.mode = Mode::Draft;
                return Ok(());
            }
            ParsedLine::AiPrompt(prompt) => {
                submit_ai_prompt(state, prompt, out)?;
                state.draft.clear();
                return Ok(());
            }
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, out, timeout)?;
                state.draft.clear();
                return Ok(());
            }
        }
    }

    state.mode = Mode::CommandRunning;
    let result = backend.run_command(&command, timeout)?;
    if !result.output.is_empty() {
        writeln!(out, "{}", result.output)?;
    }
    state.push_output_entry(OutputEntry {
        command: result.command.clone(),
        output: result.output.clone(),
        exit_code: result.exit_code,
    });
    if let Some(path) = &state.regular_history_path {
        let entry = HistoryEntry {
            command: result.command.clone(),
            t: (state.clock)(),
            exit_code: Some(result.exit_code),
            source: if executing_ai {
                HistorySource::Ai
            } else {
                HistorySource::User
            },
        };
        append_jsonl(path, &entry)?;
        state.regular_history.push(entry);
    }
    state.last_status = Some(result.exit_code);
    if let Some(cwd) = result.cwd {
        state.current_cwd = Some(PathBuf::from(cwd));
    }
    state.draft.clear();
    state.draft_from_editor = false;
    state.draft_from_template = false;
    if executing_ai && result.exit_code == 0 {
        state.advance_after_ai_success();
    } else if executing_ai {
        state.mode = Mode::Ai;
    } else {
        state.mode = Mode::Draft;
    }
    Ok(())
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
    match request_ai_items(&state.ai_config, prompt) {
        Ok(items) => {
            let item_count = items.len();
            let model = state.ai_config.model.clone();
            if state.store_ai_session_from_items(prompt, &model, items)? {
                state.append_event(
                    EventLevel::Info,
                    &format!("AI generated {item_count} item(s)"),
                )?;
                writeln!(
                    out,
                    "AI items generated: {}",
                    state.ai_command_indices.len()
                )?;
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
    write_editor_resolution(out, state)?;
    write_path_status(out, "regular_history_path", &state.regular_history_path)?;
    write_path_status(out, "notes_path", &state.notes_path)?;
    write_path_status(out, "draft_history_path", &state.draft_history_path)?;
    Ok(())
}

fn write_config_report(state: &AppState, out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish config")?;
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
    writeln!(
        out,
        "completion.max_results={}",
        state.completion_config.max_results
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

    let mut config = config::load_config(path)?;
    match name {
        "model" => config.ai.model = value.to_string(),
        "base-url" => config.ai.base_url = normalize_chat_completions_url(value)?,
        "env-key" => config.ai.env_key = value.to_string(),
        _ => unreachable!("unknown AI config field"),
    }
    config::normalize_config(&mut config);
    config::save_config(path, &config)?;
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
    let mut config = config::load_config(path)?;
    update(&mut config)?;
    config::normalize_config(&mut config);
    config::save_config(path, &config)?;
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

fn parse_template_args(args: &str) -> Option<(&str, &str)> {
    let args = args.trim();
    let split_at = args.find(char::is_whitespace)?;
    let (name, body) = args.split_at(split_at);
    let body = body.trim_start();
    (!name.is_empty() && !body.is_empty()).then_some((name, body))
}

fn parse_template_subcommand_args(args: &str) -> Option<(&str, &str)> {
    let rest = args.trim_start().strip_prefix("replace")?.trim_start();
    parse_template_args(rest)
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
    "usage: #template list | #template show <name> | #template use <name> | #template rm <name> | #template replace <name> <body>"
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

fn prompt_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

fn prompt_host() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_default()
}

fn display_cwd(cwd: &std::path::Path) -> String {
    let Some(home) = prompt_home_dir() else {
        return cwd.display().to_string();
    };
    if cwd == home {
        return "~".to_string();
    }
    if let Ok(rest) = cwd.strip_prefix(&home) {
        if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        }
    } else {
        cwd.display().to_string()
    }
}

fn prompt_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

pub fn save_draft_if_configured(state: &AppState) -> Result<bool> {
    if !state.draft_persist || state.draft.is_empty() {
        return Ok(false);
    }
    let Some(path) = &state.draft_history_path else {
        return Ok(false);
    };

    append_jsonl(
        path,
        &DraftEntry {
            t: (state.clock)(),
            text: state.draft.as_str().to_string(),
        },
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn empty_tab_cycles_modes() {
        let mut state = AppState::default();
        state.handle_empty_tab();
        assert_eq!(state.mode, Mode::History);
        state.handle_empty_tab();
        assert_eq!(state.mode, Mode::Ai);
        state.handle_empty_tab();
        assert_eq!(state.mode, Mode::Draft);
    }

    #[test]
    fn non_empty_tab_does_not_switch_modes() {
        let mut state = AppState::default();
        state.draft.insert_str("git");
        state.handle_empty_tab();
        assert_eq!(state.mode, Mode::Draft);
    }

    #[test]
    fn prompt_line_uses_current_mode_symbol() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");
        assert_eq!(state.render_prompt_line(), "> git status");

        state.mode = Mode::History;
        assert_eq!(state.render_prompt_line(), "$ ");

        state.mode = Mode::Ai;
        assert_eq!(state.render_prompt_line(), "% ");
    }

    #[test]
    fn prompt_line_renders_configured_prompt_variables() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("repo");
        let mut state = AppState {
            current_cwd: Some(cwd.clone()),
            last_status: Some(7),
            prompt_templates: PromptTemplates {
                draft: "[{mode}:{basename}:{last_status}] ".to_string(),
                history: "hist {cwd} {mode} ".to_string(),
                ai: "ai {mode} ".to_string(),
            },
            ..AppState::default()
        };
        state.draft.insert_str("git status");

        assert_eq!(state.render_prompt_line(), "[>:repo:7] git status");

        state.mode = Mode::History;
        assert_eq!(
            state.render_prompt_line(),
            format!("hist {} $ ", cwd.display())
        );

        state.mode = Mode::Ai;
        assert_eq!(state.render_prompt_line(), "ai % ");
    }

    #[test]
    fn prompt_line_abbreviates_home_directory_as_tilde() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let mut state = AppState {
            current_cwd: Some(home.clone()),
            prompt_templates: PromptTemplates {
                draft: "{cwd} > ".to_string(),
                history: "{cwd} $ ".to_string(),
                ai: "{cwd} % ".to_string(),
            },
            ..AppState::default()
        };

        assert_eq!(state.render_prompt_line(), "~ > ");

        state.current_cwd = Some(home.join("repo/project"));
        assert_eq!(state.render_prompt_line(), "~/repo/project > ");
    }

    #[test]
    fn prompt_line_renders_pending_context_confirmation() {
        let state = AppState {
            pending_context: Some(PendingContextPrompt {
                prompt: "explain".to_string(),
                command: "printf context".to_string(),
                dangerous: true,
            }),
            ..AppState::default()
        };

        assert_eq!(
            state.render_prompt_line(),
            "> [dangerous context confirmation: Y/n]"
        );
        assert_eq!(
            state.terminal_cursor_column(),
            state.render_prompt_line().len() as u16
        );
    }

    #[test]
    fn completion_candidates_use_templates_before_history_for_first_token() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        append_template(
            &template_path,
            &TemplateEntry {
                name: "git-save".to_string(),
                body: "git add . && git commit".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            regular_history: vec![HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            }],
            completion_config: CompletionConfig {
                max_results: 2,
                ignore_spaces: true,
                template_first: true,
            },
            ..AppState::default()
        };
        state.draft.insert_str("git");

        let candidates = state.completion_candidates().unwrap();

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].display, "git-save");
        assert_eq!(
            candidates[0].source,
            crate::completion::CompletionSource::Template
        );
        assert_eq!(candidates[1].display, "git status");
        assert_eq!(
            candidates[1].source,
            crate::completion::CompletionSource::History
        );
    }

    #[test]
    fn completion_candidates_use_path_completion_for_path_like_token() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        state.draft.insert_str("cat src/m");

        let candidates = state.completion_candidates().unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].display, "src/main.rs");
        assert_eq!(
            candidates[0].source,
            crate::completion::CompletionSource::Path
        );
    }

    #[test]
    fn completion_candidates_skip_editor_drafts_and_read_only_modes() {
        let mut state = AppState::default();
        state.draft.insert_str("git");
        state.draft_from_editor = true;
        assert!(state.completion_candidates().unwrap().is_empty());

        state.draft_from_editor = false;
        state.mode = Mode::History;
        assert!(state.completion_candidates().unwrap().is_empty());
    }

    #[test]
    fn apply_picker_selection_replaces_current_token_with_quoted_value() {
        let mut state = AppState::default();
        state.draft.insert_str("cat old.txt");
        state.draft.move_left();
        state.draft.move_left();
        state.draft.move_left();

        assert!(state.apply_picker_selection(
            "my file.txt",
            crate::picker::PickerAction::ReplaceCurrentToken
        ));

        assert_eq!(state.draft.as_str(), "cat 'my file.txt'");
        assert_eq!(state.draft.cursor(), "cat 'my file.txt'".len());
    }

    #[test]
    fn apply_picker_selection_skips_editor_and_read_only_modes() {
        let mut state = AppState::default();
        state.draft.insert_str("cat ");
        state.draft_from_editor = true;
        assert!(!state.apply_picker_selection("file", crate::picker::PickerAction::InsertAtCursor));
        assert_eq!(state.draft.as_str(), "cat ");

        state.draft_from_editor = false;
        state.mode = Mode::History;
        assert!(!state.apply_picker_selection("file", crate::picker::PickerAction::InsertAtCursor));
    }

    #[test]
    fn apply_raw_picker_selection_replaces_without_shell_quoting() {
        let mut state = AppState::default();
        state.draft.insert_str("echo OLD");
        state.draft.move_left();
        state.draft.move_left();

        assert!(
            state.apply_raw_picker_selection(
                "$HOME",
                crate::picker::PickerAction::ReplaceCurrentToken
            )
        );

        assert_eq!(state.draft.as_str(), "echo $HOME");
        assert_eq!(state.draft.cursor(), "echo $HOME".len());
    }

    #[test]
    fn history_picker_candidates_follow_current_mode_scope() {
        let regular_history = vec![
            HistoryEntry {
                t: 1,
                command: "one".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "two".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ];
        let ai_sessions = vec![AiSession {
            id: "s1".to_string(),
            t: 3,
            prompt: "prompt".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ai command".to_string(),
                name: None,
            }],
        }];
        let mut state = AppState {
            regular_history,
            ai_sessions,
            ..AppState::default()
        };

        assert_eq!(
            state.history_picker_candidates(),
            vec!["two", "one", "ai command"]
        );
        state.mode = Mode::History;
        assert_eq!(state.history_picker_candidates(), vec!["two", "one"]);
        state.mode = Mode::Ai;
        assert_eq!(state.history_picker_candidates(), vec!["ai command"]);
    }

    #[test]
    fn replace_draft_from_history_picker_copies_raw_command_to_draft() {
        let mut state = AppState {
            mode: Mode::History,
            draft_from_editor: true,
            draft_from_template: true,
            ..AppState::default()
        };

        state.replace_draft_from_history_picker("git commit -m 'hello world'");

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git commit -m 'hello world'");
        assert!(!state.draft_from_editor);
        assert!(!state.draft_from_template);
    }

    #[test]
    fn template_picker_candidates_return_newest_unique_names() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates.jsonl");
        for (name, body) in [("deploy", "old"), ("logs", "tail"), ("deploy", "new")] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };

        assert_eq!(
            state.template_picker_candidates().unwrap(),
            vec!["deploy", "logs"]
        );
    }

    #[test]
    fn replace_draft_from_template_picker_uses_newest_template_body() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates.jsonl");
        for (name, body) in [("deploy", "old"), ("deploy", "rsync {from} {to}")] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let mut state = AppState {
            template_store_path: Some(template_path),
            draft_from_editor: true,
            ..AppState::default()
        };

        assert!(state.replace_draft_from_template_picker("deploy").unwrap());

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "rsync {from} {to}");
        assert!(state.draft_from_template);
        assert!(!state.draft_from_editor);
    }

    #[test]
    fn store_ai_session_from_items_persists_and_selects_first_command() {
        let temp = tempfile::tempdir().unwrap();
        let ai_path = temp.path().join("history/ai.jsonl");
        let mut state = AppState {
            ai_history_path: Some(ai_path.clone()),
            ai_sessions: vec![AiSession {
                id: "old".to_string(),
                t: 1,
                prompt: "old prompt".to_string(),
                ctx: false,
                model: "old-model".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "old command".to_string(),
                    name: None,
                }],
            }],
            clock: || 42,
            ..AppState::default()
        };

        assert!(
            state
                .store_ai_session_from_items(
                    "new prompt",
                    "gpt-test",
                    vec![
                        AiItem {
                            kind: AiItemKind::Template,
                            text: "template body".to_string(),
                            name: Some("tpl".to_string()),
                        },
                        AiItem {
                            kind: AiItemKind::Command,
                            text: "new command".to_string(),
                            name: None,
                        },
                    ],
                )
                .unwrap()
        );

        assert_eq!(state.mode, Mode::Ai);
        assert_eq!(state.selected_ai_index, Some(1));
        assert_eq!(state.selected_ai_command(), Some("new command"));
        assert_eq!(state.ai_sessions.len(), 2);
        assert_eq!(state.ai_sessions[1].prompt, "new prompt");
        assert_eq!(state.ai_sessions[1].model, "gpt-test");
        let loaded = load_jsonl::<AiSession>(&ai_path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].prompt, "new prompt");
    }

    #[test]
    fn store_ai_session_from_items_without_commands_stays_in_draft() {
        let mut state = AppState::default();

        assert!(
            !state
                .store_ai_session_from_items(
                    "prompt",
                    "gpt-test",
                    vec![AiItem {
                        kind: AiItemKind::Template,
                        text: "template body".to_string(),
                        name: Some("tpl".to_string()),
                    }],
                )
                .unwrap()
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.selected_ai_index, None);
        assert!(state.ai_command_indices.is_empty());
        assert_eq!(state.ai_sessions.len(), 1);
    }

    #[test]
    fn ai_prompt_reports_config_error_without_crashing() {
        let mut state = AppState::default();
        state.draft.insert_str("# how do I list files?");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("AI request failed: AI model is not configured"));
        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft.is_empty());
        assert!(state.ai_sessions.is_empty());
    }

    #[test]
    fn terminal_cursor_column_tracks_draft_cursor() {
        let mut state = AppState::default();
        state.draft.insert_str("abc");
        assert_eq!(state.terminal_cursor_column(), 5);

        state.draft.move_left();
        assert_eq!(state.terminal_cursor_column(), 4);

        state.draft.move_start();
        assert_eq!(state.terminal_cursor_column(), 2);
    }

    #[test]
    fn history_mode_selects_and_renders_regular_history_newest_first() {
        let mut state = AppState {
            regular_history: vec![
                HistoryEntry {
                    t: 1,
                    command: "one".to_string(),
                    exit_code: Some(0),
                    source: HistorySource::User,
                },
                HistoryEntry {
                    t: 2,
                    command: "two".to_string(),
                    exit_code: Some(0),
                    source: HistorySource::User,
                },
            ],
            ..AppState::default()
        };

        state.handle_empty_tab();

        assert_eq!(state.mode, Mode::History);
        assert_eq!(state.selected_history_index, Some(0));
        assert_eq!(state.selected_history_command(), Some("two"));
        assert_eq!(state.render_prompt_line(), "$ two");
        assert_eq!(state.terminal_cursor_column(), 5);

        assert!(state.move_history_selection_older());
        assert_eq!(state.selected_history_command(), Some("one"));
        assert!(!state.move_history_selection_older());
        assert!(state.move_history_selection_newer());
        assert_eq!(state.selected_history_command(), Some("two"));
    }

    #[test]
    fn selected_history_copies_to_draft_for_editing() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        assert!(state.copy_selected_history_to_draft());

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(state.draft.cursor(), "git status".len());
    }

    #[test]
    fn ai_mode_selects_and_renders_command_items_in_order() {
        let mut state = AppState {
            ai_sessions: vec![AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "make commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "one".to_string(),
                        name: None,
                    },
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "two".to_string(),
                        name: None,
                    },
                ],
            }],
            ai_command_indices: vec![
                AiCommandIndex {
                    session_index: 0,
                    item_index: 0,
                },
                AiCommandIndex {
                    session_index: 0,
                    item_index: 1,
                },
            ],
            ..AppState::default()
        };

        state.handle_empty_tab();
        state.handle_empty_tab();

        assert_eq!(state.mode, Mode::Ai);
        assert_eq!(state.selected_ai_index, Some(0));
        assert_eq!(state.selected_ai_command(), Some("one"));
        assert_eq!(state.render_prompt_line(), "% one");

        assert!(state.move_ai_selection_next());
        assert_eq!(state.selected_ai_command(), Some("two"));
        assert!(!state.move_ai_selection_next());
        assert!(state.move_ai_selection_previous());
        assert_eq!(state.selected_ai_command(), Some("one"));
    }

    #[test]
    fn selected_ai_copies_to_draft_for_editing() {
        let mut state = AppState {
            mode: Mode::Ai,
            ai_sessions: vec![AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "make commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "git status".to_string(),
                    name: None,
                }],
            }],
            ai_command_indices: vec![AiCommandIndex {
                session_index: 0,
                item_index: 0,
            }],
            selected_ai_index: Some(0),
            ..AppState::default()
        };

        assert!(state.copy_selected_ai_to_draft());

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(state.draft.cursor(), "git status".len());
    }

    #[test]
    fn prepare_editor_session_writes_draft_text() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        let session = state.prepare_editor_session(temp.path()).unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
    }

    #[test]
    fn prepare_editor_session_copies_history_selection_to_draft_and_file() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        let session = state.prepare_editor_session(temp.path()).unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
    }

    #[test]
    fn prepare_editor_session_copies_ai_selection_to_draft_and_file() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            mode: Mode::Ai,
            ai_sessions: vec![AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "status".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "git status".to_string(),
                    name: None,
                }],
            }],
            ai_command_indices: vec![AiCommandIndex {
                session_index: 0,
                item_index: 0,
            }],
            selected_ai_index: Some(0),
            ..AppState::default()
        };

        let session = state.prepare_editor_session(temp.path()).unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
    }

    #[test]
    fn replace_draft_from_editor_session_preserves_editor_content() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState::default();
        state.draft.insert_str("old draft");
        let session = state.prepare_editor_session(temp.path()).unwrap();
        std::fs::write(&session.path, "echo edited\n# filtered\n echo kept").unwrap();

        state.replace_draft_from_editor_session(&session).unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "echo edited\n# filtered\n echo kept");
        assert_eq!(state.draft.cursor(), state.draft.as_str().len());
        assert!(state.draft_from_editor);
        assert_eq!(state.last_status, None);
        assert!(state.regular_history.is_empty());
    }

    #[test]
    fn editor_draft_renders_as_opaque_summary() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState::default();
        let session = state.prepare_editor_session(temp.path()).unwrap();
        std::fs::write(&session.path, "echo one\necho two").unwrap();

        state.replace_draft_from_editor_session(&session).unwrap();

        assert_eq!(
            state.render_prompt_line(),
            "> [editor draft: 2 line(s), 17 byte(s); Ctrl-X Ctrl-E to edit, Enter to run]"
        );
        assert_eq!(
            state.terminal_cursor_column(),
            state.render_prompt_line().len() as u16
        );
    }

    #[test]
    fn replace_draft_from_editor_text_creates_opaque_editor_draft() {
        let mut state = AppState::default();

        state.replace_draft_from_editor_text("echo one\necho two");

        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft_from_editor);
        assert_eq!(state.draft.as_str(), "echo one\necho two");
        assert!(
            state
                .render_prompt_line()
                .contains("[editor draft: 2 line(s)")
        );
    }

    #[test]
    fn run_editor_roundtrip_replaces_draft_after_success() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf 'echo edited' > \"$1\"\n").unwrap();
        make_executable(&script);
        let command = EditorCommand {
            argv: vec![script.display().to_string()],
        };
        let mut state = AppState::default();
        state.draft.insert_str("old draft");

        let result = state.run_editor_roundtrip(temp.path(), &command).unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "echo edited");
        assert_eq!(state.draft.cursor(), "echo edited".len());
        assert!(state.regular_history.is_empty());
    }

    #[test]
    fn run_editor_roundtrip_keeps_original_draft_after_editor_failure() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'should not replace' > \"$1\"\nexit 9\n",
        )
        .unwrap();
        make_executable(&script);
        let command = EditorCommand {
            argv: vec![script.display().to_string()],
        };
        let mut state = AppState::default();
        state.draft.insert_str("old draft");

        let result = state.run_editor_roundtrip(temp.path(), &command).unwrap();

        assert_eq!(result.exit_code, Some(9));
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "old draft");
        assert!(state.regular_history.is_empty());
    }

    #[test]
    fn output_ring_keeps_latest_entries_up_to_capacity() {
        let mut state = AppState::default();

        for index in 0..(OUTPUT_RING_CAPACITY + 1) {
            state.push_output_entry(OutputEntry {
                command: format!("cmd {index}"),
                output: format!("out {index}"),
                exit_code: index as i32,
            });
        }

        assert_eq!(state.output_ring.len(), OUTPUT_RING_CAPACITY);
        assert_eq!(state.output_ring.front().unwrap().command, "cmd 1");
        assert_eq!(
            state.output_ring.back().unwrap().command,
            format!("cmd {OUTPUT_RING_CAPACITY}")
        );
    }

    #[test]
    fn private_exit_requests_app_exit() {
        let mut state = AppState::default();
        state.draft.insert_str("#exit");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(state.exit_requested);
        assert!(state.draft.is_empty());
        assert!(output.is_empty());
    }

    #[test]
    fn private_help_prints_available_commands() {
        let mut state = AppState::default();
        state.draft.insert_str("#help");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("#help"));
        assert!(output.contains("#status"));
        assert!(output.contains("#config"));
        assert!(output.contains("#doctor"));
        assert!(output.contains("#model"));
        assert!(output.contains("#base-url"));
        assert!(output.contains("#env-key"));
        assert!(output.contains("#key"));
        assert!(output.contains("#context"));
        assert!(output.contains("#completion"));
        assert!(output.contains("#log"));
        assert!(output.contains("#editor"));
        assert!(output.contains("#mt"));
        assert!(output.contains("#template"));
        assert!(output.contains("#encrypt"));
        assert!(output.contains("#set-remote"));
        assert!(output.contains("#push"));
        assert!(output.contains("#sync"));
        assert!(output.contains("#exit"));
        assert!(output.contains("#quit"));
        assert!(output.contains("#history"));
        assert!(output.contains("Default keybindings:"));
        assert!(output.contains("Ctrl-C [implemented] - clear or cancel draft"));
        assert!(output.contains("Ctrl-X Ctrl-E [implemented] - external editor"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_context_reports_current_config() {
        let mut state = AppState::default();
        state.draft.insert_str("#context");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("context.enabled=true"));
        assert!(output.contains("context.confirm=true"));
        assert!(output.contains("context.max_bytes=65536"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_context_commands_persist_config() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let mut config = config::Config::default();
        config.storage.home = temp.path().to_path_buf();
        config::save_config(&config_path, &config).unwrap();
        let mut state = AppState {
            config_path: Some(config_path.clone()),
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

        for (line, expected) in [
            ("#context off", "context.enabled=false"),
            ("#context confirm off", "context.confirm=false"),
            ("#context 1024", "context.max_bytes=1024"),
            ("#context on", "context.enabled=true"),
        ] {
            state.draft.insert_str(line);
            let mut output = Vec::new();
            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert!(state.draft.is_empty());
        }

        assert!(state.context_config.enabled);
        assert!(!state.context_config.confirm);
        assert_eq!(state.context_config.max_bytes, 1024);
        let loaded = config::load_config(&config_path).unwrap();
        assert_eq!(loaded.context, state.context_config);
    }

    #[test]
    fn private_context_rejects_invalid_usage_without_persisting() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        config::save_config(&config_path, &config::Config::default()).unwrap();
        let mut state = AppState {
            config_path: Some(config_path.clone()),
            ..AppState::default()
        };
        state.draft.insert_str("#context 0");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("context max bytes must be greater than 0"));
        assert_eq!(state.context_config, ContextConfig::default());
        assert_eq!(
            config::load_config(&config_path).unwrap().context,
            ContextConfig::default()
        );
    }

    #[test]
    fn ai_prompt_with_context_waits_for_confirmation_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let events_path = temp.path().join("logs/events.jsonl");
        let mut state = AppState {
            events_path: Some(events_path.clone()),
            ..AppState::default()
        };
        state.draft.insert_str("# explain < printf context");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("aish will run this command to collect context"));
        assert!(output.contains("Run context command? [Y/n]"));
        assert!(output.contains("answer Y to run context command or n to skip"));
        assert_eq!(
            state.pending_context,
            Some(PendingContextPrompt {
                prompt: "explain".to_string(),
                command: "printf context".to_string(),
                dangerous: false,
            })
        );
        assert!(state.draft.is_empty());
        assert!(state.ai_sessions.is_empty());
        let events = load_events(&events_path).unwrap();
        assert_eq!(events.items[0].msg, "context command requires confirmation");
    }

    #[test]
    fn ai_prompt_with_context_disabled_does_not_execute_command() {
        let mut state = AppState {
            context_config: ContextConfig {
                enabled: false,
                ..ContextConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("# explain < printf context");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("context collection is disabled"));
        assert!(output.contains("context command not executed: printf context"));
        assert!(state.draft.is_empty());
        assert!(state.ai_sessions.is_empty());
    }

    #[test]
    fn ai_prompt_with_context_blocks_dangerous_command_even_without_confirmation() {
        let mut state = AppState {
            context_config: ContextConfig {
                confirm: false,
                ..ContextConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("# explain < rm -rf /tmp/aish-test");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("dangerous context command requires confirmation"));
        assert_eq!(
            state.pending_context,
            Some(PendingContextPrompt {
                prompt: "explain".to_string(),
                command: "rm -rf /tmp/aish-test".to_string(),
                dangerous: true,
            })
        );
        assert!(state.draft.is_empty());
        assert!(state.ai_sessions.is_empty());
    }

    #[test]
    fn answer_context_confirmation_can_skip_pending_command() {
        let temp = tempfile::tempdir().unwrap();
        let events_path = temp.path().join("logs/events.jsonl");
        let mut state = AppState {
            events_path: Some(events_path.clone()),
            pending_context: Some(PendingContextPrompt {
                prompt: "explain".to_string(),
                command: "printf context".to_string(),
                dangerous: false,
            }),
            ..AppState::default()
        };
        let mut output = Vec::new();

        answer_context_confirmation(&mut state, false, &mut output, Duration::from_secs(5))
            .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("context command skipped: printf context"));
        assert_eq!(state.pending_context, None);
        assert!(state.ai_sessions.is_empty());
        let events = load_events(&events_path).unwrap();
        assert_eq!(events.items[0].msg, "context command skipped");
    }

    #[test]
    fn private_log_prints_recent_events() {
        let temp = tempfile::tempdir().unwrap();
        let events_path = temp.path().join("logs/events.jsonl");
        append_event(&events_path, 1, EventLevel::Info, "one", DEFAULT_MAX_EVENTS).unwrap();
        append_event(&events_path, 2, EventLevel::Warn, "two", DEFAULT_MAX_EVENTS).unwrap();
        let mut state = AppState {
            events_path: Some(events_path),
            ..AppState::default()
        };
        state.draft.insert_str("#log 1");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("one"));
        assert!(output.contains("2\tWarn\ttwo"));
    }

    #[test]
    fn private_log_reports_usage_or_missing_storage() {
        for (line, expected) in [
            ("#log", "usage: #log <count>"),
            ("#log nope", "usage: #log <count>"),
            ("#log 1", "event log storage is not configured"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
        }
    }

    #[test]
    fn ai_config_commands_persist_and_report_values() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let mut config = config::Config::default();
        config.storage.home = temp.path().to_path_buf();
        config::save_config(&config_path, &config).unwrap();
        let mut state = AppState {
            config_path: Some(config_path.clone()),
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

        for (line, expected) in [
            ("#model test-model", "#model=test-model"),
            (
                "#base-url https://example.invalid/v1",
                "#base-url=https://example.invalid/v1/chat/completions",
            ),
            ("#env-key OPENAI_API_KEY", "#env-key=OPENAI_API_KEY"),
        ] {
            state.draft.insert_str(line);
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert!(state.draft.is_empty());
        }

        assert_eq!(state.ai_config.model, "test-model");
        assert_eq!(
            state.ai_config.base_url,
            "https://example.invalid/v1/chat/completions"
        );
        assert_eq!(state.ai_config.env_key, "OPENAI_API_KEY");
        let loaded = config::load_config(&config_path).unwrap();
        assert_eq!(loaded.ai, state.ai_config);
    }

    #[test]
    fn ai_config_commands_report_unconfigured_without_config_path() {
        for (line, expected) in [
            ("#model", "#model=unconfigured"),
            ("#base-url", "#base-url=unconfigured"),
            ("#env-key", "#env-key=unconfigured"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert!(state.draft.is_empty());
        }
    }

    #[test]
    fn key_commands_report_placeholders_without_secret_side_effects() {
        for (line, expected) in [
            ("#key set", "#key set is not implemented yet; no key stored"),
            (
                "#key clear",
                "#key clear is not implemented yet; no key removed",
            ),
            ("#key", "usage: #key set | #key clear"),
            ("#key rotate", "usage: #key set | #key clear"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert_eq!(state.last_status, None);
            assert!(state.draft.is_empty());
        }
    }

    #[test]
    fn subsystem_commands_report_placeholders() {
        for (line, expected) in [
            ("#completion", "completion engine is not implemented yet"),
            ("#editor", "editor temp directory is not configured"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert_eq!(state.last_status, None);
            assert!(state.draft.is_empty());
        }
    }

    #[test]
    fn mt_command_persists_template_entry() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        let mut state = AppState {
            template_store_path: Some(template_path.clone()),
            ..AppState::default()
        };
        state.draft.insert_str("#mt deploy rsync {from} {to}");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template stored: deploy"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());

        let loaded = load_templates(&template_path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].name, "deploy");
        assert_eq!(loaded.items[0].body, "rsync {from} {to}");
    }

    #[test]
    fn template_list_prints_stored_template_names() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        append_template(
            &template_path,
            &TemplateEntry {
                name: "deploy".to_string(),
                body: "rsync {from} {to}".to_string(),
            },
        )
        .unwrap();
        append_template(
            &template_path,
            &TemplateEntry {
                name: "logs".to_string(),
                body: "tail -f {file}".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("#template list");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("deploy"));
        assert!(output.contains("logs"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn template_rm_removes_matching_templates() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [
            ("deploy", "rsync {from} {to}"),
            ("logs", "tail -f {file}"),
            ("deploy", "kubectl apply -f {file}"),
        ] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let mut state = AppState {
            template_store_path: Some(template_path.clone()),
            ..AppState::default()
        };
        state.draft.insert_str("#template rm deploy");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template removed: deploy (2)"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());

        let loaded = load_templates(&template_path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].name, "logs");
    }

    #[test]
    fn template_replace_rewrites_matching_templates() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [
            ("deploy", "old deploy"),
            ("logs", "tail -f {file}"),
            ("deploy", "older deploy"),
        ] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let mut state = AppState {
            template_store_path: Some(template_path.clone()),
            ..AppState::default()
        };
        state
            .draft
            .insert_str("#template replace deploy new deploy body");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template replaced: deploy (removed 2)"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());

        let loaded = load_templates(&template_path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].name, "logs");
        assert_eq!(loaded.items[1].name, "deploy");
        assert_eq!(loaded.items[1].body, "new deploy body");
    }

    #[test]
    fn template_use_copies_newest_matching_body_to_draft() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [
            ("deploy", "old deploy"),
            ("logs", "tail -f {file}"),
            ("deploy", "rsync {from} {user}@{host}:{to} {from}"),
        ] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str(
            "#template use deploy from=src host=prod to=/srv/app zextra=ignored aextra=unused",
        );
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template copied to draft: deploy"));
        assert!(output.contains("template placeholders: from, user, host, to"));
        assert!(output.contains("unresolved template placeholders: user"));
        assert!(output.contains("unused template values: aextra, zextra"));
        assert_eq!(state.last_status, None);
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "rsync src {user}@prod:/srv/app src");
        assert_eq!(state.draft.cursor(), state.draft.as_str().len());
    }

    #[test]
    fn template_use_reports_missing_template_without_changing_draft() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("#template use missing");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template not found: missing"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn template_use_supports_quoted_values_with_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        append_template(
            &template_path,
            &TemplateEntry {
                name: "deploy".to_string(),
                body: "echo {message} && cd {path}".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state
            .draft
            .insert_str("#template use deploy message=\"hello world\" path='/tmp/my dir'");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template copied to draft: deploy"));
        assert_eq!(state.last_status, None);
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "echo hello world && cd /tmp/my dir");
    }

    #[test]
    fn template_use_supports_described_and_variadic_placeholders() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        append_template(
            &template_path,
            &TemplateEntry {
                name: "commit".to_string(),
                body: "git commit -m {message:commit message} -- {paths...}".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state
            .draft
            .insert_str("#template use commit message='ship it' paths='src tests'");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template placeholders: message, paths"));
        assert_eq!(state.last_status, None);
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git commit -m ship it -- src tests");
        assert!(state.draft_from_template);
    }

    #[test]
    fn unresolved_template_placeholders_do_not_execute() {
        let mut state = AppState {
            draft_from_template: true,
            ..AppState::default()
        };
        state.draft.insert_str("echo {message}");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("cannot execute unresolved template placeholders: message"));
        assert_eq!(state.last_status, None);
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "echo {message}");
    }

    #[test]
    fn template_show_prints_newest_matching_body() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [
            ("deploy", "old deploy"),
            ("logs", "tail -f {file}"),
            ("deploy", "new deploy"),
        ] {
            append_template(
                &template_path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("#template show deploy");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template: deploy"));
        assert!(output.contains("new deploy"));
        assert!(!output.contains("old deploy"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn template_commands_report_usage_for_invalid_input() {
        let usage = template_usage();
        for (line, expected) in [
            ("#mt deploy", "usage: #mt <name> <body>"),
            ("#template rm", usage),
            ("#template replace deploy", usage),
            ("#template show", usage),
            ("#template use", usage),
            ("#template", usage),
            ("#template unknown deploy", usage),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert_eq!(state.last_status, None);
            assert!(state.draft.is_empty());
        }
    }

    #[test]
    fn encryption_and_sync_commands_report_placeholders_without_side_effects() {
        for (line, expected) in [
            (
                "#encrypt on",
                "encryption is not implemented yet; no data changed",
            ),
            (
                "#set-remote git@example.invalid:aish.git",
                "sync remote configuration is not implemented yet; no remote changed",
            ),
            (
                "#push",
                "sync push is not implemented yet; no git command run",
            ),
            ("#sync", "sync is not implemented yet; no git command run"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str(line);
            let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
            let mut output = Vec::new();

            execute_draft(
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();

            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
            assert_eq!(state.last_status, None);
            assert!(state.draft.is_empty());
        }
    }

    #[test]
    fn private_config_prints_read_only_runtime_config() {
        let temp = tempfile::tempdir().unwrap();
        let history_path = temp.path().join("history/regular.jsonl");
        let notes_path = temp.path().join("history/notes.jsonl");
        let draft_path = temp.path().join("history/draft.jsonl");
        let template_path = temp.path().join("templates/templates.jsonl");
        let mut state = AppState {
            regular_history_path: Some(history_path.clone()),
            notes_path: Some(notes_path.clone()),
            draft_history_path: Some(draft_path.clone()),
            template_store_path: Some(template_path.clone()),
            draft_persist: false,
            editor_config: EditorConfig {
                command: vec!["nvim".to_string(), "--clean".to_string()],
                execute_after_save: false,
            },
            completion_config: CompletionConfig {
                max_results: 8,
                ignore_spaces: false,
                template_first: true,
            },
            ai_config: AiConfig {
                model: "gpt-test".to_string(),
                base_url: "https://example.invalid/v1".to_string(),
                env_key: "OPENAI_API_KEY".to_string(),
            },
            context_config: ContextConfig {
                enabled: false,
                confirm: false,
                max_bytes: 1024,
            },
            ..AppState::default()
        };
        state.draft.insert_str("#config");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Aish config"));
        assert!(output.contains("draft.persist=false"));
        assert!(output.contains("editor.execute_after_save=false"));
        assert!(output.contains("editor.command=nvim --clean"));
        assert!(output.contains("paste.multiline=editor"));
        assert!(output.contains("paste.confirm_execute=true"));
        assert!(output.contains("completion.max_results=8"));
        assert!(output.contains("completion.ignore_spaces=false"));
        assert!(output.contains("completion.template_first=true"));
        assert!(output.contains("ai.model=gpt-test"));
        assert!(output.contains("ai.base_url=https://example.invalid/v1"));
        assert!(output.contains("ai.env_key=OPENAI_API_KEY"));
        assert!(output.contains("context.enabled=false"));
        assert!(output.contains("context.confirm=false"));
        assert!(output.contains("context.max_bytes=1024"));
        assert!(output.contains("editor.resolved=nvim --clean"));
        assert!(output.contains("history.regular="));
        assert!(output.contains(&history_path.display().to_string()));
        assert!(output.contains("history.notes="));
        assert!(output.contains(&notes_path.display().to_string()));
        assert!(output.contains("history.draft="));
        assert!(output.contains(&draft_path.display().to_string()));
        assert!(output.contains("templates.store="));
        assert!(output.contains(&template_path.display().to_string()));
        assert!(!history_path.exists());
        assert!(!notes_path.exists());
        assert!(!draft_path.exists());
        assert!(!template_path.exists());
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_doctor_prints_read_only_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let history_path = temp.path().join("history/regular.jsonl");
        let notes_path = temp.path().join("history/notes.jsonl");
        let draft_path = temp.path().join("history/draft.jsonl");
        let mut state = AppState {
            last_status: Some(7),
            current_cwd: Some(temp.path().to_path_buf()),
            editor_config: EditorConfig {
                command: vec!["vim".to_string()],
                execute_after_save: false,
            },
            regular_history_path: Some(history_path.clone()),
            notes_path: Some(notes_path.clone()),
            draft_history_path: Some(draft_path.clone()),
            regular_history: vec![HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            }],
            ai_sessions: vec![AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "ls".to_string(),
                    name: None,
                }],
            }],
            ai_command_indices: vec![AiCommandIndex {
                session_index: 0,
                item_index: 0,
            }],
            ..AppState::default()
        };
        state.draft.insert_str("#doctor");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Aish doctor"));
        assert!(output.contains("mode=>"));
        assert!(output.contains("last_status=7"));
        assert!(output.contains(&format!("cwd={}", temp.path().display())));
        assert!(output.contains("draft_persist=true"));
        assert!(output.contains("regular_history_entries=1"));
        assert!(output.contains("ai_sessions=1"));
        assert!(output.contains("ai_commands=1"));
        assert!(output.contains("output_ring_entries=0"));
        assert!(output.contains("editor.resolved=vim"));
        assert!(output.contains("regular_history_path="));
        assert!(output.contains("exists=false"));
        assert!(!history_path.exists());
        assert!(!notes_path.exists());
        assert!(!draft_path.exists());
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_editor_reports_resolution_without_launching_editor() {
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec!["code".to_string(), "--wait".to_string()],
                execute_after_save: false,
            },
            editor_temp_root: Some(std::env::temp_dir().join("aish-editor-test")),
            ..AppState::default()
        };
        state.draft.insert_str("#editor");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Aish editor"));
        assert!(output.contains("configured=code --wait"));
        assert!(output.contains("editor.resolved=code --wait"));
        assert!(output.contains("external editor launch is wired to Ctrl-X Ctrl-E"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn unknown_private_command_prints_suggestion() {
        let mut state = AppState::default();
        state.draft.insert_str("#statsu");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Aish command not implemented yet: #statsu"));
        assert!(output.contains("Did you mean #status?"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_status_prints_mode_and_last_status() {
        let mut state = AppState {
            last_status: Some(7),
            current_cwd: Some(std::env::temp_dir()),
            ..AppState::default()
        };
        state.draft.insert_str("#status");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("mode=>"));
        assert!(output.contains("last_status=7"));
        assert!(output.contains(&format!("cwd={}", std::env::temp_dir().display())));
        assert!(output.contains("keybindings=20"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_history_without_count_prints_usage() {
        let mut state = AppState::default();
        state.draft.insert_str("#history nope");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("usage: #history <count>")
        );
        assert!(state.draft.is_empty());
    }

    #[test]
    fn unix_timestamp_returns_non_negative_seconds() {
        assert!(unix_timestamp() >= 0);
    }

    fn fixed_clock() -> i64 {
        42
    }

    #[test]
    fn save_draft_if_configured_persists_non_empty_draft() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("draft.jsonl");
        let mut state = AppState {
            draft_history_path: Some(path.clone()),
            clock: fixed_clock,
            ..AppState::default()
        };
        state.draft.insert_str("git status");

        assert!(save_draft_if_configured(&state).unwrap());

        let loaded = crate::history::load_jsonl::<DraftEntry>(&path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].t, 42);
        assert_eq!(loaded.items[0].text, "git status");
    }

    #[test]
    fn save_draft_if_configured_skips_empty_or_disabled_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("draft.jsonl");
        let mut state = AppState {
            draft_history_path: Some(path.clone()),
            draft_persist: false,
            ..AppState::default()
        };
        state.draft.insert_str("git status");

        assert!(!save_draft_if_configured(&state).unwrap());
        assert!(!path.exists());

        let state = AppState {
            draft_history_path: Some(path.clone()),
            ..AppState::default()
        };
        assert!(!save_draft_if_configured(&state).unwrap());
        assert!(!path.exists());
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }
}
