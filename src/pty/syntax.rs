pub(super) fn is_incomplete_shell_syntax(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("unexpected eof")
        || stderr.contains("unexpected end of file")
        || stderr.contains("unmatched \"")
        || stderr.contains("unmatched '")
        || stderr.contains("parse error near `\\n'")
        || stderr.contains("parse error near `\n'")
        || stderr.contains("parse error: unmatched")
}

pub(super) fn shell_continuation_prompt(stderr: &str) -> Option<String> {
    let stderr = stderr.to_ascii_lowercase();
    if stderr.contains("unmatched \"") || stderr.contains("matching `\"'") {
        return Some("dquote> ".to_string());
    }
    if stderr.contains("unmatched '") || stderr.contains("matching `''") {
        return Some("quote> ".to_string());
    }
    if is_incomplete_shell_syntax(&stderr) {
        return Some("> ".to_string());
    }
    None
}

pub(super) fn ends_with_shell_line_continuation(input: &str) -> bool {
    let trailing_backslashes = input
        .as_bytes()
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count();
    trailing_backslashes % 2 == 1
}
