use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenContext {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub is_first_token: bool,
    pub quote: Option<char>,
    pub path_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub display: String,
    pub replacement: String,
    pub is_dir: bool,
    pub source: CompletionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionSource {
    Path,
    Template,
    History,
    Executable,
    TemplatePlaceholder,
}

pub fn current_token_context(line: &str, cursor: usize) -> TokenContext {
    let cursor = cursor.min(line.len());
    let cursor = previous_char_boundary(line, cursor);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_start = 0;
    let mut token_seen = false;
    let mut token_before_current = false;

    for (index, ch) in line[..cursor].char_indices() {
        if escaped {
            escaped = false;
            token_seen = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => {
                quote = None;
                token_seen = true;
            }
            Some(_) => {
                if ch == '\\' && quote == Some('"') {
                    escaped = true;
                }
                token_seen = true;
            }
            None if ch == '\\' => {
                escaped = true;
                token_seen = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_seen = true;
            }
            None if ch.is_whitespace() => {
                if token_seen {
                    token_before_current = true;
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    let text = line[token_start..cursor].to_string();
    TokenContext {
        start: token_start,
        end: cursor,
        path_like: is_path_like_token(&text),
        text,
        is_first_token: !token_before_current,
        quote,
    }
}

pub fn is_path_like_token(token: &str) -> bool {
    let token = token.trim_start_matches(['\'', '"']);
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('~')
        || token.contains('/')
}

pub fn complete_path(token: &str, cwd: &Path) -> Vec<CompletionCandidate> {
    let (quote, token) = strip_opening_quote(token);
    let (dir_token, prefix) = split_path_token(token);
    let Some(search_dir) = resolve_search_dir(dir_token, cwd) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(search_dir) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if !file_name.starts_with(prefix) {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        let replacement = format!("{quote}{dir_token}{file_name}{suffix}");
        candidates.push(CompletionCandidate {
            display: format!("{dir_token}{file_name}{suffix}"),
            replacement,
            is_dir,
            source: CompletionSource::Path,
        });
    }
    candidates.sort_by(|left, right| left.display.cmp(&right.display));
    candidates
}

pub fn complete_first_token(
    prefix: &str,
    templates: &[TemplateEntry],
    history_newest_first: &[HistoryEntry],
    path_dirs: &[PathBuf],
) -> Vec<CompletionCandidate> {
    let mut candidates = Vec::new();
    let mut seen_templates = HashSet::new();
    for template in templates {
        if template.name.starts_with(prefix) && seen_templates.insert(template.name.as_str()) {
            candidates.push(CompletionCandidate {
                display: template.name.clone(),
                replacement: template.body.clone(),
                is_dir: false,
                source: CompletionSource::Template,
            });
        }
    }

    let mut seen_history = HashSet::new();
    for entry in history_newest_first {
        if entry.command.starts_with(prefix) && seen_history.insert(entry.command.as_str()) {
            candidates.push(CompletionCandidate {
                display: entry.command.clone(),
                replacement: entry.command.clone(),
                is_dir: false,
                source: CompletionSource::History,
            });
        }
    }

    let mut executable_candidates = complete_path_executables(prefix, path_dirs);
    candidates.append(&mut executable_candidates);
    candidates
}

pub fn complete_non_first_token(
    token: &str,
    cwd: &Path,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
) -> Vec<CompletionCandidate> {
    let mut candidates = Vec::new();
    if is_path_like_token(token) {
        candidates.extend(complete_path(token, cwd));
    }
    candidates.extend(complete_history_arguments(token, history_newest_first));
    candidates.extend(complete_template_placeholders(token, templates));
    candidates
}

fn complete_history_arguments(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for entry in history_newest_first {
        for argument in command_arguments(&entry.command) {
            if argument.starts_with(prefix) && seen.insert(argument.to_string()) {
                candidates.push(CompletionCandidate {
                    display: argument.to_string(),
                    replacement: argument.to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                });
            }
        }
    }
    candidates
}

fn complete_template_placeholders(
    prefix: &str,
    templates: &[TemplateEntry],
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for template in templates {
        for placeholder in crate::templates::template_placeholders(&template.body) {
            if placeholder.starts_with(prefix) && seen.insert(placeholder.clone()) {
                candidates.push(CompletionCandidate {
                    display: placeholder.clone(),
                    replacement: placeholder,
                    is_dir: false,
                    source: CompletionSource::TemplatePlaceholder,
                });
            }
        }
    }
    candidates
}

fn command_arguments(command: &str) -> Vec<&str> {
    let mut arguments = Vec::new();
    let mut token_start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_index = 0;
    let mut token_seen = false;

    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            token_seen = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => {
                quote = None;
                token_seen = true;
            }
            Some(_) => {
                if ch == '\\' && quote == Some('"') {
                    escaped = true;
                }
                token_seen = true;
            }
            None if ch == '\\' => {
                escaped = true;
                token_seen = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_seen = true;
            }
            None if ch.is_whitespace() => {
                if token_seen {
                    if token_index > 0 {
                        arguments.push(command[token_start..index].trim_matches(['\'', '"']));
                    }
                    token_index += 1;
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    if token_seen && token_index > 0 {
        arguments.push(command[token_start..].trim_matches(['\'', '"']));
    }
    arguments
}

fn complete_path_executables(prefix: &str, path_dirs: &[PathBuf]) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for dir in path_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            if !file_name.starts_with(prefix) || !seen.insert(file_name.clone()) {
                continue;
            }
            let path = entry.path();
            if !is_executable_file(&path) {
                continue;
            }
            candidates.push(CompletionCandidate {
                display: file_name.clone(),
                replacement: file_name,
                is_dir: false,
                source: CompletionSource::Executable,
            });
        }
    }
    candidates.sort_by(|left, right| left.display.cmp(&right.display));
    candidates
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn strip_opening_quote(token: &str) -> (&str, &str) {
    if let Some(rest) = token.strip_prefix('\'') {
        ("'", rest)
    } else if let Some(rest) = token.strip_prefix('"') {
        ("\"", rest)
    } else {
        ("", token)
    }
}

fn split_path_token(token: &str) -> (&str, &str) {
    match token.rsplit_once('/') {
        Some((dir, prefix)) => (&token[..dir.len() + 1], prefix),
        None => ("", token),
    }
}

fn resolve_search_dir(dir_token: &str, cwd: &Path) -> Option<PathBuf> {
    if dir_token.is_empty() {
        return Some(cwd.to_path_buf());
    }
    if dir_token == "~/" || dir_token.starts_with("~/") {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        return Some(home.join(&dir_token[2..]));
    }
    let path = Path::new(dir_token);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        Some(cwd.join(path))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_token_detects_first_token_prefix() {
        assert_eq!(
            current_token_context("git sta", 3),
            TokenContext {
                start: 0,
                end: 3,
                text: "git".to_string(),
                is_first_token: true,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_detects_non_first_token_at_cursor() {
        assert_eq!(
            current_token_context("git sta", 7),
            TokenContext {
                start: 4,
                end: 7,
                text: "sta".to_string(),
                is_first_token: false,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_keeps_quoted_whitespace_inside_token() {
        assert_eq!(
            current_token_context("echo \"hello wo", 14),
            TokenContext {
                start: 5,
                end: 14,
                text: "\"hello wo".to_string(),
                is_first_token: false,
                quote: Some('"'),
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_keeps_escaped_whitespace_inside_token() {
        assert_eq!(
            current_token_context("cd my\\ dir/fi", 13),
            TokenContext {
                start: 3,
                end: 13,
                text: "my\\ dir/fi".to_string(),
                is_first_token: false,
                quote: None,
                path_like: true,
            }
        );
    }

    #[test]
    fn current_token_handles_cursor_inside_line() {
        assert_eq!(
            current_token_context("git checkout main", 12),
            TokenContext {
                start: 4,
                end: 12,
                text: "checkout".to_string(),
                is_first_token: false,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn path_like_detection_covers_common_shell_path_prefixes() {
        for token in ["/tmp", "./src", "../src", "~/src", "src/main.rs", "'./src"] {
            assert!(is_path_like_token(token), "{token:?} should be path-like");
        }
        for token in ["git", "status", "--flag"] {
            assert!(
                !is_path_like_token(token),
                "{token:?} should not be path-like"
            );
        }
    }

    #[test]
    fn complete_path_returns_sorted_matching_file_and_directory_candidates() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("alpha.txt"), "").unwrap();
        std::fs::create_dir(temp.path().join("app")).unwrap();
        std::fs::write(temp.path().join("beta.txt"), "").unwrap();

        assert_eq!(
            complete_path("a", temp.path()),
            [
                CompletionCandidate {
                    display: "alpha.txt".to_string(),
                    replacement: "alpha.txt".to_string(),
                    is_dir: false,
                    source: CompletionSource::Path,
                },
                CompletionCandidate {
                    display: "app/".to_string(),
                    replacement: "app/".to_string(),
                    is_dir: true,
                    source: CompletionSource::Path,
                },
            ]
        );
    }

    #[test]
    fn complete_path_uses_relative_directory_prefix() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        assert_eq!(
            complete_path("src/m", temp.path()),
            [CompletionCandidate {
                display: "src/main.rs".to_string(),
                replacement: "src/main.rs".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            }]
        );
    }

    #[test]
    fn complete_path_preserves_opening_quote_in_replacement_only() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("my file.txt"), "").unwrap();

        assert_eq!(
            complete_path("'my", temp.path()),
            [CompletionCandidate {
                display: "my file.txt".to_string(),
                replacement: "'my file.txt".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            }]
        );
    }

    #[test]
    fn complete_first_token_orders_templates_history_then_executables() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("git-now");
        std::fs::write(&executable, "#!/bin/sh\n").unwrap();
        make_executable(&executable);
        let templates = vec![TemplateEntry {
            name: "git-save".to_string(),
            body: "git add . && git commit".to_string(),
        }];
        let history = vec![HistoryEntry {
            t: 2,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        assert_eq!(
            complete_first_token("git", &templates, &history, &[bin]),
            [
                CompletionCandidate {
                    display: "git-save".to_string(),
                    replacement: "git add . && git commit".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "git status".to_string(),
                    replacement: "git status".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "git-now".to_string(),
                    replacement: "git-now".to_string(),
                    is_dir: false,
                    source: CompletionSource::Executable,
                },
            ]
        );
    }

    #[test]
    fn complete_first_token_deduplicates_each_source() {
        let templates = vec![
            TemplateEntry {
                name: "deploy".to_string(),
                body: "old".to_string(),
            },
            TemplateEntry {
                name: "deploy".to_string(),
                body: "new".to_string(),
            },
        ];
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "docker ps".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "docker ps".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        assert_eq!(
            complete_first_token("d", &templates, &history, &[]),
            [
                CompletionCandidate {
                    display: "deploy".to_string(),
                    replacement: "old".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "docker ps".to_string(),
                    replacement: "docker ps".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
            ]
        );
    }

    #[test]
    fn complete_non_first_token_orders_path_candidates_before_history_arguments() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        let history = vec![HistoryEntry {
            t: 2,
            command: "git add src/lib.rs".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        assert_eq!(
            complete_non_first_token("src/", temp.path(), &history, &[]),
            [
                CompletionCandidate {
                    display: "src/main.rs".to_string(),
                    replacement: "src/main.rs".to_string(),
                    is_dir: false,
                    source: CompletionSource::Path,
                },
                CompletionCandidate {
                    display: "src/lib.rs".to_string(),
                    replacement: "src/lib.rs".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
            ]
        );
    }

    #[test]
    fn complete_non_first_token_includes_history_arguments_without_path_prefix() {
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "kubectl get pods".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "docker get pods".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        let templates = vec![TemplateEntry {
            name: "logs".to_string(),
            body: "kubectl logs {pod_name}".to_string(),
        }];

        assert_eq!(
            complete_non_first_token("po", Path::new("/"), &history, &templates),
            [
                CompletionCandidate {
                    display: "pods".to_string(),
                    replacement: "pods".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "pod_name".to_string(),
                    replacement: "pod_name".to_string(),
                    is_dir: false,
                    source: CompletionSource::TemplatePlaceholder,
                }
            ]
        );
    }

    #[test]
    fn command_arguments_preserve_quoted_argument_spaces() {
        assert_eq!(
            command_arguments("git commit -m 'hello world' -- file"),
            ["commit", "-m", "hello world", "--", "file"]
        );
    }

    #[test]
    fn complete_path_returns_empty_for_missing_directory() {
        let temp = tempfile::tempdir().unwrap();

        assert!(complete_path("missing/file", temp.path()).is_empty());
    }

    #[test]
    fn cursor_is_snapped_to_previous_utf8_boundary() {
        assert_eq!(current_token_context("echo λ", 6).end, 5);
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}
}
