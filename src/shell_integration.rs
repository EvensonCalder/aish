const INTERACTIVE_ALLOWLIST: &[&str] = &[
    "vi", "vim", "nvim", "nano", "emacs", "hx", "helix", "kak", "less", "more", "man", "top",
    "htop", "btop", "ssh", "mosh", "fzf", "tmux", "screen",
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
}
