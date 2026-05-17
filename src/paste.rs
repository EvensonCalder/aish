pub fn preview_lines(text: &str, max_lines: usize, max_bytes: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let max_bytes = max_bytes.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used_bytes = 0usize;
    let mut truncated = false;

    for ch in text.chars() {
        let len = ch.len_utf8();
        if used_bytes + len > max_bytes {
            truncated = true;
            break;
        }
        used_bytes += len;

        if ch == '\n' {
            lines.push(escape_preview_line(&current));
            current.clear();
            if lines.len() >= max_lines {
                truncated = used_bytes < text.len();
                break;
            }
        } else {
            current.push(ch);
        }
    }

    if lines.len() < max_lines && (!current.is_empty() || text.is_empty()) {
        lines.push(escape_preview_line(&current));
    } else if !current.is_empty() {
        truncated = true;
    }

    if truncated {
        lines.push("... preview truncated; Ctrl-X Ctrl-E opens the full paste".to_string());
    }

    lines
}

fn escape_preview_line(line: &str) -> String {
    let mut escaped = String::new();
    for ch in line.chars() {
        match ch {
            '\t' => escaped.push_str("\\t"),
            '\x1b' => escaped.push_str("\\x1b"),
            '\x7f' => escaped.push_str("\\x7f"),
            ch if ch.is_control() => {
                escaped.push_str("\\x");
                escaped.push(hex_digit((ch as u32 >> 4) & 0xf));
                escaped.push(hex_digit(ch as u32 & 0xf));
            }
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn hex_digit(value: u32) -> char {
    match value {
        0..=9 => char::from(b'0' + value as u8),
        10..=15 => char::from(b'a' + (value as u8 - 10)),
        _ => unreachable!("hex digit out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_lines_limits_lines_and_marks_truncation() {
        assert_eq!(
            preview_lines("one\ntwo\nthree", 2, 100),
            [
                "one".to_string(),
                "two".to_string(),
                "... preview truncated; Ctrl-X Ctrl-E opens the full paste".to_string()
            ]
        );
    }

    #[test]
    fn preview_lines_limits_bytes_at_char_boundaries() {
        assert_eq!(
            preview_lines("echo cafe\nnext", 5, 6),
            [
                "echo c".to_string(),
                "... preview truncated; Ctrl-X Ctrl-E opens the full paste".to_string()
            ]
        );
        assert_eq!(
            preview_lines("echo cafe\nnext", 5, 7),
            [
                "echo ca".to_string(),
                "... preview truncated; Ctrl-X Ctrl-E opens the full paste".to_string()
            ]
        );
    }

    #[test]
    fn preview_lines_escapes_control_characters() {
        assert_eq!(
            preview_lines("printf '\x1b[31m'\t# red", 3, 100),
            ["printf '\\x1b[31m'\\t# red".to_string()]
        );
    }
}
