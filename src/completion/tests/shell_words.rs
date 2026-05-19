use super::*;

#[test]
fn command_arguments_preserve_quoted_argument_spaces() {
    assert_eq!(
        command_arguments("git commit -m 'hello world' -- file"),
        ["commit", "-m", "hello world", "--", "file"]
    );
}

#[test]
fn shell_like_words_remove_quotes_and_escapes_for_matching() {
    assert_eq!(
        split_shell_like_words("cmd a\"b c\"d 'x y'z hello\\ world \"a\\\"b\" \"a\\ b\""),
        ["cmd", "ab cd", "x yz", "hello world", "a\"b", "a\\ b"]
    );
}

#[test]
fn shell_like_words_keep_source_spans_for_raw_replacement() {
    let command = "cmd a\"b c\"d hello\\ world";
    let words = parser::shell_like_words(command);

    assert_eq!(words.len(), 3);
    assert_eq!(words[1].raw, "a\"b c\"d");
    assert_eq!(words[1].value, "ab cd");
    assert_eq!(&command[words[1].start..words[1].end], words[1].raw);
    assert_eq!(words[2].raw, "hello\\ world");
    assert_eq!(words[2].value, "hello world");
    assert_eq!(&command[words[2].start..words[2].end], words[2].raw);
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
