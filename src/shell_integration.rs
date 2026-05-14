const INTERACTIVE_ALLOWLIST: &[&str] = &[
    "vi", "vim", "nvim", "nano", "emacs", "hx", "helix", "kak", "less", "more", "man", "top",
    "htop", "btop", "ssh", "mosh", "fzf", "tmux", "screen", "python", "python3", "node", "ipython",
    "psql",
];

pub fn is_interactive_passthrough_command(command: &str) -> bool {
    interactive_command_name(command).is_some()
}

pub fn interactive_command_name(command: &str) -> Option<String> {
    let words = shell_words(command);
    let mut index = skip_assignments(&words, 0);
    loop {
        let word = words.get(index)?;
        match basename(word).as_str() {
            "env" => {
                index = skip_env_prefix(&words, index + 1);
            }
            "sudo" | "doas" => {
                index = skip_sudo_prefix(&words, index + 1);
            }
            "command" | "exec" => {
                index = skip_simple_wrapper(&words, index + 1);
            }
            name if INTERACTIVE_ALLOWLIST.contains(&name) => return Some(name.to_string()),
            _ => return None,
        }
    }
}

pub fn alternate_screen_active_after(output: &str, initially_active: bool) -> bool {
    let mut active = initially_active;
    for event in alternate_screen_events(output) {
        active = event == AlternateScreenEvent::Enter;
    }
    active
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassthroughReturnState {
    pub alternate_screen_active: bool,
    pub prompt_returned: bool,
}

pub fn passthrough_return_state_after(
    output: &str,
    initially_alternate_screen_active: bool,
    process_exited: bool,
) -> PassthroughReturnState {
    let alternate_screen_active =
        alternate_screen_active_after(output, initially_alternate_screen_active);
    PassthroughReturnState {
        alternate_screen_active,
        prompt_returned: process_exited && !alternate_screen_active,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlternateScreenEvent {
    Enter,
    Exit,
}

fn alternate_screen_events(output: &str) -> Vec<AlternateScreenEvent> {
    let mut events = Vec::new();
    let bytes = output.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == 0x1b && bytes[index + 1] == b'[' && bytes[index + 2] == b'?' {
            let mut end = index + 3;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end < bytes.len() {
                let code = &output[index + 3..end];
                let event = match (code, bytes[end]) {
                    ("47" | "1047" | "1049", b'h') => Some(AlternateScreenEvent::Enter),
                    ("47" | "1047" | "1049", b'l') => Some(AlternateScreenEvent::Exit),
                    _ => None,
                };
                if let Some(event) = event {
                    events.push(event);
                }
            }
            index = end.saturating_add(1);
        } else {
            index += 1;
        }
    }
    events
}

fn skip_assignments(words: &[String], mut index: usize) -> usize {
    while words.get(index).is_some_and(|word| is_assignment(word)) {
        index += 1;
    }
    index
}

fn skip_env_prefix(words: &[String], mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        if word.starts_with('-') || is_assignment(word) {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn skip_sudo_prefix(words: &[String], mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        if !word.starts_with('-') {
            break;
        }
        let consumes_next = matches!(word.as_str(), "-u" | "-g" | "-h" | "-p" | "-C" | "-T");
        index += 1;
        if consumes_next && words.get(index).is_some() {
            index += 1;
        }
    }
    skip_assignments(words, index)
}

fn skip_simple_wrapper(words: &[String], mut index: usize) -> usize {
    while words.get(index).is_some_and(|word| word.starts_with('-')) {
        index += 1;
    }
    skip_assignments(words, index)
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some('_') | Some('a'..='z') | Some('A'..='Z'))
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn basename(word: &str) -> String {
    word.rsplit('/').next().unwrap_or(word).to_string()
}

fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '#') => break,
            (None, ch) if ch.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), ch) if ch == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (_, ch) => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_allowlist_matches_common_fullscreen_commands() {
        for command in [
            "vim file",
            "nvim",
            "ssh host",
            "top",
            "less README.md",
            "fzf",
            "python3",
            "node",
        ] {
            assert!(is_interactive_passthrough_command(command), "{command}");
        }
    }

    #[test]
    fn interactive_allowlist_uses_basename_and_shell_words() {
        assert_eq!(
            interactive_command_name("/usr/bin/vim 'my file.txt'"),
            Some("vim".to_string())
        );
        assert_eq!(
            interactive_command_name("\"/opt/bin/nvim\" +q"),
            Some("nvim".to_string())
        );
    }

    #[test]
    fn interactive_allowlist_skips_common_wrappers() {
        assert_eq!(
            interactive_command_name("TERM=xterm-256color sudo -E vim file"),
            Some("vim".to_string())
        );
        assert_eq!(
            interactive_command_name("env -i TERM=xterm less README.md"),
            Some("less".to_string())
        );
        assert_eq!(
            interactive_command_name("command -p ssh example.com"),
            Some("ssh".to_string())
        );
        assert_eq!(
            interactive_command_name("exec nvim"),
            Some("nvim".to_string())
        );
    }

    #[test]
    fn interactive_allowlist_does_not_match_noninteractive_commands() {
        for command in [
            "echo vim",
            "cat README.md",
            "git status",
            "printf 'less'",
            "VAR=value",
        ] {
            assert_eq!(interactive_command_name(command), None, "{command}");
        }
    }

    #[test]
    fn alternate_screen_detection_tracks_common_enter_and_exit_sequences() {
        assert!(alternate_screen_active_after(
            "before\x1b[?1049hafter",
            false
        ));
        assert!(!alternate_screen_active_after(
            "before\x1b[?1049lafter",
            true
        ));
        assert!(alternate_screen_active_after("\x1b[?47h", false));
        assert!(!alternate_screen_active_after("\x1b[?1047l", true));
    }

    #[test]
    fn alternate_screen_detection_uses_last_seen_event() {
        assert!(!alternate_screen_active_after(
            "\x1b[?1049hbody\x1b[?1049l",
            false
        ));
        assert!(alternate_screen_active_after(
            "\x1b[?1049lexit\x1b[?1049h",
            false
        ));
    }

    #[test]
    fn alternate_screen_detection_ignores_unrelated_escape_sequences() {
        assert!(!alternate_screen_active_after("\x1b[31mred\x1b[0m", false));
        assert!(alternate_screen_active_after("\x1b[31mred\x1b[0m", true));
    }

    #[test]
    fn passthrough_return_detection_requires_process_exit_and_normal_screen() {
        assert_eq!(
            passthrough_return_state_after("", false, true),
            PassthroughReturnState {
                alternate_screen_active: false,
                prompt_returned: true
            }
        );
        assert_eq!(
            passthrough_return_state_after("", false, false),
            PassthroughReturnState {
                alternate_screen_active: false,
                prompt_returned: false
            }
        );
        assert_eq!(
            passthrough_return_state_after("\x1b[?1049h", false, true),
            PassthroughReturnState {
                alternate_screen_active: true,
                prompt_returned: false
            }
        );
    }

    #[test]
    fn passthrough_return_detection_accepts_alternate_screen_exit_before_process_exit() {
        assert_eq!(
            passthrough_return_state_after("\x1b[?1049hbody\x1b[?1049l", false, true),
            PassthroughReturnState {
                alternate_screen_active: false,
                prompt_returned: true
            }
        );
    }
}
