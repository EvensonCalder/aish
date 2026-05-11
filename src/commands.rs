#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLine<'a> {
    Ordinary(&'a str),
    Note { tag: NoteTag, text: &'a str },
    Private { name: &'a str, args: &'a str },
    AiPrompt(&'a str),
    AiPromptWithContext { prompt: &'a str, command: &'a str },
    EmptyPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteTag {
    Todo,
    Note,
    Fixme,
    Hack,
    Xxx,
}

pub fn parse_line(line: &str) -> ParsedLine<'_> {
    if !line.starts_with('#') {
        return ParsedLine::Ordinary(line);
    }

    let rest = &line[1..];
    if rest.trim().is_empty() {
        return ParsedLine::EmptyPrivate;
    }

    if let Some((tag, text)) = parse_note(rest) {
        return ParsedLine::Note { tag, text };
    }

    if let Some(prompt) = rest.strip_prefix(' ') {
        let prompt = prompt.trim();
        if let Some((prompt, command)) = parse_context_prompt(prompt) {
            return ParsedLine::AiPromptWithContext { prompt, command };
        }
        return ParsedLine::AiPrompt(prompt);
    }

    let trimmed = rest.trim_start();
    let split_at = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (name, args) = trimmed.split_at(split_at);
    ParsedLine::Private {
        name,
        args: args.trim_start(),
    }
}

fn parse_context_prompt(prompt: &str) -> Option<(&str, &str)> {
    let (prompt, command) = prompt.split_once('<')?;
    let prompt = prompt.trim();
    let command = command.trim();
    (!prompt.is_empty() && !command.is_empty()).then_some((prompt, command))
}

fn parse_note(rest: &str) -> Option<(NoteTag, &str)> {
    let trimmed = rest.trim_start();
    for (prefix, tag) in [
        ("TODO:", NoteTag::Todo),
        ("NOTE:", NoteTag::Note),
        ("FIXME:", NoteTag::Fixme),
        ("HACK:", NoteTag::Hack),
        ("XXX:", NoteTag::Xxx),
    ] {
        if let Some(text) = trimmed.strip_prefix(prefix) {
            return Some((tag, text.trim_start()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_input_is_not_private() {
        assert_eq!(parse_line("git status"), ParsedLine::Ordinary("git status"));
    }

    #[test]
    fn line_leading_hash_space_is_ai_prompt() {
        assert_eq!(
            parse_line("# how do I list files?"),
            ParsedLine::AiPrompt("how do I list files?")
        );
    }

    #[test]
    fn ai_prompt_with_context_command_is_detected() {
        assert_eq!(
            parse_line("# explain remotes < git -h && git remote -h"),
            ParsedLine::AiPromptWithContext {
                prompt: "explain remotes",
                command: "git -h && git remote -h"
            }
        );
    }

    #[test]
    fn incomplete_context_syntax_stays_plain_ai_prompt() {
        assert_eq!(
            parse_line("# explain remotes < "),
            ParsedLine::AiPrompt("explain remotes <")
        );
        assert_eq!(
            parse_line("# < git status"),
            ParsedLine::AiPrompt("< git status")
        );
    }

    #[test]
    fn private_command_allows_no_space_after_hash() {
        assert_eq!(
            parse_line("#model gpt-4.1"),
            ParsedLine::Private {
                name: "model",
                args: "gpt-4.1"
            }
        );
    }

    #[test]
    fn notes_are_detected_with_or_without_space_after_hash() {
        assert_eq!(
            parse_line("# TODO: deploy later"),
            ParsedLine::Note {
                tag: NoteTag::Todo,
                text: "deploy later"
            }
        );
        assert_eq!(
            parse_line("#TODO: deploy later"),
            ParsedLine::Note {
                tag: NoteTag::Todo,
                text: "deploy later"
            }
        );
    }
}
