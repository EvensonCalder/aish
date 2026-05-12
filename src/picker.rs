use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::history::{AiItemKind, AiSession, HistoryEntry};
use crate::templates::TemplateEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    InsertAtCursor,
    ReplaceCurrentToken,
    AppendAsArgument,
    ReplaceLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEdit {
    pub line: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRunResult {
    pub selected: Option<String>,
    pub exit_code: Option<i32>,
}

pub fn file_picker_candidates(root: &Path) -> Result<Vec<String>> {
    let mut candidates = Vec::new();
    collect_file_candidates(root, root, &mut candidates)?;
    candidates.sort();
    Ok(candidates)
}

pub fn regular_history_picker_candidates(history: &[HistoryEntry]) -> Vec<String> {
    history
        .iter()
        .rev()
        .map(|entry| entry.command.clone())
        .collect()
}

pub fn ai_history_picker_candidates(sessions: &[AiSession]) -> Vec<String> {
    sessions
        .iter()
        .flat_map(|session| &session.items)
        .filter(|item| item.kind == AiItemKind::Command)
        .map(|item| item.text.clone())
        .collect()
}

pub fn combined_history_picker_candidates(
    history: &[HistoryEntry],
    sessions: &[AiSession],
) -> Vec<String> {
    let mut candidates = regular_history_picker_candidates(history);
    candidates.extend(ai_history_picker_candidates(sessions));
    candidates
}

pub fn template_picker_candidates(templates: &[TemplateEntry]) -> Vec<String> {
    let mut candidates = Vec::new();
    for template in templates.iter().rev() {
        if !candidates.iter().any(|name| name == &template.name) {
            candidates.push(template.name.clone());
        }
    }
    candidates
}

fn collect_file_candidates(root: &Path, dir: &Path, candidates: &mut Vec<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let mut display = relative.display().to_string();
        if entry.file_type()?.is_dir() {
            display.push('/');
            candidates.push(display);
            collect_file_candidates(root, &path, candidates)?;
        } else {
            candidates.push(display);
        }
    }
    Ok(())
}

pub fn default_fzf_command() -> Vec<String> {
    vec!["fzf".to_string()]
}

pub fn run_fzf_picker(candidates: &[String]) -> Result<PickerRunResult> {
    run_picker_command(&default_fzf_command(), candidates)
}

pub fn run_picker_command(command: &[String], candidates: &[String]) -> Result<PickerRunResult> {
    let Some((program, args)) = command.split_first() else {
        bail!("picker command is empty");
    };

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run picker command {program}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        for candidate in candidates {
            writeln!(stdin, "{candidate}")?;
        }
    }

    let output = child.wait_with_output()?;
    let selected = output
        .status
        .success()
        .then(|| first_output_line(&output.stdout))
        .flatten();

    Ok(PickerRunResult {
        selected,
        exit_code: output.status.code(),
    })
}

fn first_output_line(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .lines()
        .next()
        .map(str::to_string)
}

pub fn apply_picker_result(
    line: &str,
    cursor: usize,
    value: &str,
    action: PickerAction,
) -> PickerEdit {
    let cursor = previous_char_boundary(line, cursor.min(line.len()));
    let quoted = shell_quote(value);
    match action {
        PickerAction::InsertAtCursor => insert_at(line, cursor, &quoted),
        PickerAction::ReplaceCurrentToken => replace_token(line, cursor, &quoted),
        PickerAction::AppendAsArgument => append_argument(line, &quoted),
        PickerAction::ReplaceLine => PickerEdit {
            line: quoted.clone(),
            cursor: quoted.len(),
        },
    }
}

pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn insert_at(line: &str, cursor: usize, value: &str) -> PickerEdit {
    let mut edited = String::with_capacity(line.len() + value.len());
    edited.push_str(&line[..cursor]);
    edited.push_str(value);
    edited.push_str(&line[cursor..]);
    PickerEdit {
        line: edited,
        cursor: cursor + value.len(),
    }
}

fn replace_token(line: &str, cursor: usize, value: &str) -> PickerEdit {
    let (start, end) = token_span(line, cursor);
    let mut edited = String::with_capacity(line.len() - (end - start) + value.len());
    edited.push_str(&line[..start]);
    edited.push_str(value);
    edited.push_str(&line[end..]);
    PickerEdit {
        line: edited,
        cursor: start + value.len(),
    }
}

fn append_argument(line: &str, value: &str) -> PickerEdit {
    let mut edited = line.trim_end().to_string();
    if !edited.is_empty() {
        edited.push(' ');
    }
    edited.push_str(value);
    let cursor = edited.len();
    PickerEdit {
        line: edited,
        cursor,
    }
}

fn token_span(line: &str, cursor: usize) -> (usize, usize) {
    let mut start = cursor;
    while start > 0 {
        let previous = previous_char_boundary(line, start - 1);
        if line[previous..start].chars().all(char::is_whitespace) {
            break;
        }
        start = previous;
    }

    let mut end = cursor;
    while end < line.len() {
        let next = next_char_boundary(line, end);
        if line[end..next].chars().all(char::is_whitespace) {
            break;
        }
        end = next;
    }
    (start, end)
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    if text.is_char_boundary(cursor) {
        return cursor;
    }
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_leaves_safe_values_unquoted() {
        assert_eq!(shell_quote("src/main.rs"), "src/main.rs");
        assert_eq!(shell_quote("KEY=value"), "KEY=value");
    }

    #[test]
    fn shell_quote_quotes_spaces_and_embedded_single_quotes() {
        assert_eq!(shell_quote("my file.txt"), "'my file.txt'");
        assert_eq!(shell_quote("it's.txt"), "'it'\\''s.txt'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn picker_insert_at_cursor_inserts_quoted_value() {
        assert_eq!(
            apply_picker_result("cat ", 4, "my file.txt", PickerAction::InsertAtCursor),
            PickerEdit {
                line: "cat 'my file.txt'".to_string(),
                cursor: 17,
            }
        );
    }

    #[test]
    fn picker_replace_current_token_replaces_token_under_cursor() {
        assert_eq!(
            apply_picker_result(
                "cat old.txt --raw",
                5,
                "new file.txt",
                PickerAction::ReplaceCurrentToken
            ),
            PickerEdit {
                line: "cat 'new file.txt' --raw".to_string(),
                cursor: 18,
            }
        );
    }

    #[test]
    fn picker_append_as_argument_adds_separator_when_needed() {
        assert_eq!(
            apply_picker_result("git add", 7, "src/main.rs", PickerAction::AppendAsArgument),
            PickerEdit {
                line: "git add src/main.rs".to_string(),
                cursor: 19,
            }
        );
    }

    #[test]
    fn picker_replace_line_replaces_everything() {
        assert_eq!(
            apply_picker_result("old command", 3, "new command", PickerAction::ReplaceLine),
            PickerEdit {
                line: "'new command'".to_string(),
                cursor: 13,
            }
        );
    }

    #[test]
    fn run_picker_command_returns_selected_stdout_line() {
        let command = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sed -n '2p'".to_string(),
        ];
        let candidates = vec!["one".to_string(), "two".to_string(), "three".to_string()];

        let result = run_picker_command(&command, &candidates).unwrap();

        assert_eq!(
            result,
            PickerRunResult {
                selected: Some("two".to_string()),
                exit_code: Some(0),
            }
        );
    }

    #[test]
    fn run_picker_command_returns_none_on_cancel_status() {
        let command = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exit 130".to_string(),
        ];
        let candidates = vec!["one".to_string()];

        let result = run_picker_command(&command, &candidates).unwrap();

        assert_eq!(
            result,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            }
        );
    }

    #[test]
    fn run_picker_command_rejects_empty_command() {
        let error = run_picker_command(&[], &[]).unwrap_err();

        assert!(error.to_string().contains("picker command is empty"));
    }

    #[test]
    fn default_fzf_command_uses_external_fzf() {
        assert_eq!(default_fzf_command(), ["fzf"]);
    }

    #[test]
    fn file_picker_candidates_returns_sorted_relative_files_and_dirs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        std::fs::write(temp.path().join("README.md"), "").unwrap();

        assert_eq!(
            file_picker_candidates(temp.path()).unwrap(),
            ["README.md", "src/", "src/main.rs"]
        );
    }

    #[test]
    fn history_picker_candidates_follow_history_modes() {
        let history = vec![
            HistoryEntry {
                t: 1,
                command: "one".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "two".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];
        let sessions = vec![AiSession {
            id: "s1".to_string(),
            t: 3,
            prompt: "prompt".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                crate::history::AiItem {
                    kind: AiItemKind::Template,
                    text: "template body".to_string(),
                    name: Some("tpl".to_string()),
                },
                crate::history::AiItem {
                    kind: AiItemKind::Command,
                    text: "ai command".to_string(),
                    name: None,
                },
            ],
        }];

        assert_eq!(
            regular_history_picker_candidates(&history),
            vec!["two", "one"]
        );
        assert_eq!(ai_history_picker_candidates(&sessions), vec!["ai command"]);
        assert_eq!(
            combined_history_picker_candidates(&history, &sessions),
            vec!["two", "one", "ai command"]
        );
    }

    #[test]
    fn template_picker_candidates_return_newest_unique_names() {
        let templates = vec![
            TemplateEntry {
                name: "deploy".to_string(),
                body: "old".to_string(),
            },
            TemplateEntry {
                name: "logs".to_string(),
                body: "tail".to_string(),
            },
            TemplateEntry {
                name: "deploy".to_string(),
                body: "new".to_string(),
            },
        ];

        assert_eq!(
            template_picker_candidates(&templates),
            vec!["deploy", "logs"]
        );
    }
}
