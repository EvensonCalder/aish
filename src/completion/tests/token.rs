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
