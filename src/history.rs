use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::commands::NoteTag;
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
}
