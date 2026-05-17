use anyhow::Result;

use crate::encryption::{gpg_program, load_encrypted_jsonl, rewrite_encrypted_jsonl};
use crate::history::{
    AiItemKind, AiSession, HistoryEntry, TrimHistoryLoad, load_jsonl, trim_combined_history,
};

use super::{AppState, configured_encryption_key};

pub(super) fn trim_history_for_state(state: &AppState, count: usize) -> Result<TrimHistoryLoad> {
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

    Ok(TrimHistoryLoad {
        regular,
        ai_sessions,
    })
}

pub(super) fn load_ai_sessions_for_state(state: &AppState) -> Result<Vec<AiSession>> {
    let Some(ai_path) = &state.ai_history_path else {
        return Ok(Vec::new());
    };
    if state.encryption_config.enabled {
        Ok(load_encrypted_jsonl::<AiSession>(gpg_program(), ai_path)?.items)
    } else {
        Ok(load_jsonl::<AiSession>(ai_path)?.items)
    }
}
