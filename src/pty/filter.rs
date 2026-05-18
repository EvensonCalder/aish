use super::{ready_marker, start_marker};

pub(super) struct PtyOutputFilter {
    marker: String,
    pending: Vec<u8>,
    deferred_separator: Vec<u8>,
    fish: Option<FishOutputFilter>,
    command_complete: bool,
    ready_completes_command: bool,
}

struct FishOutputFilter {
    command_active: bool,
    started_command: Option<String>,
    held_segment: Option<FishHeldSegment>,
}

struct FishHeldSegment {
    bytes: Vec<u8>,
    visible: String,
}

enum InternalMarker {
    Start(String),
    Ready,
    Status,
}

impl PtyOutputFilter {
    pub(super) fn marker(marker: &str) -> Self {
        Self {
            marker: marker.to_string(),
            pending: Vec::new(),
            deferred_separator: Vec::new(),
            fish: None,
            command_complete: false,
            ready_completes_command: false,
        }
    }

    pub(super) fn shell_events(filter_fish_repaint: bool) -> Self {
        Self {
            marker: String::new(),
            pending: Vec::new(),
            deferred_separator: Vec::new(),
            fish: filter_fish_repaint.then_some(FishOutputFilter {
                command_active: false,
                started_command: None,
                held_segment: None,
            }),
            command_complete: false,
            ready_completes_command: true,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some(end) = next_terminal_line_end(&self.pending) {
            let segment: Vec<u8> = self.pending.drain(..end).collect();
            self.push_segment(segment, &mut output);
        }
        output
    }

    pub(super) fn flush_pending(&mut self) -> Vec<u8> {
        if self.pending.is_empty() || self.command_complete || self.pending_may_be_internal_marker()
        {
            return Vec::new();
        }
        let segment = std::mem::take(&mut self.pending);
        let mut output = Vec::new();
        self.push_segment(segment, &mut output);
        output
    }

    fn push_segment(&mut self, segment: Vec<u8>, output: &mut Vec<u8>) {
        if self.command_complete {
            self.deferred_separator.clear();
            return;
        }
        if let Some(marker) = self.internal_marker_segment(&segment) {
            self.handle_internal_marker(marker, output);
            return;
        }
        if self.fish.as_ref().is_some_and(|fish| !fish.command_active) {
            self.deferred_separator.clear();
            return;
        }
        if terminal_separator_only(&segment) {
            if self
                .fish
                .as_ref()
                .is_some_and(|fish| fish.held_segment.is_some())
            {
                self.deferred_separator.extend_from_slice(&segment);
            } else {
                self.push_deferred_separator(output);
                self.deferred_separator = segment;
            }
            return;
        }
        let Some(segment) = self.filter_fish_segment(segment, output) else {
            return;
        };
        if !self.deferred_separator.is_empty() {
            output.extend_from_slice(&self.deferred_separator);
            self.deferred_separator.clear();
        }
        output.extend_from_slice(&segment);
    }

    fn internal_marker_segment(&self, segment: &[u8]) -> Option<InternalMarker> {
        let text = String::from_utf8_lossy(segment);
        let cleaned = strip_terminal_control_sequences(&text);
        let line = cleaned
            .trim_matches(['\r', '\n'])
            .trim_start_matches([' ', '\t']);
        let start_marker = start_marker();
        let ready_marker = ready_marker();
        if let Some(command) = line.strip_prefix(&format!("{start_marker}\t")) {
            return Some(InternalMarker::Start(command.trim_end().to_string()));
        }
        if line.starts_with(start_marker) {
            return Some(InternalMarker::Start(String::new()));
        }
        if line.starts_with(ready_marker) {
            return Some(InternalMarker::Ready);
        }
        if !self.marker.is_empty() && line.starts_with(&self.marker) {
            return Some(InternalMarker::Status);
        }
        None
    }

    fn pending_may_be_internal_marker(&self) -> bool {
        let text = String::from_utf8_lossy(&self.pending);
        let cleaned = strip_terminal_control_sequences(&text);
        let line = cleaned.trim_start_matches([' ', '\t']);
        line_starts_or_could_be_prefix(line, start_marker())
            || line_starts_or_could_be_prefix(line, ready_marker())
            || !self.marker.is_empty() && line_starts_or_could_be_prefix(line, &self.marker)
    }

    fn handle_internal_marker(&mut self, marker: InternalMarker, output: &mut Vec<u8>) {
        match marker {
            InternalMarker::Start(command) => {
                if let Some(fish) = &mut self.fish {
                    fish.command_active = true;
                    fish.started_command = Some(command);
                    fish.held_segment = None;
                }
            }
            InternalMarker::Ready => {
                self.flush_fish_held_segment(output);
                if let Some(fish) = &mut self.fish {
                    fish.command_active = false;
                    fish.held_segment = None;
                }
                if self.ready_completes_command {
                    self.command_complete = true;
                }
            }
            InternalMarker::Status => {
                self.command_complete = true;
            }
        }
        self.deferred_separator.clear();
    }

    fn filter_fish_segment(&mut self, segment: Vec<u8>, output: &mut Vec<u8>) -> Option<Vec<u8>> {
        if self.fish.is_none() {
            return Some(segment);
        }

        let text = String::from_utf8_lossy(&segment);
        let has_repaint_control = contains_terminal_repaint_control(&text);
        let visible = strip_terminal_control_sequences(&text).trim().to_string();

        if visible.starts_with('\u{23ce}') || visible.is_empty() && has_repaint_control {
            return None;
        }

        let started_command = self
            .fish
            .as_ref()
            .and_then(|fish| fish.started_command.as_deref());
        if let Some(command) = started_command
            && is_fish_repaint_echo_fragment(&visible, command, has_repaint_control)
        {
            return None;
        }

        if let Some(held) = self.fish.as_mut().and_then(|fish| fish.held_segment.take()) {
            if held.visible == visible && !has_repaint_control {
                self.deferred_separator.clear();
            } else {
                output.extend_from_slice(&held.bytes);
                self.push_deferred_separator(output);
            }
        }

        if has_repaint_control && !visible.is_empty() {
            if let Some(fish) = &mut self.fish {
                fish.held_segment = Some(FishHeldSegment {
                    bytes: segment,
                    visible,
                });
            }
            return None;
        }

        Some(segment)
    }

    fn flush_fish_held_segment(&mut self, output: &mut Vec<u8>) {
        let held = self.fish.as_mut().and_then(|fish| fish.held_segment.take());
        let Some(held) = held else {
            return;
        };
        let started_command = self
            .fish
            .as_ref()
            .and_then(|fish| fish.started_command.as_deref());
        if let Some(command) = started_command
            && is_fish_command_repaint_token(&held.visible, command)
        {
            self.deferred_separator.clear();
            return;
        }
        output.extend_from_slice(&held.bytes);
        self.push_deferred_separator(output);
    }

    fn push_deferred_separator(&mut self, output: &mut Vec<u8>) {
        if !self.deferred_separator.is_empty() {
            output.extend_from_slice(&self.deferred_separator);
            self.deferred_separator.clear();
        }
    }
}

fn next_terminal_line_end(bytes: &[u8]) -> Option<usize> {
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' => return Some(index + 1),
            b'\r' => {
                if index + 1 >= bytes.len() {
                    return None;
                }
                if bytes[index + 1] == b'\n' {
                    return Some(index + 2);
                }
                return Some(index + 1);
            }
            _ => {}
        }
    }
    None
}

fn terminal_separator_only(segment: &[u8]) -> bool {
    segment.iter().all(|byte| matches!(*byte, b'\r' | b'\n'))
}

fn line_starts_or_could_be_prefix(line: &str, marker: &str) -> bool {
    line.starts_with(marker) || marker.starts_with(line)
}

pub(super) fn clean_fish_repaint_lines(
    output_lines: Vec<String>,
    started_command: Option<&str>,
) -> Vec<String> {
    let lines: Vec<(String, bool)> = output_lines
        .into_iter()
        .filter_map(|line| {
            let had_terminal_control = contains_terminal_control(&line);
            let cleaned = strip_terminal_control_sequences(&line);
            let visible = cleaned.trim();
            if visible.starts_with('\u{23ce}') {
                return None;
            }
            if visible.is_empty() && had_terminal_control {
                return None;
            }
            if let Some(command) = started_command
                && is_fish_repaint_echo_fragment(visible, command, had_terminal_control)
            {
                return None;
            }
            Some((cleaned.trim_end().to_string(), had_terminal_control))
        })
        .collect();

    let mut deduped = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index].0;
        let mut end = index + 1;
        while end < lines.len() && lines[end].0 == *line {
            end += 1;
        }
        let run = &lines[index..end];
        if run.len() == 1 {
            deduped.push(line.clone());
        } else {
            let normal_lines: Vec<String> = run
                .iter()
                .filter(|(_, had_terminal_control)| !*had_terminal_control)
                .map(|(line, _)| line.clone())
                .collect();
            if normal_lines.is_empty() {
                deduped.push(line.clone());
            } else {
                deduped.extend(normal_lines);
            }
        }
        index = end;
    }
    deduped
}

fn is_fish_repaint_echo_fragment(line: &str, command: &str, had_terminal_control: bool) -> bool {
    if line.is_empty() {
        return false;
    }
    let line = line.trim_start_matches(['>', '=']).trim_start().trim_end();
    if line.is_empty() {
        return true;
    }
    line == command
        || command.starts_with(line)
        || shell_syntax_fragment(line)
            && (command.ends_with(line) || command_contains_repaint_token(command, line))
        || had_terminal_control && command_contains_repaint_token(command, line)
}

fn command_contains_repaint_token(command: &str, line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    if line
        .chars()
        .all(|ch| matches!(ch, ';' | '|' | '&' | '<' | '>'))
    {
        return command.contains(line);
    }
    let line = line.trim_matches(['\'', '"', ';']);
    let tokens: Vec<&str> = command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>'))
        .map(|part| part.trim_matches(['\'', '"']))
        .filter(|part| !part.is_empty())
        .collect();
    if !line.contains(['\'', '"', '\\', '$', ';', '|', '&', '<', '>']) {
        return tokens
            .iter()
            .position(|part| *part == line)
            .is_some_and(|index| index + 1 < tokens.len());
    }
    tokens.contains(&line)
}

fn is_fish_command_repaint_token(line: &str, command: &str) -> bool {
    let line = line.trim_start_matches(['>', '=']).trim_start().trim_end();
    if line.is_empty() || line == command || command.starts_with(line) {
        return true;
    }
    let line = line.trim_matches(['\'', '"', ';']);
    if line.is_empty() {
        return true;
    }
    command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>'))
        .map(|part| part.trim_matches(['\'', '"', ';']))
        .filter(|part| !part.is_empty())
        .any(|part| part == line)
}

fn shell_syntax_fragment(line: &str) -> bool {
    line.contains(['\'', '"', '\\', '$', ';', '|', '&', '<', '>'])
}

fn contains_terminal_control(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .any(|byte| *byte == 0x1b || *byte < 0x20 && *byte != b'\t')
}

fn contains_terminal_repaint_control(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index >= bytes.len() {
                    return true;
                }
                match bytes[index] {
                    b'[' => {
                        index += 1;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                if matches!(
                                    byte,
                                    b'A' | b'B'
                                        | b'C'
                                        | b'D'
                                        | b'E'
                                        | b'F'
                                        | b'G'
                                        | b'H'
                                        | b'J'
                                        | b'K'
                                        | b'S'
                                        | b'T'
                                        | b'f'
                                ) {
                                    return true;
                                }
                                break;
                            }
                        }
                    }
                    b']' | b'P' | b'^' | b'_' => return true,
                    b'(' | b')' | b'*' | b'+' => {
                        index = (index + 2).min(bytes.len());
                    }
                    _ => return true,
                }
            }
            0x08 => return true,
            byte if byte < 0x20 && !matches!(byte, b'\t' | b'\r' | b'\n') => return true,
            _ => index += 1,
        }
    }
    false
}

pub(super) fn strip_terminal_control_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index >= bytes.len() {
                    break;
                }
                match bytes[index] {
                    b'[' => {
                        index += 1;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                break;
                            }
                        }
                    }
                    b']' | b'P' | b'^' | b'_' => {
                        index += 1;
                        while index < bytes.len() {
                            if bytes[index] == 0x07 {
                                index += 1;
                                break;
                            }
                            if bytes[index] == 0x1b
                                && index + 1 < bytes.len()
                                && bytes[index + 1] == b'\\'
                            {
                                index += 2;
                                break;
                            }
                            index += 1;
                        }
                    }
                    b'(' | b')' | b'*' | b'+' => {
                        index = (index + 2).min(bytes.len());
                    }
                    _ => {
                        index += 1;
                    }
                }
            }
            byte if byte < 0x20 && byte != b'\t' => {
                index += 1;
            }
            _ => {
                let Some(ch) = text[index..].chars().next() else {
                    break;
                };
                output.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    output
}
