use std::io::Write;
use std::time::Duration;

use anyhow::Result;

use crate::context::{
    build_contextual_ai_prompt, is_dangerous_context_command, run_context_command,
};
use crate::log::EventLevel;
use crate::modes::Mode;

use super::{AppState, PendingContextPrompt, ai_config_for_request};

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

pub(super) fn submit_ai_prompt(
    state: &mut AppState,
    prompt: &str,
    out: &mut impl Write,
) -> Result<()> {
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

pub(super) fn submit_ai_prompt_with_context(
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
