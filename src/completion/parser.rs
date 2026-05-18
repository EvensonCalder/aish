use std::path::{Path, PathBuf};

use super::TokenContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellWord {
    pub(crate) raw: String,
    pub(crate) value: String,
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

#[cfg(test)]
pub(crate) fn command_arguments(command: &str) -> Vec<String> {
    command_word_ranges(command)
        .into_iter()
        .skip(1)
        .map(|(start, end)| shell_word_value(&command[start..end]))
        .collect()
}

pub(crate) fn split_shell_like_words(command: &str) -> Vec<String> {
    shell_like_words(command)
        .into_iter()
        .map(|word| word.value)
        .collect()
}

pub(crate) fn shell_like_words(command: &str) -> Vec<ShellWord> {
    command_word_ranges(command)
        .into_iter()
        .map(|(start, end)| {
            let raw = command[start..end].to_string();
            ShellWord {
                value: shell_word_value(&raw),
                raw,
            }
        })
        .collect()
}

pub(crate) fn command_argument_words(command: &str) -> Vec<ShellWord> {
    shell_like_words(command).into_iter().skip(1).collect()
}

fn command_word_ranges(command: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut token_start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
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
                    ranges.push((token_start, index));
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    if token_seen {
        ranges.push((token_start, command.len()));
    }
    ranges
}

pub(crate) fn shell_word_value(raw: &str) -> String {
    let mut value = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') if ch == '\'' => {
                quote = None;
            }
            Some('\'') => {
                value.push(ch);
            }
            Some('"') if ch == '"' => {
                quote = None;
            }
            Some('"') if ch == '\\' => match chars.peek().copied() {
                Some('$' | '`' | '"' | '\\') => {
                    value.push(chars.next().unwrap());
                }
                Some('\n') => {
                    chars.next();
                }
                _ => {
                    value.push(ch);
                }
            },
            Some('"') => {
                value.push(ch);
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
            }
            None if ch == '\\' => match chars.next() {
                Some('\n') => {}
                Some(next) => value.push(next),
                None => value.push(ch),
            },
            None => {
                value.push(ch);
            }
            Some(_) => {
                value.push(ch);
            }
        }
    }

    value
}

pub(crate) fn strip_opening_quote(token: &str) -> (&str, &str) {
    if let Some(rest) = token.strip_prefix('\'') {
        ("'", rest)
    } else if let Some(rest) = token.strip_prefix('"') {
        ("\"", rest)
    } else {
        ("", token)
    }
}

pub(crate) fn split_path_token(token: &str) -> (&str, &str) {
    match token.rsplit_once('/') {
        Some((dir, prefix)) => (&token[..dir.len() + 1], prefix),
        None => ("", token),
    }
}

pub(crate) fn resolve_search_dir(dir_token: &str, cwd: &Path) -> Option<PathBuf> {
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

pub(crate) fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    if text.is_char_boundary(cursor) {
        return cursor;
    }
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}
