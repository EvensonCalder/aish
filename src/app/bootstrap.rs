use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::config::{self, EncryptionConfig};
use crate::encryption::{gpg_program, load_encrypted_jsonl_with_bytes};
use crate::history::{
    AiSession, DraftEntry, HistoryEntry, HistoryStore, JsonlLoad, NoteEntry, ai_command_indices,
    newest_first_indices,
};
use crate::pty::PtyBackend;
use crate::templates::{TemplateEntry, load_templates};

use super::{AppState, sync_commands::run_startup_sync_check};

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::runtime_aish_dir()?)?;
    let mut encrypted_cache = HashMap::new();
    let store = load_history_store(&layout, &config.encryption, &mut encrypted_cache)?;
    let templates = load_template_store(&layout, &config.encryption, &mut encrypted_cache)?;
    let mut backend = PtyBackend::spawn(&config.shell.backend)?;
    let mut state = AppState {
        current_cwd: backend.initial_cwd().map(PathBuf::from),
        backend_shell: Some(backend.shell_program().to_string()),
        regular_history_path: Some(layout.regular_history),
        ai_history_path: Some(layout.ai_history),
        notes_path: Some(layout.notes),
        draft_history_path: Some(layout.draft_history),
        events_path: Some(layout.events),
        template_store_path: Some(layout.template_store),
        secret_key_path: Some(layout.secrets.join("key.json.gpg")),
        config_path: Some(layout.config),
        draft_persist: config.draft.persist,
        draft_history: store.drafts,
        regular_history: store.regular,
        ai_sessions: store.ai_sessions,
        ai_command_indices: store.ai_command_indices,
        templates: templates.items,
        template_errors: templates.errors,
        prompt_templates: config.prompt.into(),
        editor_config: config.editor,
        paste_config: config.paste,
        completion_config: config.completion,
        ai_config: config.ai,
        context_config: config.context,
        encryption_config: config.encryption,
        sync_config: config.sync,
        editor_temp_root: Some(layout.runtime_cache.join("editor")),
        ..AppState::default()
    };
    state.start_encrypted_writer_with_cache(encrypted_cache);
    run_startup_sync_check(&mut state, &layout.root, &mut io::stdout())?;
    crate::terminal::run(
        &mut state,
        &mut backend,
        &mut io::stdout(),
        Duration::from_secs(60),
    )
}

fn load_template_store(
    layout: &config::DirectoryLayout,
    encryption: &EncryptionConfig,
    encrypted_cache: &mut HashMap<PathBuf, Vec<u8>>,
) -> Result<JsonlLoad<TemplateEntry>> {
    if encryption.enabled {
        let (loaded, bytes) = load_encrypted_jsonl_with_bytes::<TemplateEntry>(
            gpg_program(),
            &layout.template_store,
        )?;
        encrypted_cache.insert(layout.template_store.clone(), bytes);
        Ok(loaded)
    } else {
        load_templates(&layout.template_store)
    }
}

fn load_history_store(
    layout: &config::DirectoryLayout,
    encryption: &EncryptionConfig,
    encrypted_cache: &mut HashMap<PathBuf, Vec<u8>>,
) -> Result<HistoryStore> {
    if !encryption.enabled {
        return HistoryStore::load(layout);
    }

    let program = gpg_program();
    let (regular, regular_bytes) =
        load_encrypted_jsonl_with_bytes::<HistoryEntry>(&program, &layout.regular_history)?;
    let (drafts, draft_bytes) =
        load_encrypted_jsonl_with_bytes::<DraftEntry>(&program, &layout.draft_history)?;
    let (ai_sessions, ai_bytes) =
        load_encrypted_jsonl_with_bytes::<AiSession>(&program, &layout.ai_history)?;
    let (notes, note_bytes) =
        load_encrypted_jsonl_with_bytes::<NoteEntry>(&program, &layout.notes)?;
    encrypted_cache.insert(layout.regular_history.clone(), regular_bytes);
    encrypted_cache.insert(layout.draft_history.clone(), draft_bytes);
    encrypted_cache.insert(layout.ai_history.clone(), ai_bytes);
    encrypted_cache.insert(layout.notes.clone(), note_bytes);
    let regular_newest_indices = newest_first_indices(regular.items.len());
    let ai_command_indices = ai_command_indices(&ai_sessions.items);

    let mut errors = Vec::new();
    errors.extend(regular.errors);
    errors.extend(drafts.errors);
    errors.extend(ai_sessions.errors);
    errors.extend(notes.errors);

    Ok(HistoryStore {
        regular: regular.items,
        regular_newest_indices,
        drafts: drafts.items,
        ai_sessions: ai_sessions.items,
        ai_command_indices,
        notes: notes.items,
        errors,
    })
}
