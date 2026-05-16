use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::IMPLEMENTED_PRIVATE_COMMANDS;
use crate::config::CompletionTabAccept;
use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

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
pub struct AcceptedCompletion {
    pub line: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionSource {
    Path,
    Template,
    History,
    Executable,
    TemplatePlaceholder,
    PrivateCommand,
}

pub fn current_token_context(line: &str, cursor: usize) -> TokenContext {
    let cursor = cursor.min(line.len());
    let cursor = previous_char_boundary(line, cursor);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_start = 0;
    let mut token_seen = false;
    let mut token_before_current = false;

    for (index, ch) in line[..cursor].char_indices() {
        if escaped {
            escaped = false;
            token_seen = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => {
                quote = None;
                token_seen = true;
            }
            Some(_) => {
                if ch == '\\' && quote == Some('"') {
                    escaped = true;
                }
                token_seen = true;
            }
            None if ch == '\\' => {
                escaped = true;
                token_seen = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_seen = true;
            }
            None if ch.is_whitespace() => {
                if token_seen {
                    token_before_current = true;
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    let text = line[token_start..cursor].to_string();
    TokenContext {
        start: token_start,
        end: cursor,
        path_like: is_path_like_token(&text),
        text,
        is_first_token: !token_before_current,
        quote,
    }
}

pub fn is_path_like_token(token: &str) -> bool {
    let token = token.trim_start_matches(['\'', '"']);
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('~')
        || token.contains('/')
}

pub fn complete_path(token: &str, cwd: &Path) -> Vec<CompletionCandidate> {
    let (quote, token) = strip_opening_quote(token);
    let (dir_token, prefix) = split_path_token(token);
    let Some(search_dir) = resolve_search_dir(dir_token, cwd) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(search_dir) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if !file_name.starts_with(prefix) {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        let replacement = format!("{quote}{dir_token}{file_name}{suffix}");
        candidates.push(CompletionCandidate {
            display: format!("{dir_token}{file_name}{suffix}"),
            replacement,
            is_dir,
            source: CompletionSource::Path,
        });
    }
    candidates.sort_by(|left, right| left.display.cmp(&right.display));
    candidates
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
    candidates.extend(complete_path(token, cwd));
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
    let token = current_token_context(line, cursor);
    let mut structural_candidates = complete_structural_templates_for_line(
        line,
        cursor,
        &token,
        templates,
        options.ignore_spaces,
        options.match_threshold_percent,
    );
    if token.text.is_empty() {
        structural_candidates.extend(complete_structural_history_for_line(
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
    structural_candidates.extend(complete_structural_history_for_line(
        line,
        cursor,
        &token,
        history_newest_first,
        options.ignore_spaces,
        options.match_threshold_percent,
    ));
    if !structural_candidates.is_empty() {
        dedupe_completion_candidates(&mut structural_candidates);
        return limit_candidates(structural_candidates, options.max_results);
    }
    let path_candidates = complete_path(&token.text, cwd);
    if token.path_like {
        return limit_candidates(path_candidates, options.max_results);
    }
    let mut candidates = Vec::new();
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
    candidates.extend(path_candidates);
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
    limit_candidates(
        complete_structural_history_for_line(
            line,
            cursor,
            &token,
            history_newest_first,
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
    limit_candidates(
        complete_structural_templates_for_line(
            line,
            cursor,
            &token,
            templates,
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
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut seen_history = HashSet::new();
    let mut candidates = Vec::new();
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
    limit_candidates(candidates, options.max_results)
}

pub fn complete_first_token_templates_with_options(
    prefix: &str,
    templates: &[TemplateEntry],
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
    limit_candidates(candidates, options.max_results)
}

pub fn complete_first_token_executables_with_options(
    prefix: &str,
    path_dirs: &[PathBuf],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        return Vec::new();
    }
    limit_candidates(
        complete_path_executables(prefix, path_dirs),
        options.max_results,
    )
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
    let path_candidates = complete_path(&token.text, cwd);
    if token.path_like {
        return limit_candidates(path_candidates, options.max_results);
    }
    let mut candidates = Vec::new();
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
    candidates.extend(path_candidates);
    dedupe_completion_candidates(&mut candidates);
    limit_candidates(candidates, options.max_results)
}

pub fn complete_first_token_typos_with_options(
    prefix: &str,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
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
    for template in templates.iter().rev() {
        let Some(first_word) = split_shell_like_words(&template.body).first().cloned() else {
            continue;
        };
        if word_prefix_matches(&first_word, prefix, options.ignore_spaces)
            || typo_similarity_percent(&first_word, prefix, options.ignore_spaces)
                < options.typo_threshold_percent
        {
            continue;
        }
        if seen_templates.insert(template.id()) {
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
        let Some(first_word) = split_shell_like_words(&entry.command).first().cloned() else {
            continue;
        };
        if word_prefix_matches(&first_word, prefix, options.ignore_spaces)
            || typo_similarity_percent(&first_word, prefix, options.ignore_spaces)
                < options.typo_threshold_percent
        {
            continue;
        }
        if seen_history.insert(entry.command.as_str()) {
            candidates.push(CompletionCandidate {
                display: entry.command.clone(),
                replacement: entry.command.clone(),
                is_dir: false,
                source: CompletionSource::History,
            });
        }
    }
    limit_candidates(candidates, options.max_results)
}

pub fn complete_non_first_token_typos_for_line_with_options(
    line: &str,
    cursor: usize,
    history_newest_first: &[HistoryEntry],
    templates: &[TemplateEntry],
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    if !options.fuzzy_enabled {
        return Vec::new();
    }
    let token = current_token_context(line, cursor);
    if token.is_first_token {
        return complete_first_token_typos_with_options(
            &token.text,
            history_newest_first,
            templates,
            options,
        );
    }
    complete_typo_candidates_for_line_with_options(
        line,
        cursor,
        history_newest_first,
        templates,
        options,
    )
}

fn complete_structural_templates_for_line(
    line: &str,
    cursor: usize,
    token: &TokenContext,
    templates: &[TemplateEntry],
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

    for template in templates.iter().rev() {
        let template_words = split_shell_like_words(&template.body);
        if template_words.len() <= current_word_index {
            continue;
        }
        if !template_words_match_threshold(
            &template_words,
            &words_before_cursor,
            ignore_spaces,
            match_threshold_percent,
        ) {
            continue;
        }

        let replacement = template_replacement_for_index(
            &template_words,
            current_word_index,
            token,
            ignore_spaces,
            match_threshold_percent,
        );

        if replacement == token.text || !seen.insert(replacement.clone()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: template.body.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::Template,
        });
    }
    candidates
}

fn complete_structural_history_for_line(
    line: &str,
    cursor: usize,
    token: &TokenContext,
    history_newest_first: &[HistoryEntry],
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

    for entry in history_newest_first {
        let history_words = split_shell_like_words(&entry.command);
        if history_words.len() <= current_word_index {
            continue;
        }
        if !words_match_threshold(
            &history_words,
            &words_before_cursor,
            ignore_spaces,
            match_threshold_percent,
        ) {
            continue;
        }

        let replacement = join_words(&history_words[current_word_index..]);

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
    for template in templates.iter().rev() {
        let template_words = split_shell_like_words(&template.body);
        if template_words.len() <= current_word_index {
            continue;
        }
        if !template_words_match_threshold_with_typos(
            &template_words,
            &words_before_cursor,
            options.ignore_spaces,
            options.match_threshold_percent,
            options.typo_threshold_percent,
        ) {
            continue;
        }
        let replacement = template_replacement_for_index(
            &template_words,
            current_word_index,
            &token,
            options.ignore_spaces,
            options.match_threshold_percent,
        );
        if replacement == token.text || !seen_templates.insert(replacement.clone()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: template.body.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::Template,
        });
    }

    let mut seen_history = HashSet::new();
    for entry in history_newest_first {
        let history_words = split_shell_like_words(&entry.command);
        if history_words.len() <= current_word_index {
            continue;
        }
        if !words_match_threshold_with_typos(
            &history_words,
            &words_before_cursor,
            options.ignore_spaces,
            options.match_threshold_percent,
            options.typo_threshold_percent,
        ) {
            continue;
        }
        let replacement = join_words(&history_words[current_word_index..]);
        if replacement == token.text || !seen_history.insert(replacement.clone()) {
            continue;
        }
        candidates.push(CompletionCandidate {
            display: replacement.clone(),
            replacement,
            is_dir: false,
            source: CompletionSource::History,
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
    candidates.sort_by_key(|candidate| completion_source_rank(candidate.source));
}

fn completion_source_rank(source: CompletionSource) -> u8 {
    match source {
        CompletionSource::PrivateCommand => 0,
        CompletionSource::Template => 10,
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

pub fn matches_completion_prefix(candidate: &str, prefix: &str, ignore_spaces: bool) -> bool {
    matches_completion_prefix_with_threshold(candidate, prefix, ignore_spaces, 50)
}

pub fn matches_completion_prefix_with_threshold(
    candidate: &str,
    prefix: &str,
    ignore_spaces: bool,
    _match_threshold_percent: usize,
) -> bool {
    if prefix.is_empty() {
        return false;
    }
    if !ignore_spaces {
        return candidate.starts_with(prefix);
    }

    let compact_prefix = remove_spaces(prefix);
    let compact_candidate = remove_spaces(candidate);
    if compact_candidate.starts_with(&compact_prefix) {
        return true;
    }

    let mut candidate_words = candidate.split_whitespace();
    for prefix_part in prefix.split_whitespace() {
        let Some(candidate_word) = candidate_words.next() else {
            return false;
        };
        if !candidate_word.starts_with(prefix_part) {
            return false;
        }
    }
    true
}

fn percent(numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        return 0;
    }
    numerator * 100 / denominator
}

pub fn limit_candidates(
    mut candidates: Vec<CompletionCandidate>,
    max_results: usize,
) -> Vec<CompletionCandidate> {
    candidates.truncate(max_results);
    candidates
}

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
    token: &TokenContext,
    width: usize,
) -> Vec<String> {
    candidates
        .iter()
        .map(|candidate| {
            truncate_with_ellipsis(
                &format!(
                    "{}\t{}",
                    completion_candidate_label(candidate),
                    anchored_candidate_display(token, candidate)
                ),
                width,
            )
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
    let len = value.chars().count();
    if len <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let prefix: String = value.chars().take(width - 3).collect();
    format!("{prefix}...")
}

fn anchored_candidate_display(token: &TokenContext, candidate: &CompletionCandidate) -> String {
    if let Some(suffix) = candidate.replacement.strip_prefix(&token.text)
        && !suffix.is_empty()
    {
        return suffix.to_string();
    }
    candidate.replacement.clone()
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
        CompletionSource::History => "history",
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplatePlaceholderWord {
    raw: String,
    name: String,
}

fn template_placeholder_words(body: &str) -> Vec<TemplatePlaceholderWord> {
    split_shell_like_words(body)
        .into_iter()
        .filter_map(|word| {
            let name = template_word_placeholder_name(&word)?.to_string();
            Some(TemplatePlaceholderWord { raw: word, name })
        })
        .collect()
}

fn words_match_threshold(
    candidate_words: &[String],
    typed_words: &[String],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> bool {
    words_match_threshold_by(
        candidate_words,
        typed_words,
        match_threshold_percent,
        |candidate, typed| word_prefix_matches(candidate, typed, ignore_spaces),
    )
}

fn template_words_match_threshold(
    template_words: &[String],
    typed_words: &[String],
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> bool {
    words_match_threshold_by(
        template_words,
        typed_words,
        match_threshold_percent,
        |candidate, typed| {
            template_word_is_placeholder(candidate)
                || word_prefix_matches(candidate, typed, ignore_spaces)
        },
    )
}

fn words_match_threshold_with_typos(
    candidate_words: &[String],
    typed_words: &[String],
    ignore_spaces: bool,
    match_threshold_percent: usize,
    typo_threshold_percent: usize,
) -> bool {
    words_match_threshold_with_typo_usage_by(
        candidate_words,
        typed_words,
        match_threshold_percent,
        |candidate, typed| word_prefix_matches(candidate, typed, ignore_spaces),
        |candidate, typed| {
            typo_similarity_percent(candidate, typed, ignore_spaces) >= typo_threshold_percent
        },
    )
}

fn template_words_match_threshold_with_typos(
    template_words: &[String],
    typed_words: &[String],
    ignore_spaces: bool,
    match_threshold_percent: usize,
    typo_threshold_percent: usize,
) -> bool {
    words_match_threshold_with_typo_usage_by(
        template_words,
        typed_words,
        match_threshold_percent,
        |candidate, typed| {
            template_word_is_placeholder(candidate)
                || word_prefix_matches(candidate, typed, ignore_spaces)
        },
        |candidate, typed| {
            !template_word_is_placeholder(candidate)
                && typo_similarity_percent(candidate, typed, ignore_spaces)
                    >= typo_threshold_percent
        },
    )
}

fn words_match_threshold_by(
    candidate_words: &[String],
    typed_words: &[String],
    match_threshold_percent: usize,
    mut word_matches: impl FnMut(&str, &str) -> bool,
) -> bool {
    if typed_words.is_empty() || candidate_words.len() < typed_words.len() {
        return false;
    }
    let matched = typed_words
        .iter()
        .zip(candidate_words.iter())
        .filter(|(typed, candidate)| word_matches(candidate, typed))
        .count();
    percent(matched, typed_words.len()) >= match_threshold_percent.min(100)
}

fn words_match_threshold_with_typo_usage_by(
    candidate_words: &[String],
    typed_words: &[String],
    match_threshold_percent: usize,
    mut structural_matches: impl FnMut(&str, &str) -> bool,
    mut typo_matches: impl FnMut(&str, &str) -> bool,
) -> bool {
    if typed_words.is_empty() || candidate_words.len() < typed_words.len() {
        return false;
    }
    let mut matched = 0;
    let mut used_typo = false;
    for (typed, candidate) in typed_words.iter().zip(candidate_words.iter()) {
        if structural_matches(candidate, typed) {
            matched += 1;
        } else if typo_matches(candidate, typed) {
            matched += 1;
            used_typo = true;
        }
    }
    used_typo && percent(matched, typed_words.len()) >= match_threshold_percent.min(100)
}

fn word_prefix_matches(candidate: &str, typed: &str, ignore_spaces: bool) -> bool {
    if typed.is_empty() {
        return false;
    }
    if ignore_spaces {
        return remove_spaces(candidate).starts_with(&remove_spaces(typed));
    }
    candidate.starts_with(typed)
}

fn typo_similarity_percent(candidate: &str, typed: &str, ignore_spaces: bool) -> usize {
    let candidate = if ignore_spaces {
        remove_spaces(candidate)
    } else {
        candidate.to_string()
    };
    let typed = if ignore_spaces {
        remove_spaces(typed)
    } else {
        typed.to_string()
    };
    if candidate.is_empty() || typed.is_empty() {
        return 0;
    }
    let distance = edit_distance_chars(&candidate, &typed);
    let max_len = candidate.chars().count().max(typed.chars().count());
    percent(max_len.saturating_sub(distance), max_len)
}

fn edit_distance_chars(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

fn template_replacement_for_index(
    template_words: &[String],
    current_word_index: usize,
    token: &TokenContext,
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> String {
    let template_word = &template_words[current_word_index];
    let rest = &template_words[current_word_index + 1..];
    if token.text.is_empty() || !template_word_is_placeholder(template_word) {
        return join_words(&template_words[current_word_index..]);
    }

    let placeholder_name = template_word_placeholder_name(template_word).unwrap_or_default();
    if token.text.starts_with('{')
        || matches_completion_prefix_with_threshold(
            template_word,
            &token.text,
            ignore_spaces,
            match_threshold_percent,
        )
        || matches_completion_prefix_with_threshold(
            placeholder_name,
            &token.text,
            ignore_spaces,
            match_threshold_percent,
        )
    {
        return join_words(&template_words[current_word_index..]);
    }

    join_words_with_first(token.text.as_str(), rest)
}

fn template_word_is_placeholder(word: &str) -> bool {
    template_word_placeholder_name(word).is_some()
}

fn template_word_placeholder_name(word: &str) -> Option<&str> {
    let candidate = word.strip_prefix('{')?.strip_suffix('}')?;
    let name = candidate
        .strip_suffix("...")
        .or_else(|| candidate.split_once(':').map(|(name, _)| name))
        .unwrap_or(candidate);
    is_placeholder_name(name).then_some(name)
}

fn is_placeholder_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn join_words_with_first(first: &str, rest: &[String]) -> String {
    if rest.is_empty() {
        return first.to_string();
    }
    format!("{} {}", first, join_words(rest))
}

fn remove_spaces(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn command_arguments(command: &str) -> Vec<&str> {
    let mut arguments = Vec::new();
    let mut token_start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_index = 0;
    let mut token_seen = false;

    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            token_seen = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => {
                quote = None;
                token_seen = true;
            }
            Some(_) => {
                if ch == '\\' && quote == Some('"') {
                    escaped = true;
                }
                token_seen = true;
            }
            None if ch == '\\' => {
                escaped = true;
                token_seen = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_seen = true;
            }
            None if ch.is_whitespace() => {
                if token_seen {
                    if token_index > 0 {
                        arguments.push(command[token_start..index].trim_matches(['\'', '"']));
                    }
                    token_index += 1;
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    if token_seen && token_index > 0 {
        arguments.push(command[token_start..].trim_matches(['\'', '"']));
    }
    arguments
}

fn split_shell_like_words(command: &str) -> Vec<String> {
    command_arguments_with_first(command)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn command_arguments_with_first(command: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut token_start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_seen = false;

    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            token_seen = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => {
                quote = None;
                token_seen = true;
            }
            Some(_) => {
                if ch == '\\' && quote == Some('"') {
                    escaped = true;
                }
                token_seen = true;
            }
            None if ch == '\\' => {
                escaped = true;
                token_seen = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_seen = true;
            }
            None if ch.is_whitespace() => {
                if token_seen {
                    words.push(command[token_start..index].trim_matches(['\'', '"']));
                }
                token_seen = false;
                token_start = index + ch.len_utf8();
            }
            None => {
                token_seen = true;
            }
        }
    }

    if token_seen {
        words.push(command[token_start..].trim_matches(['\'', '"']));
    }
    words
}

fn join_words(words: &[String]) -> String {
    words.join(" ")
}

fn complete_path_executables(prefix: &str, path_dirs: &[PathBuf]) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for dir in path_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            if !file_name.starts_with(prefix) || !seen.insert(file_name.clone()) {
                continue;
            }
            let path = entry.path();
            if !is_executable_file(&path) {
                continue;
            }
            candidates.push(CompletionCandidate {
                display: file_name.clone(),
                replacement: file_name,
                is_dir: false,
                source: CompletionSource::Executable,
            });
        }
    }
    candidates.sort_by(|left, right| left.display.cmp(&right.display));
    candidates
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn strip_opening_quote(token: &str) -> (&str, &str) {
    if let Some(rest) = token.strip_prefix('\'') {
        ("'", rest)
    } else if let Some(rest) = token.strip_prefix('"') {
        ("\"", rest)
    } else {
        ("", token)
    }
}

fn split_path_token(token: &str) -> (&str, &str) {
    match token.rsplit_once('/') {
        Some((dir, prefix)) => (&token[..dir.len() + 1], prefix),
        None => ("", token),
    }
}

fn resolve_search_dir(dir_token: &str, cwd: &Path) -> Option<PathBuf> {
    if dir_token.is_empty() {
        return Some(cwd.to_path_buf());
    }
    if dir_token == "~/" || dir_token.starts_with("~/") {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        return Some(home.join(&dir_token[2..]));
    }
    let path = Path::new(dir_token);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        Some(cwd.join(path))
    }
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    if text.is_char_boundary(cursor) {
        return cursor;
    }
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_token_detects_first_token_prefix() {
        assert_eq!(
            current_token_context("git sta", 3),
            TokenContext {
                start: 0,
                end: 3,
                text: "git".to_string(),
                is_first_token: true,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_detects_non_first_token_at_cursor() {
        assert_eq!(
            current_token_context("git sta", 7),
            TokenContext {
                start: 4,
                end: 7,
                text: "sta".to_string(),
                is_first_token: false,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_keeps_quoted_whitespace_inside_token() {
        assert_eq!(
            current_token_context("echo \"hello wo", 14),
            TokenContext {
                start: 5,
                end: 14,
                text: "\"hello wo".to_string(),
                is_first_token: false,
                quote: Some('"'),
                path_like: false,
            }
        );
    }

    #[test]
    fn current_token_keeps_escaped_whitespace_inside_token() {
        assert_eq!(
            current_token_context("cd my\\ dir/fi", 13),
            TokenContext {
                start: 3,
                end: 13,
                text: "my\\ dir/fi".to_string(),
                is_first_token: false,
                quote: None,
                path_like: true,
            }
        );
    }

    #[test]
    fn current_token_handles_cursor_inside_line() {
        assert_eq!(
            current_token_context("git checkout main", 12),
            TokenContext {
                start: 4,
                end: 12,
                text: "checkout".to_string(),
                is_first_token: false,
                quote: None,
                path_like: false,
            }
        );
    }

    #[test]
    fn path_like_detection_covers_common_shell_path_prefixes() {
        for token in ["/tmp", "./src", "../src", "~/src", "src/main.rs", "'./src"] {
            assert!(is_path_like_token(token), "{token:?} should be path-like");
        }
        for token in ["git", "status", "--flag"] {
            assert!(
                !is_path_like_token(token),
                "{token:?} should not be path-like"
            );
        }
    }

    #[test]
    fn complete_path_returns_sorted_matching_file_and_directory_candidates() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("alpha.txt"), "").unwrap();
        std::fs::create_dir(temp.path().join("app")).unwrap();
        std::fs::write(temp.path().join("beta.txt"), "").unwrap();

        assert_eq!(
            complete_path("a", temp.path()),
            [
                CompletionCandidate {
                    display: "alpha.txt".to_string(),
                    replacement: "alpha.txt".to_string(),
                    is_dir: false,
                    source: CompletionSource::Path,
                },
                CompletionCandidate {
                    display: "app/".to_string(),
                    replacement: "app/".to_string(),
                    is_dir: true,
                    source: CompletionSource::Path,
                },
            ]
        );
    }

    #[test]
    fn complete_path_uses_relative_directory_prefix() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        assert_eq!(
            complete_path("src/m", temp.path()),
            [CompletionCandidate {
                display: "src/main.rs".to_string(),
                replacement: "src/main.rs".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            }]
        );
    }

    #[test]
    fn complete_path_preserves_opening_quote_in_replacement_only() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("my file.txt"), "").unwrap();

        assert_eq!(
            complete_path("'my", temp.path()),
            [CompletionCandidate {
                display: "my file.txt".to_string(),
                replacement: "'my file.txt".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            }]
        );
    }

    #[test]
    fn complete_first_token_orders_templates_history_then_executables() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("git-now");
        std::fs::write(&executable, "#!/bin/sh\n").unwrap();
        make_executable(&executable);
        let templates = vec![TemplateEntry::new("git add . && git commit")];
        let history = vec![HistoryEntry {
            t: 2,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        assert_eq!(
            complete_first_token("git", &templates, &history, &[bin]),
            [
                CompletionCandidate {
                    display: "git add . && git commit".to_string(),
                    replacement: "git add . && git commit".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "git status".to_string(),
                    replacement: "git status".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "git-now".to_string(),
                    replacement: "git-now".to_string(),
                    is_dir: false,
                    source: CompletionSource::Executable,
                },
            ]
        );
    }

    #[test]
    fn complete_first_token_deduplicates_each_source() {
        let templates = vec![
            TemplateEntry::new("docker deploy"),
            TemplateEntry::new("docker deploy"),
        ];
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "docker ps".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "docker ps".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        assert_eq!(
            complete_first_token("d", &templates, &history, &[]),
            [
                CompletionCandidate {
                    display: "docker deploy".to_string(),
                    replacement: "docker deploy".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "docker ps".to_string(),
                    replacement: "docker ps".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
            ]
        );
    }

    #[test]
    fn complete_first_token_can_match_while_ignoring_spaces_and_limit_results() {
        let templates = vec![TemplateEntry::new("git add . && git commit")];
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "git stash".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        assert_eq!(
            complete_first_token_with_options(
                "g s",
                &templates,
                &history,
                &[],
                CompletionOptions {
                    max_results: 2,
                    ignore_spaces: true,
                    fuzzy_enabled: true,
                    match_threshold_percent: 50,
                    typo_threshold_percent: 80,
                },
            ),
            [
                CompletionCandidate {
                    display: "git status".to_string(),
                    replacement: "git status".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "git stash".to_string(),
                    replacement: "git stash".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
            ]
        );
    }

    #[test]
    fn complete_non_first_token_orders_history_arguments_before_path_candidates() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
        let history = vec![HistoryEntry {
            t: 2,
            command: "git add src/lib.rs".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        assert_eq!(
            complete_non_first_token("src/", temp.path(), &history, &[]),
            [
                CompletionCandidate {
                    display: "src/lib.rs".to_string(),
                    replacement: "src/lib.rs".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "src/main.rs".to_string(),
                    replacement: "src/main.rs".to_string(),
                    is_dir: false,
                    source: CompletionSource::Path,
                },
            ]
        );
    }

    #[test]
    fn complete_non_first_token_includes_plain_path_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("one.txt"), "").unwrap();

        let candidates = complete_non_first_token_with_options(
            "o",
            temp.path(),
            &[],
            &[],
            CompletionOptions::default(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].display, "one.txt");
        assert_eq!(candidates[0].source, CompletionSource::Path);
    }

    #[test]
    fn complete_non_first_token_includes_history_arguments_without_path_prefix() {
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "kubectl get pods".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "docker get pods".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        let templates = vec![TemplateEntry::new("kubectl logs {pod_name}")];

        assert_eq!(
            complete_non_first_token("po", Path::new("/"), &history, &templates),
            [
                CompletionCandidate {
                    display: "{pod_name}".to_string(),
                    replacement: "{pod_name}".to_string(),
                    is_dir: false,
                    source: CompletionSource::TemplatePlaceholder,
                },
                CompletionCandidate {
                    display: "pods".to_string(),
                    replacement: "pods".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                }
            ]
        );
    }

    #[test]
    fn complete_non_first_token_applies_options_to_history_and_placeholders() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "git commit featurebranch".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];
        let templates = vec![TemplateEntry::new("git checkout {featurebranch}")];

        assert_eq!(
            complete_non_first_token_with_options(
                "feature b",
                Path::new("/"),
                &history,
                &templates,
                CompletionOptions {
                    max_results: 1,
                    ignore_spaces: true,
                    fuzzy_enabled: true,
                    match_threshold_percent: 50,
                    typo_threshold_percent: 80,
                },
            ),
            [CompletionCandidate {
                display: "{featurebranch}".to_string(),
                replacement: "{featurebranch}".to_string(),
                is_dir: false,
                source: CompletionSource::TemplatePlaceholder,
            }]
        );
    }

    #[test]
    fn strict_structural_threshold_filters_current_position_mismatch() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "command add 100 file".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let candidates = complete_non_first_token_for_line_with_options(
            "command add 200",
            "command add 200".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions {
                match_threshold_percent: 80,
                ..CompletionOptions::default()
            },
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn completion_match_threshold_filters_weak_partial_matches() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let default_threshold_candidates = complete_non_first_token_for_line_with_options(
            "git stx",
            "git stx".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions::default(),
        );
        assert_eq!(
            default_threshold_candidates.first(),
            Some(&CompletionCandidate {
                display: "status --short".to_string(),
                replacement: "status --short".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            })
        );

        let strict_threshold_candidates = complete_non_first_token_for_line_with_options(
            "git stx",
            "git stx".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions {
                match_threshold_percent: 80,
                ..CompletionOptions::default()
            },
        );
        assert!(strict_threshold_candidates.is_empty());
    }

    #[test]
    fn trailing_space_completes_structural_history_without_path_noise() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("local-file"), "").unwrap();
        let history = vec![
            HistoryEntry {
                t: 2,
                command: "git commit -m message".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            HistoryEntry {
                t: 1,
                command: "git status --short".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ];

        let candidates = complete_non_first_token_for_line_with_options(
            "git ",
            "git ".len(),
            temp.path(),
            &history,
            &[],
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates,
            [
                CompletionCandidate {
                    display: "commit -m message".to_string(),
                    replacement: "commit -m message".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
                CompletionCandidate {
                    display: "status --short".to_string(),
                    replacement: "status --short".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                },
            ]
        );
    }

    #[test]
    fn trailing_space_uses_previous_word_match_threshold() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let candidates = complete_non_first_token_for_line_with_options(
            "gi ",
            "gi ".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates,
            [CompletionCandidate {
                display: "status --short".to_string(),
                replacement: "status --short".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            }]
        );

        let strict_candidates = complete_non_first_token_for_line_with_options(
            "gix ",
            "gix ".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions::default(),
        );
        assert!(strict_candidates.is_empty());
    }

    #[test]
    fn trailing_space_requires_structural_prefix_match() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let candidates = complete_non_first_token_for_line_with_options(
            "gix ",
            "gix ".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions::default(),
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn trailing_space_prefers_structural_templates() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "kubectl get pods".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];
        let templates = vec![TemplateEntry::new("kubectl get {resource}")];

        let candidates = complete_non_first_token_for_line_with_options(
            "kubectl get ",
            "kubectl get ".len(),
            Path::new("/"),
            &history,
            &templates,
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates,
            [
                CompletionCandidate {
                    display: "kubectl get {resource}".to_string(),
                    replacement: "{resource}".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "pods".to_string(),
                    replacement: "pods".to_string(),
                    is_dir: false,
                    source: CompletionSource::History,
                }
            ]
        );
    }

    #[test]
    fn trailing_space_templates_use_previous_word_match_threshold() {
        let templates = vec![TemplateEntry::new("kubectl get {resource}")];

        let candidates = complete_non_first_token_for_line_with_options(
            "kubectl g ",
            "kubectl g ".len(),
            Path::new("/"),
            &[],
            &templates,
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates,
            [CompletionCandidate {
                display: "kubectl get {resource}".to_string(),
                replacement: "{resource}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            }]
        );
    }

    #[test]
    fn private_command_completion_uses_aish_commands_only() {
        let candidates = complete_private_commands("#sta", usize::MAX);

        assert_eq!(
            candidates,
            [CompletionCandidate {
                display: "#status".to_string(),
                replacement: "#status".to_string(),
                is_dir: false,
                source: CompletionSource::PrivateCommand,
            }]
        );
        assert!(complete_private_commands("#", usize::MAX).is_empty());
        assert!(complete_private_commands("# ", usize::MAX).is_empty());
    }

    #[test]
    fn complete_non_first_token_for_line_matches_template_placeholder_name_without_braces() {
        let templates = vec![TemplateEntry::new("echo {something}")];

        let candidates = complete_non_first_token_for_line_with_options(
            "echo something",
            "echo something".len(),
            Path::new("/"),
            &[],
            &templates,
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates.first(),
            Some(&CompletionCandidate {
                display: "echo {something}".to_string(),
                replacement: "{something}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            })
        );
    }

    #[test]
    fn complete_non_first_token_for_line_treats_template_as_whole_command_shape() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "command add 100 other".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];
        let templates = vec![TemplateEntry::new("command add {amount} file")];

        let candidates = complete_non_first_token_for_line_with_options(
            "command add 200",
            "command add 200".len(),
            Path::new("/"),
            &history,
            &templates,
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates.first(),
            Some(&CompletionCandidate {
                display: "command add {amount} file".to_string(),
                replacement: "200 file".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            })
        );
    }

    #[test]
    fn complete_non_first_token_for_line_prefers_structural_template_position() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "echo {a} {something}".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];
        let templates = vec![
            TemplateEntry::new("echo {a} {older}"),
            TemplateEntry::new("echo {a} {b} {c}"),
        ];

        let candidates = complete_non_first_token_for_line_with_options(
            "echo {a} {something}",
            "echo {a} {something}".len(),
            Path::new("/"),
            &history,
            &templates,
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates,
            [
                CompletionCandidate {
                    display: "echo {a} {b} {c}".to_string(),
                    replacement: "{b} {c}".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
                CompletionCandidate {
                    display: "echo {a} {older}".to_string(),
                    replacement: "{older}".to_string(),
                    is_dir: false,
                    source: CompletionSource::Template,
                },
            ]
        );
    }

    #[test]
    fn complete_non_first_token_for_line_keeps_whole_history_suffix() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "echo word-alpha word-beta word-gamma".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let candidates = complete_non_first_token_for_line_with_options(
            "echo word",
            "echo word".len(),
            Path::new("/"),
            &history,
            &[],
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates.first(),
            Some(&CompletionCandidate {
                display: "word-alpha word-beta word-gamma".to_string(),
                replacement: "word-alpha word-beta word-gamma".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            })
        );
    }

    #[test]
    fn matches_completion_prefix_can_ignore_spaces() {
        assert!(matches_completion_prefix("git status", "g s", true));
        assert!(!matches_completion_prefix("git status", "g s", false));
        assert!(!matches_completion_prefix_with_threshold(
            "git status",
            "",
            true,
            0
        ));
        assert!(!matches_completion_prefix_with_threshold(
            "git status",
            "g x",
            true,
            50
        ));
        assert!(!matches_completion_prefix_with_threshold(
            "git status",
            "gs",
            true,
            50
        ));
        assert!(!matches_completion_prefix_with_threshold(
            "git status",
            "gs",
            true,
            49
        ));
        assert!(!matches_completion_prefix_with_threshold(
            "status", "stx", true, 50
        ));
    }

    #[test]
    fn typo_candidates_use_dedicated_typo_threshold() {
        let history = vec![HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let candidates = complete_non_first_token_typos_for_line_with_options(
            "git statuz",
            "git statuz".len(),
            &history,
            &[],
            CompletionOptions::default(),
        );

        assert_eq!(
            candidates.first(),
            Some(&CompletionCandidate {
                display: "status --short".to_string(),
                replacement: "status --short".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            })
        );

        let strict_candidates = complete_non_first_token_typos_for_line_with_options(
            "git statuz",
            "git statuz".len(),
            &history,
            &[],
            CompletionOptions {
                typo_threshold_percent: 90,
                ..CompletionOptions::default()
            },
        );
        assert!(strict_candidates.is_empty());

        let disabled_candidates = complete_non_first_token_typos_for_line_with_options(
            "git statuz",
            "git statuz".len(),
            &history,
            &[],
            CompletionOptions {
                fuzzy_enabled: false,
                ..CompletionOptions::default()
            },
        );
        assert!(disabled_candidates.is_empty());
    }

    #[test]
    fn render_completion_candidates_labels_sources_without_mutating_input() {
        let candidates = vec![
            CompletionCandidate {
                display: "deploy".to_string(),
                replacement: "kubectl apply -f {file}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
            CompletionCandidate {
                display: "src/main.rs".to_string(),
                replacement: "src/main.rs".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
        ];

        assert_eq!(
            render_completion_candidates(&candidates),
            ["template\tdeploy", "file\tsrc/main.rs"]
        );
    }

    #[test]
    fn render_completion_candidates_labels_directories_separately_from_files() {
        let candidates = vec![
            CompletionCandidate {
                display: "src/".to_string(),
                replacement: "src/".to_string(),
                is_dir: true,
                source: CompletionSource::Path,
            },
            CompletionCandidate {
                display: "src/main.rs".to_string(),
                replacement: "src/main.rs".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
        ];

        assert_eq!(
            render_completion_candidates(&candidates),
            ["dir\tsrc/", "file\tsrc/main.rs"]
        );
    }

    #[test]
    fn render_completion_candidates_for_width_elides_without_wrapping() {
        let token = current_token_context("cat very-long", "cat very-long".len());
        let candidates = vec![CompletionCandidate {
            display: "very-long-file-name-that-will-not-fit.txt".to_string(),
            replacement: "very-long-file-name-that-will-not-fit.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }];

        let rows = render_completion_candidates_for_width(&candidates, &token, 24);

        assert_eq!(rows, ["file\t-file-name-that-..."]);
        assert!(rows[0].chars().count() <= 24);
    }

    #[test]
    fn render_completion_candidates_for_width_keeps_source_label_when_possible() {
        let token = current_token_context("git", "git".len());
        let candidates = vec![CompletionCandidate {
            display: "git status --short".to_string(),
            replacement: "git status --short".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        }];

        let rows = render_completion_candidates_for_width(&candidates, &token, 80);

        assert_eq!(rows, ["history\t status --short"]);
    }

    #[test]
    fn render_completion_candidates_for_width_shows_replacement_for_non_suffix_candidates() {
        let token = current_token_context("echo {a} {something}", "echo {a} {something}".len());
        let candidates = vec![CompletionCandidate {
            display: "echo {a} {b} {c}".to_string(),
            replacement: "{b} {c}".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        }];

        let rows = render_completion_candidates_for_width(&candidates, &token, 80);

        assert_eq!(rows, ["template\t{b} {c}"]);
    }

    #[test]
    fn ghost_completion_suffix_is_display_only_tail() {
        let token = current_token_context("git sta", "git sta".len());
        let candidate = CompletionCandidate {
            display: "status".to_string(),
            replacement: "status".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        };

        assert_eq!(
            ghost_completion_suffix(&token, &candidate),
            Some("tus".to_string())
        );
    }

    #[test]
    fn ghost_completion_suffix_works_across_completion_sources() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("mytool");
        std::fs::write(&executable, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions).unwrap();
        }
        let templates = vec![TemplateEntry::new("git add . && git commit")];
        let history = vec![HistoryEntry {
            t: 1,
            command: "git checkout feature/test".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }];

        let template_token = current_token_context("git", 3);
        let template = complete_first_token("git", &templates, &[], &[]);
        assert_eq!(
            ghost_completion_suffix(&template_token, &template[0]),
            Some(" add . && git commit".to_string())
        );

        let executable_token = current_token_context("my", 2);
        let executable = complete_first_token("my", &[], &[], &[bin]);
        assert_eq!(
            ghost_completion_suffix(&executable_token, &executable[0]),
            Some("tool".to_string())
        );

        let path_token = current_token_context("cat sr", "cat sr".len());
        let path = complete_path("sr", temp.path());
        assert_eq!(
            ghost_completion_suffix(&path_token, &path[0]),
            Some("c/".to_string())
        );

        let argument_token = current_token_context("git checkout fea", "git checkout fea".len());
        let argument = complete_non_first_token("fea", temp.path(), &history, &[]);
        assert_eq!(
            ghost_completion_suffix(&argument_token, &argument[0]),
            Some("ture/test".to_string())
        );
    }

    #[test]
    fn accept_completion_replaces_token_and_returns_new_cursor() {
        let line = "git sta --short";
        let token = current_token_context(line, 7);
        let candidate = CompletionCandidate {
            display: "status".to_string(),
            replacement: "status".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        };

        assert_eq!(
            accept_completion(line, &token, &candidate),
            AcceptedCompletion {
                line: "git status --short".to_string(),
                cursor: 10,
            }
        );
    }

    #[test]
    fn accept_completion_word_mode_stops_at_next_word_boundary() {
        let line = "kub";
        let token = current_token_context(line, line.len());
        let candidate = CompletionCandidate {
            display: "kubectl apply -f file.yaml".to_string(),
            replacement: "kubectl apply -f file.yaml".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        };

        assert_eq!(
            accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
            AcceptedCompletion {
                line: "kubectl".to_string(),
                cursor: "kubectl".len(),
            }
        );
    }

    #[test]
    fn accept_completion_word_mode_includes_leading_space_and_next_word() {
        let line = "git";
        let token = current_token_context(line, line.len());
        let candidate = CompletionCandidate {
            display: "git status --short".to_string(),
            replacement: "git status --short".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        };

        assert_eq!(
            accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
            AcceptedCompletion {
                line: "git status".to_string(),
                cursor: "git status".len(),
            }
        );
    }

    #[test]
    fn accept_completion_word_mode_uses_full_suffix_without_boundary() {
        let line = "cat Car";
        let token = current_token_context(line, line.len());
        let candidate = CompletionCandidate {
            display: "Cargo.toml".to_string(),
            replacement: "Cargo.toml".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        };

        assert_eq!(
            accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
            AcceptedCompletion {
                line: "cat Cargo.toml".to_string(),
                cursor: "cat Cargo.toml".len(),
            }
        );
    }

    #[test]
    fn accept_completion_word_mode_stops_at_next_word_for_non_prefix_replacement() {
        let line = "echo {a} {something}";
        let token = current_token_context(line, line.len());
        let candidate = CompletionCandidate {
            display: "echo {a} {b} {c}".to_string(),
            replacement: "{b} {c}".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        };

        assert_eq!(
            accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
            AcceptedCompletion {
                line: "echo {a} {b}".to_string(),
                cursor: "echo {a} {b}".len(),
            }
        );
    }

    #[test]
    fn command_arguments_preserve_quoted_argument_spaces() {
        assert_eq!(
            command_arguments("git commit -m 'hello world' -- file"),
            ["commit", "-m", "hello world", "--", "file"]
        );
    }

    #[test]
    fn complete_path_returns_empty_for_missing_directory() {
        let temp = tempfile::tempdir().unwrap();

        assert!(complete_path("missing/file", temp.path()).is_empty());
    }

    #[test]
    fn cursor_is_snapped_to_previous_utf8_boundary() {
        assert_eq!(current_token_context("echo λ", 6).end, 5);
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}
}
