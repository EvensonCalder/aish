use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::commands::{ParsedLine, parse_line};
use crate::history::DraftEntry;

mod bootstrap;
mod completion_runtime;
mod config_commands;
mod context_prompt;
mod editor_state;
mod encryption_commands;
mod event_log;
mod execution;
mod help;
mod history_ops;
mod private_commands;
mod private_output;
mod prompt;
mod prompt_command;
mod reports;
mod selection_state;
pub mod startup_unlock;
mod sync_commands;
mod template_args;
mod template_remotes;

use config_commands::{
    update_ai_config_field, update_completion_config, update_context_config, update_paste_config,
};
use encryption_commands::{
    ai_config_for_request, clear_stored_key, configured_encryption_key, parse_key_command,
    set_stored_key, update_encryption_config,
};
pub use prompt::PromptTemplates;
use reports::{write_config_report, write_doctor_report, write_editor_report, write_status_report};
pub(crate) use sync_commands::{
    drain_background_sync_events, queue_due_periodic_sync_if_needed, run_exit_sync_if_enabled,
    wait_for_background_sync_on_exit,
};
use sync_commands::{set_sync_remote, set_sync_schedule};

mod state;

pub use state::{
    AppState, InlineCompletion, OutputEntry, PendingCompletion, PendingCompletionUpdate,
    PendingContextPrompt,
};

pub use bootstrap::run;
pub use context_prompt::answer_context_confirmation;
use event_log::show_event_log;
pub use execution::execute_draft;
#[cfg(test)]
pub(crate) use execution::{foreground_shell_args, record_completed_command, write_command_output};
use history_ops::{load_ai_sessions_for_state, trim_history_for_state};
pub use private_output::{
    PendingPrivateOutput, PrivateOutputSink, answer_private_output_confirmation,
};
use template_args::{
    parse_template_body, parse_template_find_query, parse_template_subcommand_args,
    parse_template_values, template_usage,
};
use template_remotes::template_remote_command;

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

pub fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

pub fn save_draft_if_configured(state: &mut AppState) -> Result<bool> {
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
