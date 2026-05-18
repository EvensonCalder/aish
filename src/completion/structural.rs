use std::collections::HashSet;

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

use super::index::{
    IndexedHistoryEntry, IndexedTemplateEntry, index_history_entries, index_template_entries,
};
use super::matching::{
    join_words, template_replacement_for_index, template_words_match_threshold,
    words_match_threshold,
};
use super::parser::{current_token_context, split_shell_like_words};
use super::ranking::limit_candidates;
use super::{CompletionCandidate, CompletionOptions, CompletionSource, TokenContext};

pub fn complete_structural_history_for_line_with_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[HistoryEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let token = current_token_context(line, cursor);
    let indexed_history = index_history_entries(history_newest_first);
    limit_candidates(
        complete_structural_history_for_line_indexed(
            line,
            cursor,
            &token,
            &indexed_history,
            options.ignore_spaces,
            options.match_threshold_percent,
        ),
        options.max_results,
    )
}

pub fn complete_structural_templates_for_line_with_options(
    line: &str,
    cursor: usize,
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let token = current_token_context(line, cursor);
    let indexed_templates = index_template_entries(templates);
    limit_candidates(
        complete_structural_templates_for_line_indexed(
            line,
            cursor,
            &token,
            &indexed_templates,
            options.ignore_spaces,
            options.match_threshold_percent,
        ),
        options.max_results,
    )
}

pub(crate) fn complete_structural_templates_for_line_indexed(
    line: &str,
    cursor: usize,
    token: &TokenContext,
    templates: &[IndexedTemplateEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    if cursor != line.len() {
        return Vec::new();
    }
    let words_before_cursor = split_shell_like_words(&line[..cursor]);
    if words_before_cursor.is_empty() {
        return Vec::new();
    }
    let current_word_index = if token.text.is_empty() {
        words_before_cursor.len()
    } else {
        words_before_cursor.len().saturating_sub(1)
    };
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for indexed in templates.iter().rev() {
        if indexed.words.len() <= current_word_index {
            continue;
        }
        if !template_words_match_threshold(
            &indexed.words,
            &words_before_cursor,
            ignore_spaces,
            match_threshold_percent,
        ) {
            continue;
        }

        let replacement = template_replacement_for_index(
            &indexed.words,
            current_word_index,
            token,
            ignore_spaces,
            match_threshold_percent,
        );

        if replacement == token.text || !seen.insert(replacement.clone()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: indexed.entry.body.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::Template,
        });
    }
    candidates
}

pub(crate) fn complete_structural_history_for_line_indexed(
    line: &str,
    cursor: usize,
    token: &TokenContext,
    history_newest_first: &[IndexedHistoryEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    if cursor != line.len() {
        return Vec::new();
    }
    let words_before_cursor = split_shell_like_words(&line[..cursor]);
    if words_before_cursor.is_empty() {
        return Vec::new();
    }
    let current_word_index = if token.text.is_empty() {
        words_before_cursor.len()
    } else {
        words_before_cursor.len().saturating_sub(1)
    };
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for indexed in history_newest_first {
        if indexed.words.len() <= current_word_index {
            continue;
        }
        if !words_match_threshold(
            &indexed.words,
            &words_before_cursor,
            ignore_spaces,
            match_threshold_percent,
        ) {
            continue;
        }

        let replacement = join_words(&indexed.raw_words[current_word_index..]);

        if replacement == token.text || !seen.insert(replacement.clone()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: replacement.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::History,
        });
    }
    candidates
}
