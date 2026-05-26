use std::io::Write;

use anyhow::Result;

use crate::commands::suggest_private_command;
use crate::history::{AiItemKind, ai_command_indices};
use crate::input::InputBuffer;
use crate::modes::Mode;
use crate::templates::{TemplateEntry, apply_template_values_with_usage, template_placeholders};

use super::private_output::{
    list_output_from_commands, parse_list_output_sink, write_or_confirm_private_output,
};
use super::{
    AppState, clear_stored_key, help, load_ai_sessions_for_state, parse_key_command,
    parse_template_body, parse_template_find_query, parse_template_subcommand_args,
    parse_template_values, prompt_command, set_stored_key, set_sync_remote, set_sync_schedule,
    show_event_log, template_remote_command, template_usage, trim_history_for_state,
    update_ai_config_field, update_completion_config, update_context_config,
    update_encryption_config, update_paste_config, write_config_report, write_doctor_report,
    write_editor_report, write_status_report,
};

pub(super) fn execute_private_command(
    state: &mut AppState,
    out: &mut impl Write,
    name: &str,
    args: &str,
) -> Result<()> {
    match name {
        "exit" | "quit" => {
            state.exit_requested = true;
        }
        "help" => help::write_help(out, args, &state.keybinding_config)?,
        "status" => write_status_report(state, out)?,
        "config" => write_config_report(state, out)?,
        "doctor" => write_doctor_report(state, out)?,
        "prompt" => prompt_command::update_prompt_config(state, out, args)?,
        "model" => update_ai_config_field(state, out, "model", args)?,
        "base-url" => update_ai_config_field(state, out, "base-url", args)?,
        "env-key" => update_ai_config_field(state, out, "env-key", args)?,
        "key" => match parse_key_command(args) {
            Some("set") => set_stored_key(state, out)?,
            Some("clear") => clear_stored_key(state, out)?,
            _ => writeln!(out, "usage: #key set | #key clear")?,
        },
        "unlock" => unlock_encrypted_storage_command(state, out)?,
        "context" => update_context_config(state, out, args)?,
        "paste" => update_paste_config(state, out, args)?,
        "completion" => update_completion_config(state, out, args)?,
        "log" => show_event_log(state, out, args)?,
        "editor" => write_editor_report(state, out)?,
        "history" => history_command(state, out, args)?,
        "ai" => ai_command(state, out, args)?,
        "draft" => draft_command(state, out, args)?,
        "mt" => create_template_command(state, out, args)?,
        "template" => {
            let keep_draft = template_command(state, out, args)?;
            if keep_draft {
                state.selected_draft_index = None;
                state.mode = Mode::Draft;
                return Ok(());
            }
        }
        "encrypt" => update_encryption_config(state, out, args)?,
        "set-remote" => set_sync_remote(state, out, args)?,
        "sync" => set_sync_schedule(state, out, args)?,
        _ => match suggest_private_command(name) {
            Some(suggestion) => writeln!(
                out,
                "unknown Aish command: #{name}. Did you mean #{suggestion}?"
            )?,
            None => writeln!(out, "unknown Aish command: #{name}")?,
        },
    }
    state.clear_draft_for_new_draft();
    Ok(())
}

fn unlock_encrypted_storage_command(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    if !state.encryption_config.enabled {
        writeln!(out, "encryption is off")?;
        return Ok(());
    }
    if state.encrypted_storage_unlocked {
        writeln!(out, "encrypted storage already unlocked")?;
        return Ok(());
    }
    state.unlock_encrypted_storage_interactively()?;
    writeln!(out, "encrypted storage unlocked")?;
    Ok(())
}

const HISTORY_USAGE: &str =
    "usage: #history search <query> | #history list [>|>> <path> | | <command>] | #history <count>";
const AI_USAGE: &str = "usage: #ai search <query> | #ai list [>|>> <path> | | <command>]";
const DRAFT_USAGE: &str = "usage: #draft search <query> | #draft list [>|>> <path> | | <command>]";

fn history_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match args.split_whitespace().next() {
        Some("list") => history_list_command(state, out, args),
        Some("search") => history_search_command(state, out, args),
        _ => trim_history_command(state, out, args),
    }
}

fn history_list_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "history is still unlocking; run #unlock")?;
        return Ok(());
    }
    let rest = subcommand_rest(args, "list").unwrap_or_default();
    let sink = match parse_list_output_sink(rest, state.current_cwd.as_deref()) {
        Ok(sink) => sink,
        Err(_) => {
            writeln!(out, "{HISTORY_USAGE}")?;
            return Ok(());
        }
    };
    let output = list_output_from_commands(
        state
            .regular_history
            .iter()
            .rev()
            .map(|entry| entry.command.as_str()),
    );
    write_or_confirm_private_output(state, out, "history", output, sink)
}

fn history_search_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "history is still unlocking; run #unlock")?;
        return Ok(());
    }
    let Some(query) = search_query(args) else {
        writeln!(out, "{HISTORY_USAGE}")?;
        return Ok(());
    };
    let output = list_output_from_commands(
        state
            .regular_history
            .iter()
            .rev()
            .filter(|entry| entry.command.contains(query))
            .map(|entry| entry.command.as_str()),
    );
    out.write_all(output.as_bytes())?;
    Ok(())
}

fn ai_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match args.split_whitespace().next() {
        Some("list") => ai_list_command(state, out, args),
        Some("search") => ai_search_command(state, out, args),
        _ => {
            writeln!(out, "{AI_USAGE}")?;
            Ok(())
        }
    }
}

fn ai_list_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "history is still unlocking; run #unlock")?;
        return Ok(());
    }
    let rest = subcommand_rest(args, "list").unwrap_or_default();
    let sink = match parse_list_output_sink(rest, state.current_cwd.as_deref()) {
        Ok(sink) => sink,
        Err(_) => {
            writeln!(out, "{AI_USAGE}")?;
            return Ok(());
        }
    };
    let commands = ai_command_texts_newest(state);
    let output = list_output_from_commands(commands);
    write_or_confirm_private_output(state, out, "ai", output, sink)
}

fn ai_search_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "history is still unlocking; run #unlock")?;
        return Ok(());
    }
    let Some(query) = search_query(args) else {
        writeln!(out, "{AI_USAGE}")?;
        return Ok(());
    };
    let output = list_output_from_commands(
        ai_command_texts_newest(state)
            .into_iter()
            .filter(|command| command.contains(query)),
    );
    out.write_all(output.as_bytes())?;
    Ok(())
}

fn draft_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match args.split_whitespace().next() {
        Some("list") => draft_list_command(state, out, args),
        Some("search") => draft_search_command(state, out, args),
        _ => {
            writeln!(out, "{DRAFT_USAGE}")?;
            Ok(())
        }
    }
}

fn draft_list_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "drafts are still unlocking; run #unlock")?;
        return Ok(());
    }
    let rest = subcommand_rest(args, "list").unwrap_or_default();
    let sink = match parse_list_output_sink(rest, state.current_cwd.as_deref()) {
        Ok(sink) => sink,
        Err(_) => {
            writeln!(out, "{DRAFT_USAGE}")?;
            return Ok(());
        }
    };
    let output = list_output_from_commands(
        state
            .draft_history
            .iter()
            .rev()
            .map(|entry| entry.text.as_str()),
    );
    write_or_confirm_private_output(state, out, "draft", output, sink)
}

fn draft_search_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "drafts are still unlocking; run #unlock")?;
        return Ok(());
    }
    let Some(query) = search_query(args) else {
        writeln!(out, "{DRAFT_USAGE}")?;
        return Ok(());
    };
    let output = list_output_from_commands(
        state
            .draft_history
            .iter()
            .rev()
            .filter(|entry| entry.text.contains(query))
            .map(|entry| entry.text.as_str()),
    );
    out.write_all(output.as_bytes())?;
    Ok(())
}

fn ai_command_texts_newest(state: &AppState) -> Vec<&str> {
    let indices = ai_command_indices(&state.ai_sessions);
    indices
        .iter()
        .rev()
        .filter_map(|index| {
            let session = state.ai_sessions.get(index.session_index)?;
            let item = session.items.get(index.item_index)?;
            (item.kind == AiItemKind::Command).then_some(item.text.as_str())
        })
        .collect()
}

fn search_query(args: &str) -> Option<&str> {
    let query = subcommand_rest(args, "search")?.trim();
    (!query.is_empty()).then_some(query)
}

fn subcommand_rest<'a>(args: &'a str, subcommand: &str) -> Option<&'a str> {
    let args = args.trim_start();
    let rest = args.strip_prefix(subcommand)?;
    if rest.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
}

fn trim_history_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    if state.encrypted_storage_is_locked() {
        writeln!(out, "history is still unlocking; run #unlock")?;
        return Ok(());
    }
    let count = args.parse::<usize>();
    match (count, &state.regular_history_path, &state.ai_history_path) {
        (Ok(count), Some(_), Some(_)) => {
            let loaded = trim_history_for_state(state, count)?;
            let keep_from = loaded.regular.items.len().saturating_sub(count);
            state.regular_history = loaded.regular.items[keep_from..].to_vec();
            state.invalidate_completion_history_snapshot();
            state.ai_sessions = load_ai_sessions_for_state(state)?;
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
        (Err(_), _, _) => writeln!(out, "{HISTORY_USAGE}")?,
    }
    Ok(())
}

fn create_template_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match parse_template_body(args) {
        Some(body) => {
            if state.template_store_path.is_some() {
                let entry = TemplateEntry::new(body);
                let id = entry.id();
                state.append_template(&entry)?;
                writeln!(out, "template stored: {id}")?;
            } else {
                writeln!(out, "template storage is not configured")?;
            }
        }
        None => writeln!(out, "usage: #mt <template-body>")?,
    }
    Ok(())
}

fn template_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<bool> {
    let mut keep_draft = false;
    let subcommand = args.split_whitespace().next();
    if state.encrypted_storage_is_locked() && template_subcommand_needs_unlocked_storage(subcommand)
    {
        writeln!(out, "templates are still unlocking; run #unlock")?;
        return Ok(false);
    }
    match subcommand {
        Some("list") => template_list_command(state, out, args)?,
        Some("search") => template_search_command(state, out, args)?,
        Some("find") => template_find_command(state, out, args)?,
        Some("rm") => template_rm_command(state, out, args)?,
        Some("replace") => template_replace_command(state, out, args)?,
        Some("show") => template_show_command(state, out, args)?,
        Some("use") => {
            keep_draft = template_use_command(state, out, args)?;
        }
        Some("remote" | "publish" | "fetch" | "analyze" | "import") => {
            template_remote_command(state, out, args)?;
        }
        _ => writeln!(out, "{}", template_usage())?,
    }
    Ok(keep_draft)
}

fn template_subcommand_needs_unlocked_storage(subcommand: Option<&str>) -> bool {
    matches!(
        subcommand,
        Some(
            "list"
                | "search"
                | "find"
                | "rm"
                | "replace"
                | "show"
                | "use"
                | "publish"
                | "analyze"
                | "import",
        )
    )
}

fn template_list_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let rest = subcommand_rest(args, "list").unwrap_or_default();
    let sink = match parse_list_output_sink(rest, state.current_cwd.as_deref()) {
        Ok(sink) => sink,
        Err(_) => {
            writeln!(out, "{}", template_usage())?;
            return Ok(());
        }
    };
    if state.template_store_path.is_some() {
        let loaded = state.load_templates()?;
        let output = list_output_from_commands(
            loaded
                .items
                .iter()
                .rev()
                .map(|template| template.body.as_str()),
        );
        write_or_confirm_private_output(state, out, "template", output, sink)?;
    } else {
        writeln!(out, "template storage is not configured")?;
    }
    Ok(())
}

fn template_search_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let Some(query) = search_query(args) else {
        writeln!(out, "{}", template_usage())?;
        return Ok(());
    };
    if state.template_store_path.is_some() {
        let loaded = state.load_templates()?;
        let output = list_output_from_commands(
            loaded
                .items
                .iter()
                .rev()
                .filter(|template| template.id().contains(query) || template.body.contains(query))
                .map(|template| template.body.as_str()),
        );
        out.write_all(output.as_bytes())?;
    } else {
        writeln!(out, "template storage is not configured")?;
    }
    Ok(())
}

fn template_find_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match parse_template_find_query(args) {
        Some(query) => {
            if state.template_store_path.is_some() {
                let loaded = state.load_templates()?;
                let mut matches = Vec::new();
                for template in loaded.items.iter().rev() {
                    let id = template.id();
                    if id.contains(query) || template.body.contains(query) {
                        matches.push((id, template.body.as_str()));
                    }
                }
                if matches.is_empty() {
                    writeln!(out, "no templates matched: {query}")?;
                } else {
                    for (id, body) in matches {
                        writeln!(out, "template {id}\t{body}")?;
                    }
                }
                write_template_errors(out, loaded.errors.len())?;
            } else {
                writeln!(out, "template storage is not configured")?;
            }
        }
        None => writeln!(out, "{}", template_usage())?,
    }
    Ok(())
}

fn template_rm_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match args.split_whitespace().nth(1) {
        Some(id) => match state.remove_templates_by_id(id)? {
            Some(removal) => {
                writeln!(out, "template removed: {id} ({})", removal.removed)?;
                write_template_errors(out, removal.errors.len())?;
            }
            None => writeln!(out, "template storage is not configured")?,
        },
        None => writeln!(out, "{}", template_usage())?,
    }
    Ok(())
}

fn template_replace_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match parse_template_subcommand_args(args) {
        Some((id, body)) => {
            if state.template_store_path.is_some() {
                let entry = TemplateEntry::new(body);
                let new_id = entry.id();
                if let Some(removal) = state.replace_template_by_id(id, entry)? {
                    writeln!(
                        out,
                        "template replaced: {id} -> {new_id} (removed {})",
                        removal.removed
                    )?;
                    write_template_errors(out, removal.errors.len())?;
                } else {
                    writeln!(out, "template storage is not configured")?;
                }
            } else {
                writeln!(out, "template storage is not configured")?;
            }
        }
        None => writeln!(out, "{}", template_usage())?,
    }
    Ok(())
}

fn template_show_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<()> {
    match args.split_whitespace().nth(1) {
        Some(id) => {
            if state.template_store_path.is_some() {
                let loaded = state.find_template_by_id(id)?;
                match loaded.items.first() {
                    Some(template) => {
                        writeln!(out, "template: {}", template.id())?;
                        writeln!(out, "{}", template.body)?;
                    }
                    None => writeln!(out, "template not found: {id}")?,
                }
                write_template_errors(out, loaded.errors.len())?;
            } else {
                writeln!(out, "template storage is not configured")?;
            }
        }
        None => writeln!(out, "{}", template_usage())?,
    }
    Ok(())
}

fn template_use_command(state: &mut AppState, out: &mut impl Write, args: &str) -> Result<bool> {
    match args.split_whitespace().nth(1) {
        Some(id) => {
            if state.template_store_path.is_none() {
                writeln!(out, "template storage is not configured")?;
                return Ok(false);
            }
            let loaded = state.find_template_by_id(id)?;
            match loaded.items.first() {
                Some(template) => {
                    let values = parse_template_values(args);
                    let (rendered, used_keys) =
                        apply_template_values_with_usage(&template.body, &values);
                    state.draft = InputBuffer::from(rendered);
                    state.draft_from_editor = false;
                    state.draft_from_ai_editor = false;
                    state.draft_from_template = true;
                    state.draft_has_paste_preview = false;
                    writeln!(out, "template copied to draft: {}", template.id())?;
                    let placeholders = template_placeholders(&template.body);
                    if !placeholders.is_empty() {
                        writeln!(out, "template placeholders: {}", placeholders.join(", "))?;
                    }
                    let mut unresolved = template_placeholders(state.draft.as_str());
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
                        .filter(|key| !used_keys.iter().any(|used| used == *key))
                        .cloned()
                        .collect();
                    unused_keys.sort();
                    if !unused_keys.is_empty() {
                        writeln!(out, "unused template values: {}", unused_keys.join(", "))?;
                    }
                }
                None => {
                    writeln!(out, "template not found: {id}")?;
                    write_template_errors(out, loaded.errors.len())?;
                    return Ok(false);
                }
            }
            write_template_errors(out, loaded.errors.len())?;
            Ok(true)
        }
        None => {
            writeln!(out, "{}", template_usage())?;
            Ok(false)
        }
    }
}

fn write_template_errors(out: &mut impl Write, count: usize) -> Result<()> {
    if count != 0 {
        writeln!(out, "skipped {count} bad template line(s)")?;
    }
    Ok(())
}
