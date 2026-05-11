use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::commands::{ParsedLine, parse_line};
use crate::config;
use crate::history::{HistoryEntry, HistorySource, append_jsonl};
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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Draft,
            draft: InputBuffer::new(),
            last_status: None,
            exit_requested: false,
            regular_history_path: None,
        }
    }
}

impl AppState {
    pub fn handle_empty_tab(&mut self) {
        if self.draft.is_empty() {
            self.mode = self.mode.next_primary();
        }
    }

    pub fn prompt_prefix(&self) -> String {
        format!("{} ", self.mode.symbol())
    }

    pub fn render_prompt_line(&self) -> String {
        format!("{}{}", self.prompt_prefix(), self.draft.as_str())
    }

    pub fn terminal_cursor_column(&self) -> u16 {
        let column = self.prompt_prefix().len() + self.draft.cursor();
        column.min(u16::MAX as usize) as u16
    }
}

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::default_aish_dir())?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        regular_history_path: Some(layout.regular_history),
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
        ParsedLine::Note { .. } => {
            writeln!(out, "note stored")?;
            state.draft.clear();
            state.mode = Mode::Draft;
            return Ok(());
        }
        ParsedLine::Private { name, .. } => {
            match name {
                "exit" | "quit" => {
                    state.exit_requested = true;
                    state.draft.clear();
                    state.mode = Mode::Draft;
                    return Ok(());
                }
                "help" => {
                    writeln!(out, "Aish private commands: #help, #status, #exit")?;
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
                _ => {}
            }
            writeln!(out, "Aish command not implemented yet: #{name}")?;
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
    }

    state.mode = Mode::CommandRunning;
    let result = backend.run_command(&command, timeout)?;
    if !result.output.is_empty() {
        writeln!(out, "{}", result.output)?;
    }
    if let Some(path) = &state.regular_history_path {
        append_jsonl(
            path,
            &HistoryEntry {
                command: result.command.clone(),
                exit_code: Some(result.exit_code),
                source: HistorySource::User,
            },
        )?;
    }
    state.last_status = Some(result.exit_code);
    state.draft.clear();
    state.mode = Mode::Draft;
    Ok(())
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
        assert_eq!(state.render_prompt_line(), "$ git status");

        state.mode = Mode::Ai;
        assert_eq!(state.render_prompt_line(), "% git status");
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
        assert!(output.contains("#exit"));
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
}
