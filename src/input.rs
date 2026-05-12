#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputBuffer {
    text: String,
    cursor: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn insert_str(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn backspace(&mut self) -> bool {
        let Some(prev) = self.previous_boundary() else {
            return false;
        };
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        let next = self.next_boundary().unwrap_or(self.text.len());
        self.text.drain(self.cursor..next);
        true
    }

    pub fn drain_range(&mut self, start: usize, end: usize) -> bool {
        if start > end
            || end > self.text.len()
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return false;
        }
        self.text.drain(start..end);
        self.cursor = start;
        true
    }

    pub fn move_left(&mut self) -> bool {
        let Some(prev) = self.previous_boundary() else {
            return false;
        };
        self.cursor = prev;
        true
    }

    pub fn move_right(&mut self) -> bool {
        let Some(next) = self.next_boundary() else {
            return false;
        };
        self.cursor = next;
        true
    }

    pub fn move_start(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn delete_to_start(&mut self) {
        self.text.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn delete_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    pub fn delete_previous_word(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        let end = self.cursor;
        while self.cursor > 0 {
            let prev = self.previous_boundary().expect("cursor is not at start");
            if !self.text[prev..self.cursor]
                .chars()
                .all(char::is_whitespace)
            {
                break;
            }
            self.cursor = prev;
        }
        while self.cursor > 0 {
            let prev = self.previous_boundary().expect("cursor is not at start");
            if self.text[prev..self.cursor]
                .chars()
                .all(char::is_whitespace)
            {
                break;
            }
            self.cursor = prev;
        }

        self.text.drain(self.cursor..end);
        true
    }

    pub fn move_previous_word(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        while self.cursor > 0 {
            let prev = self.previous_boundary().expect("cursor is not at start");
            self.cursor = prev;
            if !self.text[prev..].chars().next().unwrap().is_whitespace() {
                break;
            }
        }
        while self.cursor > 0 {
            let prev = self.previous_boundary().expect("cursor is not at start");
            if self.text[prev..self.cursor]
                .chars()
                .all(char::is_whitespace)
            {
                break;
            }
            self.cursor = prev;
        }
        true
    }

    pub fn move_next_word(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        while self.cursor < self.text.len() {
            let next = self.next_boundary().expect("cursor is not at end");
            let is_whitespace = self.text[self.cursor..next]
                .chars()
                .all(char::is_whitespace);
            self.cursor = next;
            if is_whitespace {
                break;
            }
        }
        while self.cursor < self.text.len() {
            let next = self.next_boundary().expect("cursor is not at end");
            if !self.text[self.cursor..next]
                .chars()
                .all(char::is_whitespace)
            {
                break;
            }
            self.cursor = next;
        }
        true
    }

    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(idx, _)| self.cursor + idx)
            .or_else(|| (self.cursor < self.text.len()).then_some(self.text.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_edits_in_middle() {
        let mut buffer = InputBuffer::from("git stus");
        buffer.move_left();
        buffer.move_left();
        buffer.insert_char('a');
        buffer.insert_char('t');
        assert_eq!(buffer.as_str(), "git status");
        assert_eq!(buffer.cursor(), 8);
    }

    #[test]
    fn backspace_and_delete_are_utf8_safe() {
        let mut buffer = InputBuffer::from("aλb");
        buffer.move_left();
        assert!(buffer.backspace());
        assert_eq!(buffer.as_str(), "ab");
        assert!(buffer.delete());
        assert_eq!(buffer.as_str(), "a");
    }

    #[test]
    fn control_deletion_matches_readline_basics() {
        let mut buffer = InputBuffer::from("cargo test --all");
        assert!(buffer.delete_previous_word());
        assert_eq!(buffer.as_str(), "cargo test ");
        buffer.delete_to_start();
        assert_eq!(buffer.as_str(), "");

        let mut buffer = InputBuffer::from("cargo test --all");
        buffer.move_previous_word();
        buffer.delete_to_end();
        assert_eq!(buffer.as_str(), "cargo test ");
    }

    #[test]
    fn drain_range_removes_byte_span_and_moves_cursor_to_start() {
        let mut buffer = InputBuffer::from("echo {name} now");

        assert!(buffer.drain_range(5, 12));

        assert_eq!(buffer.as_str(), "echo now");
        assert_eq!(buffer.cursor(), 5);
    }

    #[test]
    fn word_navigation_skips_tokens() {
        let mut buffer = InputBuffer::from("git commit -m test");
        buffer.move_start();
        assert!(buffer.move_next_word());
        assert_eq!(buffer.cursor(), 4);
        assert!(buffer.move_next_word());
        assert_eq!(buffer.cursor(), 11);
        assert!(buffer.move_previous_word());
        assert_eq!(buffer.cursor(), 4);
    }
}
