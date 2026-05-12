use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::commands::NoteTag;
use crate::config::DirectoryLayout;
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

    for line in input.lines() {
        let trimmed = line.trim();
        if (trimmed.is_empty() || trimmed.starts_with('#')) && current.is_empty() {
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
        quote_state.update_line(line);
        if !line_ends_with_continuation(line) && !quote_state.is_open() {
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create JSONL directory {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open JSONL file {}", path.display()))?;
    serde_json::to_writer(&mut file, item)
        .with_context(|| format!("failed to serialize JSONL item for {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to write JSONL newline to {}", path.display()))?;
    Ok(())
}

pub fn rewrite_jsonl<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create JSONL directory {}", parent.display()))?;
    }

    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create JSONL temp file {}", tmp.display()))?;
        for item in items {
            serde_json::to_writer(&mut file, item)
                .with_context(|| format!("failed to serialize JSONL item for {}", tmp.display()))?;
            file.write_all(b"\n")
                .with_context(|| format!("failed to write JSONL newline to {}", tmp.display()))?;
        }
    }
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to replace JSONL file {} with {}",
            path.display(),
            tmp.display()
        )
    })?;
    Ok(())
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
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestEntry {
        command: String,
        exit_code: Option<i32>,
    }

    #[test]
    fn append_and_load_jsonl_items() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history/regular.jsonl");

        append_jsonl(
            &path,
            &TestEntry {
                command: "pwd".to_string(),
                exit_code: Some(0),
            },
        )
        .unwrap();
        append_jsonl(
            &path,
            &TestEntry {
                command: "false".to_string(),
                exit_code: Some(1),
            },
        )
        .unwrap();

        let loaded = load_jsonl::<TestEntry>(&path).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].command, "pwd");
        assert_eq!(loaded.items[1].exit_code, Some(1));
    }

    #[test]
    fn missing_jsonl_file_loads_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let loaded = load_jsonl::<TestEntry>(&temp.path().join("missing.jsonl")).unwrap();

        assert!(loaded.items.is_empty());
        assert!(loaded.errors.is_empty());
    }

    #[test]
    fn bad_jsonl_lines_are_reported_and_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("regular.jsonl");
        fs::write(
            &path,
            "{\"command\":\"pwd\",\"exit_code\":0}\nnot-json\n\n{\"command\":\"false\",\"exit_code\":1}\n",
        )
        .unwrap();

        let loaded = load_jsonl::<TestEntry>(&path).unwrap();

        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.errors.len(), 1);
        assert_eq!(loaded.errors[0].line, 2);
        assert_eq!(loaded.errors[0].path, path);
        assert!(loaded.errors[0].message.contains("expected"));
    }

    #[test]
    fn rewrite_jsonl_replaces_existing_contents() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("regular.jsonl");
        append_jsonl(
            &path,
            &TestEntry {
                command: "old".to_string(),
                exit_code: Some(0),
            },
        )
        .unwrap();

        rewrite_jsonl(
            &path,
            &[TestEntry {
                command: "new".to_string(),
                exit_code: Some(1),
            }],
        )
        .unwrap();

        let loaded = load_jsonl::<TestEntry>(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].command, "new");
    }

    #[test]
    fn trim_regular_history_keeps_newest_entries_and_skips_bad_lines() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("regular.jsonl");
        fs::write(
            &path,
            [
                "{\"t\":1,\"command\":\"one\",\"exit_code\":0,\"source\":\"user\"}",
                "bad-json",
                "{\"t\":2,\"command\":\"two\",\"exit_code\":0,\"source\":\"user\"}",
                "{\"t\":3,\"command\":\"three\",\"exit_code\":1,\"source\":\"user\"}",
                "",
            ]
            .join("\n"),
        )
        .unwrap();

        let before_trim = trim_regular_history(&path, 2).unwrap();
        let after_trim = load_jsonl::<HistoryEntry>(&path).unwrap();

        assert_eq!(before_trim.items.len(), 3);
        assert_eq!(before_trim.errors.len(), 1);
        assert_eq!(after_trim.errors, []);
        assert_eq!(after_trim.items.len(), 2);
        assert_eq!(after_trim.items[0].command, "two");
        assert_eq!(after_trim.items[1].command, "three");
    }

    #[test]
    fn trim_combined_history_limits_regular_plus_ai_command_items() {
        let temp = tempfile::tempdir().unwrap();
        let regular_path = temp.path().join("regular.jsonl");
        let ai_path = temp.path().join("ai.jsonl");

        for (t, command) in [(1, "one"), (2, "two"), (3, "three")] {
            append_jsonl(
                &regular_path,
                &HistoryEntry {
                    t,
                    command: command.to_string(),
                    exit_code: Some(0),
                    source: HistorySource::User,
                },
            )
            .unwrap();
        }

        append_jsonl(
            &ai_path,
            &AiSession {
                id: "a_1".to_string(),
                t: 4,
                prompt: "older".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "ai one".to_string(),
                        name: None,
                    },
                    AiItem {
                        kind: AiItemKind::Template,
                        text: "template one".to_string(),
                        name: Some("t1".to_string()),
                    },
                ],
            },
        )
        .unwrap();
        append_jsonl(
            &ai_path,
            &AiSession {
                id: "a_2".to_string(),
                t: 5,
                prompt: "newer".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "ai two".to_string(),
                        name: None,
                    },
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "ai three".to_string(),
                        name: None,
                    },
                ],
            },
        )
        .unwrap();

        let before_trim = trim_combined_history(&regular_path, &ai_path, 2).unwrap();
        let after_regular = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
        let after_ai = load_jsonl::<AiSession>(&ai_path).unwrap();

        assert_eq!(before_trim.regular.items.len(), 3);
        assert_eq!(before_trim.ai_sessions.items.len(), 2);
        assert_eq!(after_regular.items.len(), 2);
        assert_eq!(after_regular.items[0].command, "two");
        assert_eq!(after_regular.items[1].command, "three");
        assert!(after_ai.items.is_empty());
    }

    #[test]
    fn history_entry_serializes_source_as_snake_case() {
        let entry = HistoryEntry {
            t: 123,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        };

        let raw = serde_json::to_string(&entry).unwrap();

        assert!(raw.contains("\"source\":\"user\""));
        assert!(raw.contains("\"t\":123"));
    }

    #[test]
    fn note_entry_serializes_tag_as_snake_case() {
        let entry = NoteEntry {
            tag: NoteTag::Fixme,
            text: "clean this up".to_string(),
        };

        let raw = serde_json::to_string(&entry).unwrap();

        assert!(raw.contains("\"tag\":\"fixme\""));
    }

    #[test]
    fn draft_entry_roundtrips_through_json() {
        let entry = DraftEntry {
            t: 123,
            text: "git status".to_string(),
        };

        let raw = serde_json::to_string(&entry).unwrap();
        let parsed: DraftEntry = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed, entry);
    }

    #[test]
    fn ai_session_roundtrips_through_jsonl() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history/ai.jsonl");
        let session = AiSession {
            id: "a_123".to_string(),
            t: 123,
            prompt: "set git user".to_string(),
            ctx: false,
            model: "test-model".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "git config --global user.name \"{name}\"".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Template,
                    text: "git config --global user.email \"{email}\"".to_string(),
                    name: Some("git-email".to_string()),
                },
            ],
        };

        append_jsonl(&path, &session).unwrap();
        let loaded = load_jsonl::<AiSession>(&path).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items, [session]);
        assert_eq!(loaded.items[0].items[0].kind, AiItemKind::Command);
        assert_eq!(loaded.items[0].items[1].kind, AiItemKind::Template);
    }

    #[test]
    fn ai_item_kind_serializes_as_snake_case() {
        let item = AiItem {
            kind: AiItemKind::Command,
            text: "pwd".to_string(),
            name: None,
        };

        let raw = serde_json::to_string(&item).unwrap();

        assert!(raw.contains("\"kind\":\"command\""));
        assert!(!raw.contains("name"));
    }

    #[test]
    fn history_store_loads_all_history_categories() {
        let temp = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(temp.path().join("aish-home"));
        layout.create_dirs().unwrap();

        append_jsonl(
            &layout.regular_history,
            &HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        )
        .unwrap();
        append_jsonl(
            &layout.draft_history,
            &DraftEntry {
                t: 2,
                text: "git status".to_string(),
            },
        )
        .unwrap();
        append_jsonl(
            &layout.ai_history,
            &AiSession {
                id: "a_1".to_string(),
                t: 3,
                prompt: "list files".to_string(),
                ctx: false,
                model: "test-model".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "ls".to_string(),
                    name: None,
                }],
            },
        )
        .unwrap();
        append_jsonl(
            &layout.notes,
            &NoteEntry {
                tag: NoteTag::Todo,
                text: "ship it".to_string(),
            },
        )
        .unwrap();

        let store = HistoryStore::load(&layout).unwrap();

        assert_eq!(store.errors, []);
        assert_eq!(store.regular.len(), 1);
        assert_eq!(store.regular_newest_indices, [0]);
        assert_eq!(store.drafts.len(), 1);
        assert_eq!(store.ai_sessions.len(), 1);
        assert_eq!(store.ai_command_indices.len(), 1);
        assert_eq!(store.notes.len(), 1);
        assert_eq!(store.regular[0].command, "pwd");
        assert_eq!(store.drafts[0].text, "git status");
        assert_eq!(store.ai_sessions[0].items[0].text, "ls");
        assert_eq!(store.notes[0].text, "ship it");
    }

    #[test]
    fn history_store_indexes_regular_history_newest_first() {
        let temp = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(temp.path().join("aish-home"));
        layout.create_dirs().unwrap();

        for (t, command) in [(1, "one"), (2, "two"), (3, "three")] {
            append_jsonl(
                &layout.regular_history,
                &HistoryEntry {
                    t,
                    command: command.to_string(),
                    exit_code: Some(0),
                    source: HistorySource::User,
                },
            )
            .unwrap();
        }

        let store = HistoryStore::load(&layout).unwrap();
        let commands: Vec<_> = store
            .regular_newest()
            .map(|entry| entry.command.as_str())
            .collect();

        assert_eq!(store.regular_newest_indices, [2, 1, 0]);
        assert_eq!(commands, ["three", "two", "one"]);
        assert_eq!(store.regular_by_newest_index(1).unwrap().command, "two");
        assert!(store.regular_by_newest_index(3).is_none());
    }

    #[test]
    fn history_store_indexes_ai_command_items_in_execution_order() {
        let temp = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(temp.path().join("aish-home"));
        layout.create_dirs().unwrap();

        append_jsonl(
            &layout.ai_history,
            &AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "setup".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "one".to_string(),
                        name: None,
                    },
                    AiItem {
                        kind: AiItemKind::Template,
                        text: "skip-template".to_string(),
                        name: Some("template".to_string()),
                    },
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "two".to_string(),
                        name: None,
                    },
                ],
            },
        )
        .unwrap();
        append_jsonl(
            &layout.ai_history,
            &AiSession {
                id: "a_2".to_string(),
                t: 2,
                prompt: "next".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![AiItem {
                    kind: AiItemKind::Command,
                    text: "three".to_string(),
                    name: None,
                }],
            },
        )
        .unwrap();

        let store = HistoryStore::load(&layout).unwrap();
        let commands: Vec<_> = store
            .ai_commands()
            .map(|(_, item)| item.text.as_str())
            .collect();

        assert_eq!(
            store.ai_command_indices,
            [
                AiCommandIndex {
                    session_index: 0,
                    item_index: 0
                },
                AiCommandIndex {
                    session_index: 0,
                    item_index: 2
                },
                AiCommandIndex {
                    session_index: 1,
                    item_index: 0
                },
            ]
        );
        assert_eq!(commands, ["one", "two", "three"]);
        assert_eq!(store.ai_command_by_index(1).unwrap().1.text, "two");
        assert!(store.ai_command_by_index(3).is_none());
    }

    #[test]
    fn split_logical_commands_splits_simple_non_empty_lines() {
        let commands = split_logical_commands("\ncd /tmp\n\npwd\n");

        assert_eq!(commands, ["cd /tmp", "pwd"]);
    }

    #[test]
    fn split_logical_commands_preserves_backslash_continuations() {
        let commands = split_logical_commands("echo foo \\\n+bar\npwd");

        assert_eq!(commands, ["echo foo \\\n+bar", "pwd"]);
    }

    #[test]
    fn split_logical_commands_skips_standalone_comments() {
        let commands = split_logical_commands("# comment\npwd\n  # another\necho done");

        assert_eq!(commands, ["pwd", "echo done"]);
    }

    #[test]
    fn split_logical_commands_preserves_inline_hash_content() {
        let commands = split_logical_commands("echo '# not a comment'\necho value # inline");

        assert_eq!(commands, ["echo '# not a comment'", "echo value # inline"]);
    }

    #[test]
    fn split_logical_commands_preserves_single_quoted_newlines() {
        let commands = split_logical_commands("printf 'one\ntwo'\npwd");

        assert_eq!(commands, ["printf 'one\ntwo'", "pwd"]);
    }

    #[test]
    fn split_logical_commands_preserves_double_quoted_newlines() {
        let commands = split_logical_commands("printf \"one\ntwo\"\npwd");

        assert_eq!(commands, ["printf \"one\ntwo\"", "pwd"]);
    }

    #[test]
    fn split_logical_commands_ignores_escaped_quotes() {
        let commands = split_logical_commands("echo \"one \\\"two\\\"\"\npwd");

        assert_eq!(commands, ["echo \"one \\\"two\\\"\"", "pwd"]);
    }

    #[test]
    fn history_store_aggregates_load_errors_across_categories() {
        let temp = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(temp.path().join("aish-home"));
        layout.create_dirs().unwrap();
        fs::write(&layout.regular_history, "bad-regular\n").unwrap();
        fs::write(&layout.ai_history, "bad-ai\n").unwrap();

        let store = HistoryStore::load(&layout).unwrap();

        assert!(store.regular.is_empty());
        assert!(store.ai_sessions.is_empty());
        assert_eq!(store.errors.len(), 2);
        assert!(
            store
                .errors
                .iter()
                .any(|error| error.path == layout.regular_history)
        );
        assert!(
            store
                .errors
                .iter()
                .any(|error| error.path == layout.ai_history)
        );
    }
}
