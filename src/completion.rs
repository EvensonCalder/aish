use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::commands::{HELP_TOPICS, IMPLEMENTED_PRIVATE_COMMANDS};
use crate::config::CompletionTabAccept;
use crate::display_width::{
    display_width, truncate_end_with_ellipsis, truncate_start_with_ellipsis,
};
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

const PATH_COMPLETION_CACHE_MAX_AGE: Duration = Duration::from_millis(250);
const PATH_COMPLETION_CACHE_MAX_DIRS: usize = 128;

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
    complete_path_internal(token, cwd, None)
}

fn complete_path_with_options(
    token: &str,
    cwd: &Path,
    options: CompletionOptions,
) -> Vec<CompletionCandidate> {
    complete_path_internal(token, cwd, Some(options))
}

fn complete_path_internal(
    token: &str,
    cwd: &Path,
    typo_options: Option<CompletionOptions>,
) -> Vec<CompletionCandidate> {
    let (quote, token) = strip_opening_quote(token);
    let (dir_token, prefix) = split_path_token(token);
    let Some(search_dir) = resolve_search_dir(dir_token, cwd) else {
        return Vec::new();
    };
    let entries = cached_path_entries(&search_dir);

    let mut candidates = Vec::new();
    for entry in &entries {
        if !entry.name.starts_with(prefix) {
            continue;
        }
        let suffix = if entry.is_dir { "/" } else { "" };
        let replacement = format!("{quote}{dir_token}{}{suffix}", entry.name);
        candidates.push(CompletionCandidate {
            display: format!("{dir_token}{}{suffix}", entry.name),
            replacement,
            is_dir: entry.is_dir,
            source: CompletionSource::Path,
        });
    }

    let has_prefix_directory = candidates.iter().any(|candidate| candidate.is_dir);
    if !has_prefix_directory
        && let Some(options) = typo_options
        && options.fuzzy_enabled
    {
        for entry in &entries {
            if !entry.is_dir
                || entry.name.starts_with(prefix)
                || !directory_typo_matches(&entry.name, prefix, options)
            {
                continue;
            }
            candidates.push(CompletionCandidate {
                display: format!("{dir_token}{}/", entry.name),
                replacement: format!("{quote}{dir_token}{}/", entry.name),
                is_dir: true,
                source: CompletionSource::Path,
            });
        }
    }
    candidates.sort_by(|left, right| left.display.cmp(&right.display));
    dedupe_completion_candidates(&mut candidates);
    candidates
}

#[derive(Debug, Clone)]
struct PathEntry {
    name: String,
    is_dir: bool,
}

#[derive(Debug, Clone)]
struct PathCompletionCacheEntry {
    entries: Vec<PathEntry>,
    read_at: Instant,
    modified: Option<SystemTime>,
}

static PATH_COMPLETION_CACHE: OnceLock<Mutex<HashMap<PathBuf, PathCompletionCacheEntry>>> =
    OnceLock::new();

fn cached_path_entries(search_dir: &Path) -> Vec<PathEntry> {
    let key = search_dir.to_path_buf();
    let now = Instant::now();
    let modified = directory_modified(search_dir);
    let cache = PATH_COMPLETION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(cache) = cache.lock()
        && let Some(entry) = cache.get(&key)
        && entry.modified == modified
        && now.saturating_duration_since(entry.read_at) <= PATH_COMPLETION_CACHE_MAX_AGE
    {
        return entry.entries.clone();
    }

    let entries = read_path_entries(search_dir);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(
            key,
            PathCompletionCacheEntry {
                entries: entries.clone(),
                read_at: now,
                modified,
            },
        );
        prune_path_completion_cache(&mut cache);
    }
    entries
}

fn directory_modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn read_path_entries(search_dir: &Path) -> Vec<PathEntry> {
    let Ok(entries) = fs::read_dir(search_dir) else {
        return Vec::new();
    };
    let mut path_entries = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false);
        path_entries.push(PathEntry { name, is_dir });
    }
    path_entries.sort_by(|left, right| left.name.cmp(&right.name));
    path_entries
}

fn prune_path_completion_cache(cache: &mut HashMap<PathBuf, PathCompletionCacheEntry>) {
    while cache.len() > PATH_COMPLETION_CACHE_MAX_DIRS {
        let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.read_at)
            .map(|(path, _)| path.clone())
        else {
            return;
        };
        cache.remove(&oldest);
    }
}

fn directory_typo_matches(candidate: &str, typed: &str, options: CompletionOptions) -> bool {
    if typed.chars().count() < 3 {
        return false;
    }
    if typo_similarity_percent(candidate, typed, options.ignore_spaces)
        >= options.typo_threshold_percent
    {
        return true;
    }
    let candidate_len = candidate.chars().count();
    let typed_len = typed.chars().count();
    candidate.chars().next() == typed.chars().next()
        && candidate_len.min(typed_len) >= 3
        && candidate_len.abs_diff(typed_len) <= 1
        && edit_distance_chars(candidate, typed) <= 1
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
        ("help", []) => HELP_TOPICS,
        ("key", []) => &["set", "clear"],
        ("prompt", []) => &["draft", "history", "ai", "reset"],
        ("context", []) => &["on", "off", "confirm"],
        ("context", ["confirm"]) => &["on", "off"],
        ("template", []) => &["find", "list", "rm", "replace", "show", "use"],
        ("encrypt", []) => &["on", "off", "rotate", "rewrite-history"],
        ("encrypt", ["rewrite-history"]) => &["plan", "run"],
        ("sync", []) => &["off", "ai", "history", "templates", "drafts"],
        ("sync", ["ai" | "history" | "templates" | "drafts"]) => &["on", "off"],
        _ => &[],
    };
    candidates
        .iter()
        .copied()
        .filter(|candidate| prefix.is_empty() || candidate.starts_with(prefix))
        .collect()
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

fn order_path_candidates_for_completion(
    candidates: Vec<CompletionCandidate>,
) -> Vec<CompletionCandidate> {
    let (mut directories, files) = split_path_candidates(candidates);
    directories.extend(files);
    directories
}

fn split_path_candidates(
    candidates: Vec<CompletionCandidate>,
) -> (Vec<CompletionCandidate>, Vec<CompletionCandidate>) {
    candidates
        .into_iter()
        .partition(|candidate| candidate.source == CompletionSource::Path && candidate.is_dir)
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

fn completion_candidate_replaces_whole_line(candidate: &CompletionCandidate) -> bool {
    matches!(
        candidate.source,
        CompletionSource::TemplateTypo | CompletionSource::HistoryTypo
    )
}

pub fn truncate_with_ellipsis(value: &str, width: usize) -> String {
    truncate_end_with_ellipsis(value, width)
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
    let executables = scan_path_executables(path_dirs);
    complete_first_token_executables_from_names_with_options(
        prefix,
        &executables,
        CompletionOptions {
            max_results: usize::MAX,
            ignore_spaces: false,
            fuzzy_enabled: true,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        },
    )
}

pub(crate) fn scan_path_executables(path_dirs: &[PathBuf]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for dir in path_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            if !seen.insert(file_name.clone()) {
                continue;
            }
            let path = entry.path();
            if !is_executable_file(&path) {
                continue;
            }
            names.push(file_name);
        }
    }
    names.sort();
    names
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
mod tests;
