use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::commands::NoteTag;
use crate::config::{DirectoryLayout, create_private_dir_all, set_private_file_handle_permissions};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub t: i64,
    pub command: String,
    pub exit_code: Option<i32>,
    pub source: HistorySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    User,
    Ai,
    Editor,
    Paste,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEntry {
    pub tag: NoteTag,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftEntry {
    pub t: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSession {
    pub id: String,
    pub t: i64,
    pub prompt: String,
    pub ctx: bool,
    pub model: String,
    pub items: Vec<AiItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiItem {
    pub kind: AiItemKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiItemKind {
    Command,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonlLoad<T> {
    pub items: Vec<T>,
    pub errors: Vec<JsonlLineError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonlLineError {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStore {
    pub regular: Vec<HistoryEntry>,
    pub regular_newest_indices: Vec<usize>,
    pub drafts: Vec<DraftEntry>,
    pub ai_sessions: Vec<AiSession>,
    pub ai_command_indices: Vec<AiCommandIndex>,
    pub notes: Vec<NoteEntry>,
    pub errors: Vec<JsonlLineError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimHistoryLoad {
    pub regular: JsonlLoad<HistoryEntry>,
    pub ai_sessions: JsonlLoad<AiSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiCommandIndex {
    pub session_index: usize,
    pub item_index: usize,
}

impl HistoryStore {
    pub fn load(layout: &DirectoryLayout) -> Result<Self> {
        let regular = load_jsonl::<HistoryEntry>(&layout.regular_history)?;
        let drafts = load_jsonl::<DraftEntry>(&layout.draft_history)?;
        let ai_sessions = load_jsonl::<AiSession>(&layout.ai_history)?;
        let notes = load_jsonl::<NoteEntry>(&layout.notes)?;
        let regular_newest_indices = newest_first_indices(regular.items.len());
        let ai_command_indices = ai_command_indices(&ai_sessions.items);

        let mut errors = Vec::new();
        errors.extend(regular.errors);
        errors.extend(drafts.errors);
        errors.extend(ai_sessions.errors);
        errors.extend(notes.errors);

        Ok(Self {
            regular: regular.items,
            regular_newest_indices,
            drafts: drafts.items,
            ai_sessions: ai_sessions.items,
            ai_command_indices,
            notes: notes.items,
            errors,
        })
    }

    pub fn regular_newest(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.regular_newest_indices
            .iter()
            .map(|index| &self.regular[*index])
    }

    pub fn regular_by_newest_index(&self, index: usize) -> Option<&HistoryEntry> {
        self.regular_newest_indices
            .get(index)
            .map(|regular_index| &self.regular[*regular_index])
    }

    pub fn ai_commands(&self) -> impl Iterator<Item = (&AiSession, &AiItem)> {
        self.ai_command_indices.iter().map(|index| {
            let session = &self.ai_sessions[index.session_index];
            (session, &session.items[index.item_index])
        })
    }

    pub fn ai_command_by_index(&self, index: usize) -> Option<(&AiSession, &AiItem)> {
        self.ai_command_indices.get(index).map(|command_index| {
            let session = &self.ai_sessions[command_index.session_index];
            (session, &session.items[command_index.item_index])
        })
    }
}

pub fn newest_first_indices(len: usize) -> Vec<usize> {
    (0..len).rev().collect()
}

pub fn ai_command_indices(sessions: &[AiSession]) -> Vec<AiCommandIndex> {
    sessions
        .iter()
        .enumerate()
        .flat_map(|(session_index, session)| {
            session
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.kind == AiItemKind::Command)
                .map(move |(item_index, _)| AiCommandIndex {
                    session_index,
                    item_index,
                })
        })
        .collect()
}

pub fn split_logical_commands(input: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current = String::new();
    let mut quote_state = ShellQuoteState::default();
    let mut heredoc_delimiter: Option<String> = None;

    for line in input.lines() {
        let trimmed = line.trim();
        if (trimmed.is_empty() || trimmed.starts_with('#')) && current.is_empty() {
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
        if let Some(delimiter) = heredoc_delimiter.as_deref() {
            if trimmed == delimiter {
                let command = current.trim();
                if !command.is_empty() {
                    commands.push(command.to_string());
                }
                current.clear();
                quote_state = ShellQuoteState::default();
                heredoc_delimiter = None;
            }
            continue;
        }
        quote_state.update_line(line);
        if !quote_state.is_open() {
            heredoc_delimiter = parse_heredoc_delimiter(line);
        }
        if heredoc_delimiter.is_none()
            && !line_ends_with_continuation(line)
            && !quote_state.is_open()
        {
            let command = current.trim();
            if !command.is_empty() {
                commands.push(command.to_string());
            }
            current.clear();
            quote_state = ShellQuoteState::default();
        }
    }

    let command = current.trim();
    if !command.is_empty() {
        commands.push(command.to_string());
    }

    commands
}

fn line_ends_with_continuation(line: &str) -> bool {
    line.trim_end().ends_with('\\')
}

fn parse_heredoc_delimiter(line: &str) -> Option<String> {
    let marker = line.split_once("<<")?.1.trim_start();
    if marker.starts_with('<') || marker.is_empty() {
        return None;
    }
    let token = marker.split_whitespace().next()?.trim_matches(['\'', '"']);
    (!token.is_empty()).then_some(token.to_string())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ShellQuoteState {
    single: bool,
    double: bool,
}

impl ShellQuoteState {
    fn is_open(self) -> bool {
        self.single || self.double
    }

    fn update_line(&mut self, line: &str) {
        let mut escaped = false;
        for ch in line.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && !self.single {
                escaped = true;
                continue;
            }
            match ch {
                '\'' if !self.double => self.single = !self.single,
                '"' if !self.single => self.double = !self.double,
                _ => {}
            }
        }
    }
}

pub fn append_jsonl<T: Serialize>(path: &Path, item: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)
            .with_context(|| format!("failed to create JSONL directory {}", parent.display()))?;
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open JSONL file {}", path.display()))?;
    set_private_file_handle_permissions(&file, path)?;
    serde_json::to_writer(&mut file, item)
        .with_context(|| format!("failed to serialize JSONL item for {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to write JSONL newline to {}", path.display()))?;
    Ok(())
}

pub fn rewrite_jsonl<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)
            .with_context(|| format!("failed to create JSONL directory {}", parent.display()))?;
    }

    let tmp = JsonlRewriteTemp::new(path.with_extension("jsonl.tmp"));
    {
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(tmp.path()).with_context(|| {
            format!("failed to create JSONL temp file {}", tmp.path().display())
        })?;
        set_private_file_handle_permissions(&file, tmp.path())?;
        for item in items {
            serde_json::to_writer(&mut file, item).with_context(|| {
                format!(
                    "failed to serialize JSONL item for {}",
                    tmp.path().display()
                )
            })?;
            file.write_all(b"\n").with_context(|| {
                format!("failed to write JSONL newline to {}", tmp.path().display())
            })?;
        }
    }
    fs::rename(tmp.path(), path).with_context(|| {
        format!(
            "failed to replace JSONL file {} with {}",
            path.display(),
            tmp.path().display()
        )
    })?;
    tmp.disarm();
    Ok(())
}

struct JsonlRewriteTemp {
    path: PathBuf,
    remove_on_drop: bool,
}

impl JsonlRewriteTemp {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            remove_on_drop: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(mut self) {
        self.remove_on_drop = false;
    }
}

impl Drop for JsonlRewriteTemp {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn trim_regular_history(path: &Path, max_entries: usize) -> Result<JsonlLoad<HistoryEntry>> {
    let loaded = load_jsonl::<HistoryEntry>(path)?;
    let keep_from = loaded.items.len().saturating_sub(max_entries);
    rewrite_jsonl(path, &loaded.items[keep_from..])?;
    Ok(loaded)
}

pub fn trim_combined_history(
    regular_path: &Path,
    ai_path: &Path,
    max_entries: usize,
) -> Result<TrimHistoryLoad> {
    let regular = load_jsonl::<HistoryEntry>(regular_path)?;
    let ai_sessions = load_jsonl::<AiSession>(ai_path)?;

    let keep_from = regular.items.len().saturating_sub(max_entries);
    let trimmed_regular = regular.items[keep_from..].to_vec();

    let mut remaining_ai_commands = max_entries.saturating_sub(trimmed_regular.len());
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

    rewrite_jsonl(regular_path, &trimmed_regular)?;
    rewrite_jsonl(ai_path, &trimmed_ai_sessions)?;

    Ok(TrimHistoryLoad {
        regular,
        ai_sessions,
    })
}

pub fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<JsonlLoad<T>> {
    if !path.exists() {
        return Ok(JsonlLoad {
            items: Vec::new(),
            errors: Vec::new(),
        });
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open JSONL file {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    let mut errors = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.with_context(|| {
            format!(
                "failed to read line {line_number} from JSONL file {}",
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(&line) {
            Ok(item) => items.push(item),
            Err(error) => errors.push(JsonlLineError {
                path: path.to_path_buf(),
                line: line_number,
                message: error.to_string(),
            }),
        }
    }

    Ok(JsonlLoad { items, errors })
}

#[cfg(test)]
mod tests;
