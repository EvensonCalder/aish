use crate::config::CompletionTabAccept;
use crate::display_width::{
    display_width, truncate_end_with_ellipsis, truncate_start_with_ellipsis,
};

use super::{AcceptedCompletion, CompletionCandidate, CompletionSource, TokenContext};

pub fn render_completion_candidates(candidates: &[CompletionCandidate]) -> Vec<String> {
    candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}\t{}",
                completion_candidate_label(candidate),
                candidate.display
            )
        })
        .collect()
}

pub fn render_completion_candidates_for_width(
    candidates: &[CompletionCandidate],
    line: &str,
    token: &TokenContext,
    content_start_col: usize,
    width: usize,
) -> Vec<String> {
    candidates
        .iter()
        .map(|candidate| {
            render_completion_candidate_for_width(candidate, line, token, content_start_col, width)
        })
        .collect()
}

pub fn ghost_completion_suffix(
    token: &TokenContext,
    candidate: &CompletionCandidate,
) -> Option<String> {
    candidate
        .replacement
        .strip_prefix(&token.text)
        .filter(|suffix| !suffix.is_empty())
        .map(str::to_string)
}

pub fn accept_completion(
    line: &str,
    token: &TokenContext,
    candidate: &CompletionCandidate,
) -> AcceptedCompletion {
    accept_completion_with_mode(line, token, candidate, CompletionTabAccept::Full)
}

pub fn accept_completion_with_mode(
    line: &str,
    token: &TokenContext,
    candidate: &CompletionCandidate,
    mode: CompletionTabAccept,
) -> AcceptedCompletion {
    if completion_candidate_replaces_whole_line(candidate) {
        return AcceptedCompletion {
            line: candidate.replacement.clone(),
            cursor: candidate.replacement.len(),
        };
    }
    let replacement = accepted_replacement(token, candidate, mode);
    let mut accepted =
        String::with_capacity(line.len() - (token.end - token.start) + replacement.len());
    accepted.push_str(&line[..token.start]);
    accepted.push_str(&replacement);
    accepted.push_str(&line[token.end..]);
    let cursor = token.start + replacement.len();
    AcceptedCompletion {
        line: accepted,
        cursor,
    }
}

pub fn truncate_with_ellipsis(value: &str, width: usize) -> String {
    truncate_end_with_ellipsis(value, width)
}

fn completion_candidate_replaces_whole_line(candidate: &CompletionCandidate) -> bool {
    matches!(
        candidate.source,
        CompletionSource::TemplateTypo | CompletionSource::HistoryTypo
    )
}

fn render_completion_candidate_for_width(
    candidate: &CompletionCandidate,
    line: &str,
    token: &TokenContext,
    content_start_col: usize,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let label = completion_candidate_label(candidate);
    let label_width = display_width(label);
    if width <= label_width {
        return truncate_with_ellipsis(label, width);
    }
    let preferred_content_col = content_start_col.max(label_width + 1);
    let content_col = if preferred_content_col < width {
        preferred_content_col
    } else {
        label_width + 1
    };
    if content_col >= width {
        return truncate_with_ellipsis(label, width);
    }
    let display = accept_completion(line, token, candidate).line;
    let display = left_elide_words(&display, width - content_col);
    let mut row = String::with_capacity(width.min(label.len() + display.len() + 8));
    row.push_str(label);
    row.extend(std::iter::repeat_n(' ', content_col - label_width));
    row.push_str(&display);
    row
}

fn left_elide_words(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let words: Vec<&str> = value.split_whitespace().collect();
    if words.len() <= 1 {
        return left_truncate_with_ellipsis(value, width);
    }

    let available = width - 4;
    let mut selected = Vec::new();
    let mut selected_width = 0;
    for word in words.iter().rev() {
        let word_width = display_width(word);
        let next_width = if selected.is_empty() {
            word_width
        } else {
            selected_width + 1 + word_width
        };
        if next_width > available {
            break;
        }
        selected.push(*word);
        selected_width = next_width;
    }
    if selected.is_empty() {
        return left_truncate_with_ellipsis(value, width);
    }
    selected.reverse();
    format!("... {}", selected.join(" "))
}

fn left_truncate_with_ellipsis(value: &str, width: usize) -> String {
    truncate_start_with_ellipsis(value, width)
}

fn accepted_replacement(
    token: &TokenContext,
    candidate: &CompletionCandidate,
    mode: CompletionTabAccept,
) -> String {
    match mode {
        CompletionTabAccept::Full => candidate.replacement.clone(),
        CompletionTabAccept::Word => {
            let Some(suffix) = candidate.replacement.strip_prefix(&token.text) else {
                return accepted_word_suffix(&candidate.replacement).to_string();
            };
            format!("{}{}", token.text, accepted_word_suffix(suffix))
        }
    }
}

fn accepted_word_suffix(suffix: &str) -> &str {
    let mut seen_non_whitespace = false;
    for (index, ch) in suffix.char_indices() {
        if ch.is_whitespace() {
            if seen_non_whitespace {
                return &suffix[..index];
            }
        } else {
            seen_non_whitespace = true;
        }
    }
    suffix
}

fn completion_source_label(source: CompletionSource) -> &'static str {
    match source {
        CompletionSource::Path => "file",
        CompletionSource::Template => "template",
        CompletionSource::TemplateTypo => "template",
        CompletionSource::History => "history",
        CompletionSource::HistoryTypo => "history",
        CompletionSource::Executable => "exec",
        CompletionSource::TemplatePlaceholder => "placeholder",
        CompletionSource::PrivateCommand => "aish",
    }
}

fn completion_candidate_label(candidate: &CompletionCandidate) -> &'static str {
    match candidate.source {
        CompletionSource::Path if candidate.is_dir => "dir",
        _ => completion_source_label(candidate.source),
    }
}
