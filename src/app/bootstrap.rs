use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::config::{self, EncryptionConfig, EncryptionStartupUnlockMode};
use crate::history::{HistoryStore, JsonlLoad};
use crate::pty::PtyBackend;
use crate::templates::{TemplateEntry, load_templates};

use super::{
    AppState,
    startup_unlock::{
        EncryptedStartupPaths, EncryptedStartupUnlock, UnlockMode, empty_history_store,
        load_encrypted_startup_data,
    },
    sync_commands::run_startup_sync_check,
};

pub fn run() -> Result<()> {
    let (layout, config) = config::init_default_layout(config::runtime_aish_dir()?)?;
    let encryption_enabled = config.encryption.enabled;
    let encrypted_startup = load_initial_encrypted_storage(&layout, &config.encryption)?;
    let store = encrypted_startup.store;
    let templates = encrypted_startup.templates;
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
        keybinding_config: config.keybindings,
        ai_config: config.ai,
        context_config: config.context,
        encryption_config: config.encryption,
        encrypted_storage_unlocked: encrypted_startup.unlocked,
        encrypted_startup_unlock: encrypted_startup.background_unlock,
        encrypted_startup_unlock_message: encrypted_startup.message,
        sync_config: config.sync,
        editor_temp_root: Some(layout.runtime_cache.join("editor")),
        ..AppState::default()
    };
    if encryption_enabled {
        state.start_encrypted_writer_with_cache(encrypted_startup.encrypted_cache);
    }
    run_startup_sync_check(&mut state, &layout.root, &mut io::stdout())?;
    crate::terminal::run(
        &mut state,
        &mut backend,
        &mut io::stdout(),
        Duration::from_secs(60),
    )
}

struct InitialEncryptedStorage {
    store: HistoryStore,
    templates: JsonlLoad<TemplateEntry>,
    encrypted_cache: HashMap<PathBuf, Vec<u8>>,
    unlocked: bool,
    background_unlock: Option<EncryptedStartupUnlock>,
    message: Option<String>,
}

fn load_initial_encrypted_storage(
    layout: &config::DirectoryLayout,
    encryption: &EncryptionConfig,
) -> Result<InitialEncryptedStorage> {
    if !encryption.enabled {
        return Ok(InitialEncryptedStorage {
            store: HistoryStore::load(layout)?,
            templates: load_templates(&layout.template_store)?,
            encrypted_cache: HashMap::new(),
            unlocked: true,
            background_unlock: None,
            message: None,
        });
    }

    match encryption.startup_unlock {
        EncryptionStartupUnlockMode::Prompt => {
            let paths = EncryptedStartupPaths::from_layout(layout);
            let data = load_encrypted_startup_data(&paths, UnlockMode::Interactive)?;
            Ok(InitialEncryptedStorage {
                store: data.store,
                templates: data.templates,
                encrypted_cache: data.encrypted_cache,
                unlocked: true,
                background_unlock: None,
                message: Some("encrypted storage unlocked".to_string()),
            })
        }
        EncryptionStartupUnlockMode::Lazy => Ok(InitialEncryptedStorage {
            store: empty_history_store(),
            templates: JsonlLoad {
                items: Vec::new(),
                errors: Vec::new(),
            },
            encrypted_cache: HashMap::new(),
            unlocked: false,
            background_unlock: Some(EncryptedStartupUnlock::start(
                EncryptedStartupPaths::from_layout(layout),
            )),
            message: Some("encrypted storage unlocking in background".to_string()),
        }),
    }
}
