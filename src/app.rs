use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::commands::{
    IMPLEMENTED_PRIVATE_COMMANDS, ParsedLine, parse_line, suggest_private_command,
};
use crate::config::{self, EditorConfig, PromptConfig};
use crate::editor::resolve_editor_command;
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, HistorySource,
    HistoryStore, NoteEntry, ai_command_indices, append_jsonl, load_jsonl, trim_combined_history,
};
use crate::input::InputBuffer;
use crate::keybindings::default_keybindings;
use crate::modes::Mode;
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
    pub template_store_path: Option<PathBuf>,
    pub draft_persist: bool,
    pub regular_history: Vec<HistoryEntry>,
    pub selected_history_index: Option<usize>,
    pub ai_sessions: Vec<AiSession>,
    pub ai_command_indices: Vec<AiCommandIndex>,
    pub selected_ai_index: Option<usize>,
    pub output_ring: VecDeque<OutputEntry>,
    pub prompt_templates: PromptTemplates,
    pub editor_config: EditorConfig,
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
            template_store_path: None,
            draft_persist: true,
            regular_history: Vec::new(),
            selected_history_index: None,
            ai_sessions: Vec::new(),
            ai_command_indices: Vec::new(),
            selected_ai_index: None,
            output_ring: VecDeque::new(),
            prompt_templates: PromptTemplates::default(),
            editor_config: EditorConfig::default(),
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
            .map(|cwd| cwd.display().to_string())
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
        let text = match self.mode {
            Mode::History => self.selected_history_command().unwrap_or(""),
            Mode::Ai => self.selected_ai_command().unwrap_or(""),
            _ => self.draft.as_str(),
        };
        format!("{}{}", self.prompt_prefix(), text)
    }

    pub fn terminal_cursor_column(&self) -> u16 {
        let cursor = match self.mode {
            Mode::History => self.selected_history_command().unwrap_or("").len(),
            Mode::Ai => self.selected_ai_command().unwrap_or("").len(),
            _ => self.draft.cursor(),
        };
        let column = self.prompt_prefix().len() + cursor;
        column.min(u16::MAX as usize) as u16
    }

    fn push_output_entry(&mut self, entry: OutputEntry) {
        if self.output_ring.len() == OUTPUT_RING_CAPACITY {
            self.output_ring.pop_front();
        }
        self.output_ring.push_back(entry);
    }
}

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::default_aish_dir())?;
    let store = HistoryStore::load(&layout)?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        regular_history_path: Some(layout.regular_history),
        ai_history_path: Some(layout.ai_history),
        notes_path: Some(layout.notes),
        draft_history_path: Some(layout.draft_history),
        template_store_path: Some(layout.template_store),
        draft_persist: config.draft.persist,
        regular_history: store.regular,
        ai_sessions: store.ai_sessions,
        ai_command_indices: store.ai_command_indices,
        prompt_templates: config.prompt.into(),
        editor_config: config.editor,
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
                    write_ai_config_placeholder(out, "model", args)?;
                    state.draft.clear();
                    state.mode = Mode::Draft;
                    return Ok(());
                }
                "base-url" => {
                    write_ai_config_placeholder(out, "base-url", args)?;
                    state.draft.clear();
                    state.mode = Mode::Draft;
                    return Ok(());
                }
                "env-key" => {
                    write_ai_config_placeholder(out, "env-key", args)?;
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
                    writeln!(
                        out,
                        "context.enabled=false context.confirm=true context.max_bytes=0"
                    )?;
                    writeln!(out, "context collection is not implemented yet")?;
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
                    writeln!(out, "event log is not implemented yet")?;
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
        ParsedLine::AiPrompt(_) => {
            writeln!(out, "AI prompts are not implemented yet")?;
            state.draft.clear();
            state.mode = Mode::Draft;
            return Ok(());
        }
        ParsedLine::AiPromptWithContext { prompt, command } => {
            writeln!(
                out,
                "AI prompts with context are not implemented yet; context command not executed: {command}"
            )?;
            writeln!(out, "prompt: {prompt}")?;
            state.draft.clear();
            state.mode = Mode::Draft;
            return Ok(());
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
    if executing_ai && result.exit_code == 0 {
        state.advance_after_ai_success();
    } else if executing_ai {
        state.mode = Mode::Ai;
    } else {
        state.mode = Mode::Draft;
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
    write_editor_resolution(out, state)?;
    write_config_path(out, "history.regular", &state.regular_history_path)?;
    write_config_path(out, "history.notes", &state.notes_path)?;
    write_config_path(out, "history.draft", &state.draft_history_path)?;
    write_config_path(out, "templates.store", &state.template_store_path)?;
    Ok(())
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
    writeln!(out, "external editor launch is not implemented yet")?;
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

fn write_ai_config_placeholder(out: &mut impl Write, name: &str, args: &str) -> Result<()> {
    if args.trim().is_empty() {
        writeln!(out, "#{name} is not configured yet")?;
    } else {
        writeln!(out, "#{name} persistence is not implemented yet")?;
    }
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
        assert!(output.contains("Ctrl-X Ctrl-E [implemented] - external editor placeholder"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn private_context_reports_disabled_placeholder() {
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
        assert!(output.contains("context.enabled=false"));
        assert!(output.contains("context.confirm=true"));
        assert!(output.contains("context collection is not implemented yet"));
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }

    #[test]
    fn ai_config_commands_report_placeholders_without_persisting() {
        for (line, expected) in [
            ("#model", "#model is not configured yet"),
            (
                "#model test-model",
                "#model persistence is not implemented yet",
            ),
            ("#base-url", "#base-url is not configured yet"),
            (
                "#base-url https://example.invalid/v1",
                "#base-url persistence is not implemented yet",
            ),
            ("#env-key", "#env-key is not configured yet"),
            (
                "#env-key OPENAI_API_KEY",
                "#env-key persistence is not implemented yet",
            ),
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
            ("#log", "event log is not implemented yet"),
            ("#editor", "external editor launch is not implemented yet"),
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
        assert!(output.contains("external editor launch is not implemented yet"));
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
}
