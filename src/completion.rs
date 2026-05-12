#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenContext {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub is_first_token: bool,
    pub quote: Option<char>,
    pub path_like: bool,
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
    fn cursor_is_snapped_to_previous_utf8_boundary() {
        assert_eq!(current_token_context("echo λ", 6).end, 5);
    }
}
