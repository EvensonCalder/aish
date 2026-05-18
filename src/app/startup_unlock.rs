use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};

use crate::config::DirectoryLayout;
use crate::encryption::{
    encrypted_path, gpg_program, load_encrypted_jsonl_with_bytes,
    load_encrypted_jsonl_with_bytes_noninteractive,
};
use crate::history::{
    AiSession, DraftEntry, HistoryEntry, HistoryStore, JsonlLoad, NoteEntry, ai_command_indices,
    newest_first_indices,
};
use crate::templates::TemplateEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncryptedStartupPaths {
    pub regular_history: PathBuf,
    pub draft_history: PathBuf,
    pub ai_history: PathBuf,
    pub notes: PathBuf,
    pub template_store: PathBuf,
}

impl EncryptedStartupPaths {
    pub(crate) fn from_layout(layout: &DirectoryLayout) -> Self {
        Self {
            regular_history: layout.regular_history.clone(),
            draft_history: layout.draft_history.clone(),
            ai_history: layout.ai_history.clone(),
            notes: layout.notes.clone(),
            template_store: layout.template_store.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct EncryptedStartupData {
    pub store: HistoryStore,
    pub templates: JsonlLoad<TemplateEntry>,
    pub encrypted_cache: HashMap<PathBuf, Vec<u8>>,
}

type StartupUnlockMessage = std::result::Result<EncryptedStartupData, String>;

pub struct EncryptedStartupUnlock {
    receiver: mpsc::Receiver<StartupUnlockMessage>,
}

impl fmt::Debug for EncryptedStartupUnlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedStartupUnlock")
            .finish_non_exhaustive()
    }
}

impl EncryptedStartupUnlock {
    pub(crate) fn start(paths: EncryptedStartupPaths) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = load_encrypted_startup_data(&paths, UnlockMode::Noninteractive)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        Self { receiver }
    }

    pub(crate) fn try_recv(&self) -> Option<StartupUnlockMessage> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("encrypted startup unlock worker stopped".to_string()))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnlockMode {
    Interactive,
    Noninteractive,
}

pub(crate) fn load_encrypted_startup_data(
    paths: &EncryptedStartupPaths,
    mode: UnlockMode,
) -> Result<EncryptedStartupData> {
    let program = gpg_program();
    let mut encrypted_cache = HashMap::new();
    let (regular, regular_bytes) =
        load_encrypted_jsonl::<HistoryEntry>(&program, &paths.regular_history, mode)?;
    let (drafts, draft_bytes) =
        load_encrypted_jsonl::<DraftEntry>(&program, &paths.draft_history, mode)?;
    let (ai_sessions, ai_bytes) =
        load_encrypted_jsonl::<AiSession>(&program, &paths.ai_history, mode)?;
    let (notes, note_bytes) = load_encrypted_jsonl::<NoteEntry>(&program, &paths.notes, mode)?;
    let (templates, template_bytes) =
        load_encrypted_jsonl::<TemplateEntry>(&program, &paths.template_store, mode)?;

    encrypted_cache.insert(paths.regular_history.clone(), regular_bytes);
    encrypted_cache.insert(paths.draft_history.clone(), draft_bytes);
    encrypted_cache.insert(paths.ai_history.clone(), ai_bytes);
    encrypted_cache.insert(paths.notes.clone(), note_bytes);
    encrypted_cache.insert(paths.template_store.clone(), template_bytes);

    let mut errors = Vec::new();
    errors.extend(regular.errors);
    errors.extend(drafts.errors);
    errors.extend(ai_sessions.errors);
    errors.extend(notes.errors);

    let regular_newest_indices = newest_first_indices(regular.items.len());
    let ai_command_indices = ai_command_indices(&ai_sessions.items);
    Ok(EncryptedStartupData {
        store: HistoryStore {
            regular: regular.items,
            regular_newest_indices,
            drafts: drafts.items,
            ai_sessions: ai_sessions.items,
            ai_command_indices,
            notes: notes.items,
            errors,
        },
        templates,
        encrypted_cache,
    })
}

fn load_encrypted_jsonl<T: serde::de::DeserializeOwned>(
    program: &str,
    path: &PathBuf,
    mode: UnlockMode,
) -> Result<(JsonlLoad<T>, Vec<u8>)> {
    let encrypted = encrypted_path(path);
    match mode {
        UnlockMode::Interactive => load_encrypted_jsonl_with_bytes::<T>(program, path),
        UnlockMode::Noninteractive => {
            load_encrypted_jsonl_with_bytes_noninteractive::<T>(program, path)
        }
    }
    .with_context(|| {
        format!(
            "failed to unlock encrypted startup file {}",
            encrypted.display()
        )
    })
}

pub(crate) fn empty_history_store() -> HistoryStore {
    HistoryStore {
        regular: Vec::new(),
        regular_newest_indices: Vec::new(),
        drafts: Vec::new(),
        ai_sessions: Vec::new(),
        ai_command_indices: Vec::new(),
        notes: Vec::new(),
        errors: Vec::new(),
    }
}
