use std::collections::HashSet;

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

use super::index::{
    IndexedHistoryEntry, IndexedTemplateEntry, index_history_entries, index_template_entries,
};
use super::matching::{
    template_words_match_threshold_with_typos, typo_similarity_percent, word_prefix_matches,
    words_match_threshold_with_typos,
};
use super::parser::{current_token_context, split_shell_like_words};
use super::ranking::{dedupe_completion_candidates, limit_candidates};
use super::{CompletionCandidate, CompletionOptions, CompletionSource};

pub fn complete_first_token_typos_with_options(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_history = index_history_entries(history_newest_first);
    let indexed_templates = index_template_entries(templates);
    complete_first_token_typos_with_indexed_options(
        prefix,
        &indexed_history,
        &indexed_templates,
        options,
    )
}

pub fn complete_non_first_token_typos_for_line_with_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_history = index_history_entries(history_newest_first);
    let indexed_templates = index_template_entries(templates);
    complete_non_first_token_typos_for_line_with_indexed_options(
        line,
        cursor,
        &indexed_history,
        &indexed_templates,
        options,
    )
}

pub(crate) fn complete_non_first_token_typos_for_line_with_indexed_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[IndexedHistoryEntry],
    templates: &[IndexedTemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if !options.fuzzy_enabled {
        return Vec::new();
    }
    let token = current_token_context(line, cursor);
    if token.is_first_token {
        return complete_first_token_typos_with_indexed_options(
            &token.text,
            history_newest_first,
            templates,
            options,
        );
    }
    complete_typo_candidates_for_line_with_indexed_options(
        line,
        cursor,
        history_newest_first,
        templates,
        options,
    )
}

pub(crate) fn complete_first_token_typos_with_indexed_options(
    prefix: &str,
    history_newest_first: &[IndexedHistoryEntry],
    templates: &[IndexedTemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if !options.fuzzy_enabled {
        return Vec::new();
    }
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let mut seen_templates = HashSet::new();
    for indexed in templates.iter().rev() {
        let Some(first_word) = indexed.words.first() else {
            continue;
        };
        if word_prefix_matches(first_word, prefix, options.ignore_spaces)
            || typo_similarity_percent(first_word, prefix, options.ignore_spaces)
                < options.typo_threshold_percent
        {
            continue;
        }
        if seen_templates.insert(indexed.id.as_str()) {
            candidates.push(CompletionCandidate {
                display: indexed.entry.body.clone(),
                replacement: indexed.entry.body.clone(),
                is_dir: false,
                source: CompletionSource::TemplateTypo,
            });
        }
    }
    let mut seen_history = HashSet::new();
    for indexed in history_newest_first {
        let Some(first_word) = indexed.words.first() else {
            continue;
        };
        if word_prefix_matches(first_word, prefix, options.ignore_spaces)
            || typo_similarity_percent(first_word, prefix, options.ignore_spaces)
                < options.typo_threshold_percent
        {
            continue;
        }
        if seen_history.insert(indexed.entry.command.as_str()) {
            candidates.push(CompletionCandidate {
                display: indexed.entry.command.clone(),
                replacement: indexed.entry.command.clone(),
                is_dir: false,
                source: CompletionSource::HistoryTypo,
            });
        }
    }
    limit_candidates(candidates, options.max_results)
}

pub fn complete_typo_candidates_for_line_with_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_history = index_history_entries(history_newest_first);
    let indexed_templates = index_template_entries(templates);
    complete_typo_candidates_for_line_with_indexed_options(
        line,
        cursor,
        &indexed_history,
        &indexed_templates,
        options,
    )
}

fn complete_typo_candidates_for_line_with_indexed_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[IndexedHistoryEntry],
    templates: &[IndexedTemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if !options.fuzzy_enabled {
        return Vec::new();
    }
    let token = current_token_context(line, cursor);
    let words_before_cursor = split_shell_like_words(&line[..cursor.min(line.len())]);
    if words_before_cursor.is_empty() {
        return Vec::new();
    }
    let current_word_index = if token.text.is_empty() {
        words_before_cursor.len()
    } else {
        words_before_cursor.len().saturating_sub(1)
    };

    let mut candidates = Vec::new();
    let mut seen_templates = HashSet::new();
    for indexed in templates.iter().rev() {
        if indexed.words.len() <= current_word_index {
            continue;
        }
        if !template_words_match_threshold_with_typos(
            &indexed.words,
            &words_before_cursor,
            options.ignore_spaces,
            options.match_threshold_percent,
            options.typo_threshold_percent,
        ) {
            continue;
        }
        let replacement = indexed.entry.body.clone();
        if replacement == line || !seen_templates.insert(indexed.id.as_str()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: indexed.entry.body.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::TemplateTypo,
        });
    }

    let mut seen_history = HashSet::new();
    for indexed in history_newest_first {
        if indexed.words.len() <= current_word_index {
            continue;
        }
        if !words_match_threshold_with_typos(
            &indexed.words,
            &words_before_cursor,
            options.ignore_spaces,
            options.match_threshold_percent,
            options.typo_threshold_percent,
        ) {
            continue;
        }
        let replacement = indexed.entry.command.clone();
        if replacement == line || !seen_history.insert(indexed.entry.command.as_str()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: indexed.entry.command.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::HistoryTypo,
        });
    }

    dedupe_completion_candidates(&mut candidates);
    limit_candidates(candidates, options.max_results)
}
