use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::commands::{
    IMPLEMENTED_PRIVATE_COMMANDS, ParsedLine, parse_line, suggest_private_command,
};
use crate::config;
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, DraftEntry, HistoryEntry, HistorySource,
    HistoryStore, NoteEntry, append_jsonl, trim_regular_history,
};
use crate::input::InputBuffer;
use crate::modes::Mode;
use crate::pty::PtyBackend;

#[derive(Debug)]
pub struct AppState {
    pub mode: Mode,
    pub draft: InputBuffer,
    pub last_status: Option<i32>,
    pub exit_requested: bool,
    pub regular_history_path: Option<PathBuf>,
    pub notes_path: Option<PathBuf>,
    pub draft_history_path: Option<PathBuf>,
    pub draft_persist: bool,
    pub regular_history: Vec<HistoryEntry>,
    pub selected_history_index: Option<usize>,
    pub ai_sessions: Vec<AiSession>,
    pub ai_command_indices: Vec<AiCommandIndex>,
    pub selected_ai_index: Option<usize>,
    pub clock: fn() -> i64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Draft,
            draft: InputBuffer::new(),
            last_status: None,
            exit_requested: false,
            regular_history_path: None,
            notes_path: None,
            draft_history_path: None,
            draft_persist: true,
            regular_history: Vec::new(),
            selected_history_index: None,
            ai_sessions: Vec::new(),
            ai_command_indices: Vec::new(),
            selected_ai_index: None,
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
        format!("{} ", self.mode.symbol())
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
}

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::default_aish_dir())?;
    let store = HistoryStore::load(&layout)?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        regular_history_path: Some(layout.regular_history),
        notes_path: Some(layout.notes),
        draft_history_path: Some(layout.draft_history),
        draft_persist: config.draft.persist,
        regular_history: store.regular,
        ai_sessions: store.ai_sessions,
        ai_command_indices: store.ai_command_indices,
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
                    state.draft.clear();
                    state.mode = Mode::Draft;
                    return Ok(());
                }
                "status" => {
                    writeln!(
                        out,
                        "mode={} last_status={}",
                        state.mode.symbol(),
                        state
                            .last_status
                            .map(|status| status.to_string())
                            .unwrap_or_else(|| "none".to_string())
                    )?;
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
                "history" => {
                    let count = args.parse::<usize>();
                    match (count, &state.regular_history_path) {
                        (Ok(count), Some(path)) => {
                            let loaded = trim_regular_history(path, count)?;
                            let keep_from = loaded.items.len().saturating_sub(count);
                            state.regular_history = loaded.items[keep_from..].to_vec();
                            state.selected_history_index = None;
                            writeln!(
                                out,
                                "history trimmed to {count}; skipped {} bad line(s)",
                                loaded.errors.len()
                            )?;
                        }
                        (Ok(_), None) => writeln!(out, "history storage is not configured")?,
                        (Err(_), _) => writeln!(out, "usage: #history <count>")?,
                    }
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
    writeln!(out, "draft_persist={}", state.draft_persist)?;
    writeln!(
        out,
        "regular_history_entries={}",
        state.regular_history.len()
    )?;
    writeln!(out, "ai_sessions={}", state.ai_sessions.len())?;
    writeln!(out, "ai_commands={}", state.ai_command_indices.len())?;
    write_path_status(out, "regular_history_path", &state.regular_history_path)?;
    write_path_status(out, "notes_path", &state.notes_path)?;
    write_path_status(out, "draft_history_path", &state.draft_history_path)?;
    Ok(())
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
        assert!(output.contains("#doctor"));
        assert!(output.contains("#exit"));
        assert!(output.contains("#quit"));
        assert!(output.contains("#history"));
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
        assert!(output.contains("draft_persist=true"));
        assert!(output.contains("regular_history_entries=1"));
        assert!(output.contains("ai_sessions=1"));
        assert!(output.contains("ai_commands=1"));
        assert!(output.contains("regular_history_path="));
        assert!(output.contains("exists=false"));
        assert!(!history_path.exists());
        assert!(!notes_path.exists());
        assert!(!draft_path.exists());
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
