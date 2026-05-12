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
}
