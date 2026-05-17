use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

mod matching;
mod parser;
mod path;
mod private;
mod render;

pub use matching::{matches_completion_prefix, matches_completion_prefix_with_threshold};
pub use parser::{current_token_context, is_path_like_token};
pub use path::complete_path;
pub use private::{complete_private_command_line, complete_private_commands};
pub use render::{
    accept_completion, accept_completion_with_mode, ghost_completion_suffix,
    render_completion_candidates, render_completion_candidates_for_width, truncate_with_ellipsis,
};

use matching::{
    edit_distance_chars, join_words, template_placeholder_words, template_replacement_for_index,
    template_words_match_threshold, template_words_match_threshold_with_typos,
    typo_similarity_percent, word_prefix_matches, words_match_threshold,
    words_match_threshold_with_typos,
};
use parser::{command_arguments, split_shell_like_words};
pub(crate) use path::scan_path_executables;
use path::{
    complete_path_executables, complete_path_with_options, order_path_candidates_for_completion,
    split_path_candidates,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionOptions {
    pub max_results: usize,
    pub ignore_spaces: bool,
    pub fuzzy_enabled: bool,
    pub match_threshold_percent: usize,
    pub typo_threshold_percent: usize,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            max_results: 5,
            ignore_spaces: true,
            fuzzy_enabled: true,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenContext {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub is_first_token: bool,
    pub quote: Option<char>,
    pub path_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub display: String,
    pub replacement: String,
    pub is_dir: bool,
    pub source: CompletionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedHistoryEntry {
    pub(crate) entry: HistoryEntry,
    pub(crate) words: Vec<String>,
    pub(crate) arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedTemplateEntry {
    pub(crate) entry: TemplateEntry,
    pub(crate) id: String,
    pub(crate) words: Vec<String>,
    pub(crate) placeholders: Vec<IndexedTemplatePlaceholder>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedTemplatePlaceholder {
    pub(crate) raw: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCompletion {
    pub line: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionSource {
    Path,
    Template,
    TemplateTypo,
    History,
    HistoryTypo,
    Executable,
    TemplatePlaceholder,
    PrivateCommand,
}

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
    let mut seen_templates = HashSet::new();
    for template in templates.iter().rev() {
        if matches_completion_prefix_with_threshold(
            &template.body,
            prefix,
            options.ignore_spaces,
            options.match_threshold_percent,
        ) && seen_templates.insert(template.id())
        {
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
        if matches_completion_prefix_with_threshold(
            &entry.command,
            prefix,
            options.ignore_spaces,
            options.match_threshold_percent,
        ) && seen_history.insert(entry.command.as_str())
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

pub fn complete_non_first_token(
    token: &str,
    cwd: &Path,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
) -> Vec<CompletionCandidate> {
    complete_non_first_token_with_options(
        token,
        cwd,
        history_newest_first,
        templates,
        CompletionOptions {
            max_results: usize::MAX,
            ignore_spaces: false,
            fuzzy_enabled: true,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        },
    )
}

pub fn complete_non_first_token_with_options(
    token: &str,
    cwd: &Path,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if token.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let path_candidates = complete_path_with_options(token, cwd, options);
    let (directory_candidates, file_candidates) = split_path_candidates(path_candidates);
    candidates.extend(directory_candidates);
    candidates.extend(complete_template_placeholders(
        token,
        templates,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(complete_history_arguments(
        token,
        history_newest_first,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(file_candidates);
    limit_candidates(candidates, options.max_results)
}

pub fn complete_non_first_token_for_line_with_options(
    line: &str,
    cursor: usize,
    cwd: &Path,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let indexed_history = index_history_entries(history_newest_first);
    let indexed_templates = index_template_entries(templates);
    complete_non_first_token_for_line_with_indexed_options(
        line,
        cursor,
        cwd,
        &indexed_history,
        &indexed_templates,
        options,
    )
}

pub(crate) fn index_history_entries(
    history_newest_first: &[HistoryEntry],
) -> Vec<IndexedHistoryEntry> {
    history_newest_first
        .iter()
        .cloned()
        .map(|entry| IndexedHistoryEntry {
            words: split_shell_like_words(&entry.command),
            arguments: command_arguments(&entry.command)
                .into_iter()
                .map(str::to_string)
                .collect(),
            entry,
        })
        .collect()
}

pub(crate) fn index_template_entries(templates: &[TemplateEntry]) -> Vec<IndexedTemplateEntry> {
    templates
        .iter()
        .cloned()
        .map(|entry| {
            let placeholders = template_placeholder_words(&entry.body)
                .into_iter()
                .map(|placeholder| IndexedTemplatePlaceholder {
                    raw: placeholder.raw,
                    name: placeholder.name,
                })
                .collect();
            IndexedTemplateEntry {
                id: entry.id(),
                words: split_shell_like_words(&entry.body),
                placeholders,
                entry,
            }
        })
        .collect()
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
    for indexed in history_newest_first {
        if matches_completion_prefix_with_threshold(
            &indexed.entry.command,
            prefix,
            options.ignore_spaces,
            options.match_threshold_percent,
        ) && seen_history.insert(indexed.entry.command.as_str())
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
    for indexed in templates.iter().rev() {
        if matches_completion_prefix_with_threshold(
            &indexed.entry.body,
            prefix,
            options.ignore_spaces,
            options.match_threshold_percent,
        ) && seen_templates.insert(indexed.id.as_str())
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

pub(crate) fn complete_non_first_token_for_line_with_indexed_options(
    line: &str,
    cursor: usize,
    cwd: &Path,
    history_newest_first: &[IndexedHistoryEntry],
    templates: &[IndexedTemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let token = current_token_context(line, cursor);
    let mut structural_candidates = complete_structural_templates_for_line_indexed(
        line,
        cursor,
        &token,
        templates,
        options.ignore_spaces,
        options.match_threshold_percent,
    );
    if token.text.is_empty() {
        structural_candidates.extend(complete_structural_history_for_line_indexed(
            line,
            cursor,
            &token,
            history_newest_first,
            options.ignore_spaces,
            options.match_threshold_percent,
        ));
        dedupe_completion_candidates(&mut structural_candidates);
        return limit_candidates(structural_candidates, options.max_results);
    }
    let structural_history_candidates = complete_structural_history_for_line_indexed(
        line,
        cursor,
        &token,
        history_newest_first,
        options.ignore_spaces,
        options.match_threshold_percent,
    );
    let path_candidates = complete_path_with_options(&token.text, cwd, options);
    let (directory_candidates, file_candidates) = split_path_candidates(path_candidates);
    if token.path_like {
        if !directory_candidates.is_empty() {
            let mut candidates = directory_candidates;
            candidates.extend(structural_candidates);
            candidates.extend(structural_history_candidates);
            candidates.extend(file_candidates);
            dedupe_completion_candidates(&mut candidates);
            return limit_candidates(candidates, options.max_results);
        }
        structural_candidates.extend(structural_history_candidates);
        if !structural_candidates.is_empty() {
            dedupe_completion_candidates(&mut structural_candidates);
            return limit_candidates(structural_candidates, options.max_results);
        }
        return limit_candidates(file_candidates, options.max_results);
    }
    let has_structural =
        !structural_candidates.is_empty() || !structural_history_candidates.is_empty();
    if has_structural {
        structural_candidates.extend(directory_candidates);
        structural_candidates.extend(structural_history_candidates);
        rank_completion_candidates(&mut structural_candidates);
        dedupe_completion_candidates(&mut structural_candidates);
        return limit_candidates(structural_candidates, options.max_results);
    }
    let mut candidates = Vec::new();
    candidates.extend(directory_candidates);
    candidates.extend(complete_template_placeholders_indexed(
        &token.text,
        templates,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(complete_history_arguments_indexed(
        &token.text,
        history_newest_first,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(file_candidates);
    dedupe_completion_candidates(&mut candidates);
    limit_candidates(candidates, options.max_results)
}

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

pub fn complete_history_arguments_for_token_with_options(
    token: &str,
    history_newest_first: &[HistoryEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    limit_candidates(
        complete_history_arguments(
            token,
            history_newest_first,
            options.ignore_spaces,
            options.match_threshold_percent,
        ),
        options.max_results,
    )
}

pub fn complete_template_placeholders_for_token_with_options(
    token: &str,
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    limit_candidates(
        complete_template_placeholders(
            token,
            templates,
            options.ignore_spaces,
            options.match_threshold_percent,
        ),
        options.max_results,
    )
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

pub fn complete_non_first_token_fallbacks_for_line_with_options(
    line: &str,
    cursor: usize,
    cwd: &Path,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    let token = current_token_context(line, cursor);
    if token.text.is_empty() {
        return Vec::new();
    }
    let path_candidates = complete_path_with_options(&token.text, cwd, options);
    if token.path_like {
        return limit_candidates(
            order_path_candidates_for_completion(path_candidates),
            options.max_results,
        );
    }
    let mut candidates = Vec::new();
    let (directory_candidates, file_candidates) = split_path_candidates(path_candidates);
    candidates.extend(directory_candidates);
    candidates.extend(complete_template_placeholders(
        &token.text,
        templates,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(complete_history_arguments(
        &token.text,
        history_newest_first,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    candidates.extend(file_candidates);
    dedupe_completion_candidates(&mut candidates);
    limit_candidates(candidates, options.max_results)
}

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

fn complete_structural_templates_for_line_indexed(
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

fn complete_structural_history_for_line_indexed(
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

        let replacement = join_words(&indexed.words[current_word_index..]);

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

fn complete_history_arguments(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for entry in history_newest_first {
        for argument in command_arguments(&entry.command) {
            if matches_completion_prefix_with_threshold(
                argument,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            ) && seen.insert(argument.to_string())
            {
                candidates.push(CompletionCandidate {
                    display: argument.to_string(),
                    replacement: argument.to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                });
            }
        }
    }
    candidates
}

fn complete_history_arguments_indexed(
    prefix: &str,
    history_newest_first: &[IndexedHistoryEntry],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for indexed in history_newest_first {
        for argument in &indexed.arguments {
            if matches_completion_prefix_with_threshold(
                argument,
                prefix,
                ignore_spaces,
                match_threshold_percent,
            ) && seen.insert(argument.clone())
            {
                candidates.push(CompletionCandidate {
                    display: argument.clone(),
                    replacement: argument.clone(),
                    is_dir: false,
                    source: CompletionSource::History,
                });
            }
        }
    }
    candidates
}

pub(crate) fn dedupe_completion_candidates(candidates: &mut Vec<CompletionCandidate>) {
    let mut seen = HashSet::new();
    candidates.retain(|candidate| {
        seen.insert((
            candidate.source,
            candidate.replacement.clone(),
            candidate.display.clone(),
        ))
    });
}

pub(crate) fn rank_completion_candidates(candidates: &mut [CompletionCandidate]) {
    candidates.sort_by_key(completion_candidate_rank);
}

fn completion_candidate_rank(candidate: &CompletionCandidate) -> u8 {
    if candidate.source == CompletionSource::Path && candidate.is_dir {
        return 18;
    }
    completion_source_rank(candidate.source)
}

fn completion_source_rank(source: CompletionSource) -> u8 {
    match source {
        CompletionSource::PrivateCommand => 0,
        CompletionSource::TemplateTypo => 9,
        CompletionSource::Template => 10,
        CompletionSource::HistoryTypo => 19,
        CompletionSource::History => 20,
        CompletionSource::Executable => 30,
        CompletionSource::TemplatePlaceholder => 40,
        CompletionSource::Path => 50,
    }
}

fn complete_template_placeholders(
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

fn complete_template_placeholders_indexed(
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

pub fn limit_candidates(
    mut candidates: Vec<CompletionCandidate>,
    max_results: usize,
) -> Vec<CompletionCandidate> {
    candidates.truncate(max_results);
    candidates
}

#[cfg(test)]
mod tests;
