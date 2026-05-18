use std::io::Write;

use anyhow::Result;

use crate::commands::suggest_private_command;
use crate::history::ai_command_indices;
use crate::input::InputBuffer;
use crate::modes::Mode;
use crate::templates::{TemplateEntry, apply_template_values_with_usage, template_placeholders};

use super::{
    AppState, clear_stored_key, help, load_ai_sessions_for_state, parse_key_command,
    parse_template_body, parse_template_find_query, parse_template_subcommand_args,
    parse_template_values, prompt_command, run_manual_sync_push, set_stored_key, set_sync_remote,
    set_sync_schedule, show_event_log, template_usage, trim_history_for_state,
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
        "history" => trim_history_command(state, out, args)?,
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
        "push" => run_manual_sync_push(state, out)?,
        "sync" => set_sync_schedule(state, out, args)?,
        _ => match suggest_private_command(name) {
            Some(suggestion) => writeln!(
                out,
                "Aish command not implemented yet: #{name}. Did you mean #{suggestion}?"
            )?,
            None => writeln!(out, "Aish command not implemented yet: #{name}")?,
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
        (Err(_), _, _) => writeln!(out, "usage: #history <count>")?,
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
    if state.encrypted_storage_is_locked() && !matches!(subcommand, Some("list") | None) {
        writeln!(out, "templates are still unlocking; run #unlock")?;
        return Ok(false);
    }
    match subcommand {
        Some("list") => {
            writeln!(
                out,
                "template listing is intentionally not supported; use #template find <query> or inspect the template store file"
            )?;
        }
        Some("find") => template_find_command(state, out, args)?,
        Some("rm") => template_rm_command(state, out, args)?,
        Some("replace") => template_replace_command(state, out, args)?,
        Some("show") => template_show_command(state, out, args)?,
        Some("use") => {
            keep_draft = template_use_command(state, out, args)?;
        }
        _ => writeln!(out, "{}", template_usage())?,
    }
    Ok(keep_draft)
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
