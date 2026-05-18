use std::collections::HashSet;

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

use super::index::{IndexedHistoryEntry, IndexedTemplateEntry};
use super::matching::{matches_completion_prefix_with_threshold, template_placeholder_words};
use super::parser::{command_argument_words, shell_word_value};
use super::{CompletionCandidate, CompletionSource};

pub(crate) fn complete_history_arguments(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let prefix = shell_word_match_text(prefix);
    for entry in history_newest_first {
        for argument in command_argument_words(&entry.command) {
            if matches_completion_prefix_with_threshold(
                &argument.value,
                &prefix,
                ignore_spaces,
                match_threshold_percent,
            ) && seen.insert(argument.raw.clone())
            {
                candidates.push(CompletionCandidate {
                    display: argument.raw.clone(),
                    replacement: argument.raw,
                    is_dir: false,
                    source: CompletionSource::History,
                });
            }
        }
    }
    candidates
}

pub(crate) fn complete_history_arguments_indexed(
    prefix: &str,
    history_newest_first: &[IndexedHistoryEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let prefix = shell_word_match_text(prefix);
    for indexed in history_newest_first {
        for argument in &indexed.arguments {
            if matches_completion_prefix_with_threshold(
                &argument.value,
                &prefix,
                ignore_spaces,
                match_threshold_percent,
            ) && seen.insert(argument.raw.clone())
            {
                candidates.push(CompletionCandidate {
                    display: argument.raw.clone(),
                    replacement: argument.raw.clone(),
                    is_dir: false,
                    source: CompletionSource::History,
                });
            }
        }
    }
    candidates
}

fn shell_word_match_text(token: &str) -> String {
    shell_word_value(token)
}

pub(crate) fn complete_template_placeholders(
    prefix: &str,
    templates: &[TemplateEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for template in templates {
        for placeholder in template_placeholder_words(&template.body) {
            if (matches_completion_prefix_with_threshold(
                &placeholder.raw,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            ) || matches_completion_prefix_with_threshold(
                &placeholder.name,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            )) && seen.insert(placeholder.raw.clone())
            {
                candidates.push(CompletionCandidate {
                    display: placeholder.raw.clone(),
                    replacement: placeholder.raw,
                    is_dir: false,
                    source: CompletionSource::TemplatePlaceholder,
                });
            }
        }
    }
    candidates
}

pub(crate) fn complete_template_placeholders_indexed(
    prefix: &str,
    templates: &[IndexedTemplateEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for indexed in templates {
        for placeholder in &indexed.placeholders {
            if (matches_completion_prefix_with_threshold(
                &placeholder.raw,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            ) || matches_completion_prefix_with_threshold(
                &placeholder.name,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            )) && seen.insert(placeholder.raw.clone())
            {
                candidates.push(CompletionCandidate {
                    display: placeholder.raw.clone(),
                    replacement: placeholder.raw.clone(),
                    is_dir: false,
                    source: CompletionSource::TemplatePlaceholder,
                });
            }
        }
    }
    candidates
}
