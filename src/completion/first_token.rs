use std::collections::HashSet;
use std::path::PathBuf;

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

use super::index::{
    IndexedHistoryEntry, IndexedTemplateEntry, index_history_entries, index_template_entries,
};
use super::matching::CompletionMatcher;
use super::path::{complete_path_executables, scan_path_executables};
use super::ranking::limit_candidates;
use super::{CompletionCandidate, CompletionOptions, CompletionSource};

pub fn complete_first_token(
    prefix: &str,
    templates: &[TemplateEntry],
    history_newest_first: &[HistoryEntry],
    path_dirs: &[PathBuf],
) -> Vec<CompletionCandidate> {
    complete_first_token_with_options(
        prefix,
        templates,
        history_newest_first,
        path_dirs,
        CompletionOptions {
            max_results: usize::MAX,
            ignore_spaces: false,
            fuzzy_enabled: true,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        },
    )
}

pub fn complete_first_token_with_options(
    prefix: &str,
    templates: &[TemplateEntry],
    history_newest_first: &[HistoryEntry],
    path_dirs: &[PathBuf],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let matcher = CompletionMatcher::new(
        options.ignore_spaces,
        options.match_threshold_percent,
        options.typo_threshold_percent,
    );
    let mut seen_templates = HashSet::new();
    for template in templates.iter().rev() {
        if matcher.prefix_matches(&template.body, prefix) && seen_templates.insert(template.id()) {
            candidates.push(CompletionCandidate {
                display: template.body.clone(),
                replacement: template.body.clone(),
                is_dir: false,
                source: CompletionSource::Template,
            });
        }
    }

    let mut seen_history = HashSet::new();
    for entry in history_newest_first {
        if matcher.prefix_matches(&entry.command, prefix)
            && seen_history.insert(entry.command.as_str())
        {
            candidates.push(CompletionCandidate {
                display: entry.command.clone(),
                replacement: entry.command.clone(),
                is_dir: false,
                source: CompletionSource::History,
            });
        }
    }

    let mut executable_candidates = complete_path_executables(prefix, path_dirs);
    candidates.append(&mut executable_candidates);
    limit_candidates(candidates, options.max_results)
}

pub(crate) fn complete_first_token_history_with_indexed_options(
    prefix: &str,
    history_newest_first: &[IndexedHistoryEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut seen_history = HashSet::new();
    let mut candidates = Vec::new();
    let matcher = CompletionMatcher::new(
        options.ignore_spaces,
        options.match_threshold_percent,
        options.typo_threshold_percent,
    );
    for indexed in history_newest_first {
        if matcher.prefix_matches(&indexed.entry.command, prefix)
            && seen_history.insert(indexed.entry.command.as_str())
        {
            candidates.push(CompletionCandidate {
                display: indexed.entry.command.clone(),
                replacement: indexed.entry.command.clone(),
                is_dir: false,
                source: CompletionSource::History,
            });
        }
    }
    limit_candidates(candidates, options.max_results)
}

pub(crate) fn complete_first_token_templates_with_indexed_options(
    prefix: &str,
    templates: &[IndexedTemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let mut seen_templates = HashSet::new();
    let matcher = CompletionMatcher::new(
        options.ignore_spaces,
        options.match_threshold_percent,
        options.typo_threshold_percent,
    );
    for indexed in templates.iter().rev() {
        if matcher.prefix_matches(&indexed.entry.body, prefix)
            && seen_templates.insert(indexed.id.as_str())
        {
            candidates.push(CompletionCandidate {
                display: indexed.entry.body.clone(),
                replacement: indexed.entry.body.clone(),
                is_dir: false,
                source: CompletionSource::Template,
            });
        }
    }
    limit_candidates(candidates, options.max_results)
}

pub fn complete_first_token_history_with_options(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_history = index_history_entries(history_newest_first);
    complete_first_token_history_with_indexed_options(prefix, &indexed_history, options)
}

pub fn complete_first_token_templates_with_options(
    prefix: &str,
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_templates = index_template_entries(templates);
    complete_first_token_templates_with_indexed_options(prefix, &indexed_templates, options)
}

pub fn complete_first_token_executables_with_options(
    prefix: &str,
    path_dirs: &[PathBuf],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let executables = scan_path_executables(path_dirs);
    complete_first_token_executables_from_names_with_options(prefix, &executables, options)
}

pub(crate) fn complete_first_token_executables_from_names_with_options(
    prefix: &str,
    executables: &[String],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let candidates = executables
        .iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| CompletionCandidate {
            display: name.clone(),
            replacement: name.clone(),
            is_dir: false,
            source: CompletionSource::Executable,
        })
        .collect();
    limit_candidates(candidates, options.max_results)
}
