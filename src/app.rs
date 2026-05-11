use std::io::{self, Write};
use std::time::Duration;

use anyhow::Result;

use crate::commands::{ParsedLine, parse_line};
use crate::config;
use crate::input::InputBuffer;
use crate::modes::Mode;
use crate::pty::PtyBackend;

#[derive(Debug)]
pub struct AppState {
    pub mode: Mode,
    pub draft: InputBuffer,
    pub last_status: Option<i32>,
    pub exit_requested: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Draft,
            draft: InputBuffer::new(),
            last_status: None,
            exit_requested: false,
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
}

pub fn run() -> Result<()> {
    let (_layout, config) = config::init_default_layout(config::default_aish_dir())?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState::default();
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
            if matches!(name, "exit" | "quit") {
                state.exit_requested = true;
                state.draft.clear();
                state.mode = Mode::Draft;
                return Ok(());
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
}
