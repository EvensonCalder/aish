use crate::commands::{HELP_TOPICS, IMPLEMENTED_PRIVATE_COMMANDS};

use super::parser::{current_token_context, previous_char_boundary, split_shell_like_words};
use super::{CompletionCandidate, CompletionSource, limit_candidates};

pub fn complete_private_commands(prefix: &str, max_results: usize) -> Vec<CompletionCandidate> {
    let Some(command_prefix) = prefix.strip_prefix('#') else {
        return Vec::new();
    };
    if command_prefix.is_empty() || command_prefix.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    let candidates = IMPLEMENTED_PRIVATE_COMMANDS
        .iter()
        .copied()
        .filter(|command| command.starts_with(command_prefix))
        .map(|command| CompletionCandidate {
            display: format!("#{command}"),
            replacement: format!("#{command}"),
            is_dir: false,
            source: CompletionSource::PrivateCommand,
        })
        .collect();
    limit_candidates(candidates, max_results)
}

pub fn complete_private_command_line(
    line: &str,
    cursor: usize,
    max_results: usize,
) -> Vec<CompletionCandidate> {
    let cursor = previous_char_boundary(line, cursor.min(line.len()));
    let before_cursor = &line[..cursor];
    let Some(rest) = before_cursor.strip_prefix('#') else {
        return Vec::new();
    };
    if rest.chars().next().is_some_and(char::is_whitespace) {
        return Vec::new();
    }

    let token = current_token_context(line, cursor);
    if token.is_first_token && token.text.starts_with('#') {
        return complete_private_commands(&token.text, max_results);
    }

    let words = split_shell_like_words(&line[..token.start]);
    let Some(command) = words
        .first()
        .and_then(|word| word.strip_prefix('#'))
        .filter(|command| IMPLEMENTED_PRIVATE_COMMANDS.contains(command))
    else {
        return Vec::new();
    };
    let args = words.iter().skip(1).map(String::as_str).collect::<Vec<_>>();
    let candidates = private_command_argument_candidates(command, &args, &token.text);
    let candidates = candidates.into_iter().map(|candidate| CompletionCandidate {
        display: candidate.to_string(),
        replacement: candidate.to_string(),
        is_dir: false,
        source: CompletionSource::PrivateCommand,
    });
    limit_candidates(candidates.collect(), max_results)
}

fn private_command_argument_candidates(
    command: &str,
    args_before_cursor: &[&str],
    prefix: &str,
) -> Vec<&'static str> {
    let candidates: &[&str] = match (command, args_before_cursor) {
        ("completion", []) => &[
            "on",
            "off",
            "mode",
            "max",
            "coalesce-ms",
            "display-delay-ms",
            "inline",
            "fuzzy",
            "tab-accept",
            "match-threshold",
            "typo-threshold",
        ],
        ("completion", ["mode"]) => &["auto", "tab", "off"],
        ("completion", ["inline" | "fuzzy"]) => &["on", "off"],
        ("completion", ["tab-accept"]) => &["full", "word"],
        ("paste", []) => &[
            "multiline",
            "confirm",
            "confirm-execute",
            "preview",
            "preview-lines",
            "preview-bytes",
        ],
        ("paste", ["multiline"]) => &["editor", "execute", "discard"],
        ("paste", ["confirm" | "confirm-execute" | "preview"]) => &["on", "off"],
        ("help", []) => HELP_TOPICS,
        ("key", []) => &["set", "clear"],
        ("prompt", []) => &["draft", "history", "ai", "reset"],
        ("context", []) => &["on", "off", "confirm"],
        ("context", ["confirm"]) => &["on", "off"],
        ("template", []) => &["find", "rm", "replace", "show", "use"],
        ("encrypt", []) => &["on", "off", "rotate", "unlock-mode", "rewrite-history"],
        ("encrypt", ["unlock-mode"]) => &["lazy", "prompt"],
        ("encrypt", ["rewrite-history"]) => &["plan", "run"],
        ("sync", []) => &[
            "off",
            "startup",
            "exit",
            "ai",
            "history",
            "templates",
            "drafts",
        ],
        ("sync", ["startup" | "exit"]) => &["on", "off"],
        ("sync", ["ai" | "history" | "templates" | "drafts"]) => &["on", "off"],
        _ => &[],
    };
    candidates
        .iter()
        .copied()
        .filter(|candidate| prefix.is_empty() || candidate.starts_with(prefix))
        .collect()
}
