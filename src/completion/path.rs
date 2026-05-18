use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use super::matching::{edit_distance_chars, typo_similarity_percent};
use super::parser::{resolve_search_dir, shell_word_value, split_path_token};
use super::{
    CompletionCandidate, CompletionOptions, CompletionSource,
    complete_first_token_executables_from_names_with_options, dedupe_completion_candidates,
};

const PATH_COMPLETION_CACHE_MAX_AGE: Duration = Duration::from_millis(250);
const PATH_COMPLETION_CACHE_MAX_DIRS: usize = 128;
const PATH_COMPLETION_MAX_COMPONENT_BASES: usize = 32;

pub fn complete_path(token: &str, cwd: &Path) -> Vec<CompletionCandidate> {
    complete_path_internal(token, cwd, None)
}

pub(super) fn complete_path_with_options(
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
    let quote = token.chars().next().filter(|ch| matches!(ch, '\'' | '"'));
    let home_tilde_active = home_tilde_expansion_is_active(token);
    let token_value = shell_word_value(token);
    let (dir_token, prefix) = split_path_token(&token_value);
    let Some(search_dir) = resolve_search_dir(dir_token, cwd, home_tilde_active) else {
        return Vec::new();
    };
    let literal_leading_tilde = token_value.starts_with("~/") && !home_tilde_active;
    let bases = if search_dir.is_dir() {
        vec![PathCompletionBase {
            search_dir,
            display_dir: dir_token.to_string(),
        }]
    } else {
        component_completion_bases(
            dir_token,
            cwd,
            home_tilde_active,
            typo_options.unwrap_or_default(),
        )
    };

    let mut candidates =
        complete_path_from_bases(&bases, prefix, quote, literal_leading_tilde, typo_options);
    sort_path_candidates(&mut candidates);
    dedupe_completion_candidates(&mut candidates);
    candidates
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathCompletionBase {
    search_dir: PathBuf,
    display_dir: String,
}

fn complete_path_from_bases(
    bases: &[PathCompletionBase],
    prefix: &str,
    quote: Option<char>,
    literal_leading_tilde: bool,
    typo_options: Option<CompletionOptions>,
) -> Vec<CompletionCandidate> {
    let mut candidates = Vec::new();
    let mut seen_prefix_directory = false;

    for base in bases {
        let entries = cached_path_entries(&base.search_dir);
        for entry in &entries {
            if !entry.name.starts_with(prefix) {
                continue;
            }
            if entry.is_dir {
                seen_prefix_directory = true;
            }
            let display = format_path_display(&base.display_dir, &entry.name, entry.is_dir);
            candidates.push(CompletionCandidate {
                replacement: path_replacement(quote, &display, literal_leading_tilde),
                display,
                is_dir: entry.is_dir,
                source: CompletionSource::Path,
            });
        }
    }

    if seen_prefix_directory {
        return candidates;
    }

    let Some(options) = typo_options else {
        return candidates;
    };
    if !options.fuzzy_enabled {
        return candidates;
    }

    for base in bases {
        let entries = cached_path_entries(&base.search_dir);
        for entry in &entries {
            if !entry.is_dir
                || entry.name.starts_with(prefix)
                || !directory_typo_matches(&entry.name, prefix, options)
            {
                continue;
            }
            let display = format_path_display(&base.display_dir, &entry.name, true);
            candidates.push(CompletionCandidate {
                replacement: path_replacement(quote, &display, literal_leading_tilde),
                display,
                is_dir: true,
                source: CompletionSource::Path,
            });
        }
    }

    candidates
}

fn component_completion_bases(
    dir_token: &str,
    cwd: &Path,
    expand_home_tilde: bool,
    options: CompletionOptions,
) -> Vec<PathCompletionBase> {
    let Some(root) = component_completion_root(dir_token, cwd, expand_home_tilde) else {
        return Vec::new();
    };
    let mut bases = vec![PathCompletionBase {
        search_dir: root.search_dir,
        display_dir: root.display_dir,
    }];

    for component in root.components {
        let mut next = Vec::new();
        for base in &bases {
            next.extend(intermediate_directory_matches(base, &component, options));
            if next.len() >= PATH_COMPLETION_MAX_COMPONENT_BASES {
                break;
            }
        }
        dedupe_path_completion_bases(&mut next);
        next.truncate(PATH_COMPLETION_MAX_COMPONENT_BASES);
        if next.is_empty() {
            return Vec::new();
        }
        bases = next;
    }

    bases
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComponentCompletionRoot {
    search_dir: PathBuf,
    display_dir: String,
    components: Vec<String>,
}

fn component_completion_root(
    dir_token: &str,
    cwd: &Path,
    expand_home_tilde: bool,
) -> Option<ComponentCompletionRoot> {
    let trimmed = dir_token.trim_end_matches('/');
    if trimmed.is_empty() {
        return Some(ComponentCompletionRoot {
            search_dir: cwd.to_path_buf(),
            display_dir: String::new(),
            components: Vec::new(),
        });
    }

    if expand_home_tilde && let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        return Some(ComponentCompletionRoot {
            search_dir: home,
            display_dir: "~/".to_string(),
            components: path_components(rest),
        });
    }

    if trimmed == "~" {
        return Some(ComponentCompletionRoot {
            search_dir: cwd.to_path_buf(),
            display_dir: String::new(),
            components: vec!["~".to_string()],
        });
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return Some(ComponentCompletionRoot {
            search_dir: PathBuf::from("/"),
            display_dir: "/".to_string(),
            components: path_components(rest),
        });
    }

    Some(ComponentCompletionRoot {
        search_dir: cwd.to_path_buf(),
        display_dir: String::new(),
        components: path_components(trimmed),
    })
}

fn path_components(value: &str) -> Vec<String> {
    value
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect()
}

fn intermediate_directory_matches(
    base: &PathCompletionBase,
    component: &str,
    options: CompletionOptions,
) -> Vec<PathCompletionBase> {
    if component.is_empty() {
        return vec![base.clone()];
    }

    let exact_path = base.search_dir.join(component);
    if exact_path.is_dir() {
        return vec![PathCompletionBase {
            search_dir: exact_path,
            display_dir: format_path_display(&base.display_dir, component, true),
        }];
    }

    let entries = cached_path_entries(&base.search_dir);
    let mut matches = Vec::new();
    for entry in &entries {
        if !entry.is_dir || !entry.name.starts_with(component) {
            continue;
        }
        matches.push(PathCompletionBase {
            search_dir: base.search_dir.join(&entry.name),
            display_dir: format_path_display(&base.display_dir, &entry.name, true),
        });
    }

    if !matches.is_empty() || !options.fuzzy_enabled {
        sort_path_completion_bases(&mut matches);
        return matches;
    }

    for entry in &entries {
        if !entry.is_dir
            || entry.name.starts_with(component)
            || !directory_typo_matches(&entry.name, component, options)
        {
            continue;
        }
        matches.push(PathCompletionBase {
            search_dir: base.search_dir.join(&entry.name),
            display_dir: format_path_display(&base.display_dir, &entry.name, true),
        });
    }
    sort_path_completion_bases(&mut matches);
    matches
}

fn format_path_display(display_dir: &str, name: &str, is_dir: bool) -> String {
    let mut display = String::with_capacity(display_dir.len() + name.len() + usize::from(is_dir));
    display.push_str(display_dir);
    display.push_str(name);
    if is_dir {
        display.push('/');
    }
    display
}

fn home_tilde_expansion_is_active(raw_token: &str) -> bool {
    raw_token.starts_with("~/")
}

fn path_replacement(quote: Option<char>, value: &str, literal_leading_tilde: bool) -> String {
    match quote {
        Some('\'') => single_quoted_path(value),
        Some('"') => double_quoted_path(value),
        _ => unquoted_path(value, literal_leading_tilde),
    }
}

fn single_quoted_path(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn double_quoted_path(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        if matches!(ch, '"' | '\\' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

fn unquoted_path(value: &str, literal_leading_tilde: bool) -> String {
    if value.contains('\n') {
        return single_quoted_path(value);
    }

    let mut escaped = String::with_capacity(value.len());
    for (index, ch) in value.chars().enumerate() {
        if index == 0 && ch == '~' && literal_leading_tilde {
            escaped.push('\\');
            escaped.push(ch);
            continue;
        }
        if is_unquoted_path_safe_char(ch) {
            escaped.push(ch);
        } else {
            escaped.push('\\');
            escaped.push(ch);
        }
    }
    escaped
}

fn is_unquoted_path_safe_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(ch, '/' | '.' | '_' | '-' | ':' | '~')
        || (!ch.is_ascii() && !ch.is_whitespace() && !ch.is_control())
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
        let is_dir = path_entry_is_directory(&entry);
        path_entries.push(PathEntry { name, is_dir });
    }
    path_entries.sort_by(|left, right| left.name.cmp(&right.name));
    path_entries
}

fn path_entry_is_directory(entry: &fs::DirEntry) -> bool {
    let Ok(file_type) = entry.file_type() else {
        return false;
    };
    if file_type.is_dir() {
        return true;
    }
    if !file_type.is_symlink() {
        return false;
    }
    fs::metadata(entry.path())
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
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

fn sort_path_candidates(candidates: &mut [CompletionCandidate]) {
    candidates.sort_by(|left, right| {
        path_candidate_is_hidden(left)
            .cmp(&path_candidate_is_hidden(right))
            .then(left.display.cmp(&right.display))
    });
}

fn path_candidate_is_hidden(candidate: &CompletionCandidate) -> bool {
    last_path_component(&candidate.display).is_some_and(|component| {
        component.starts_with('.') && component != "." && component != ".."
    })
}

fn sort_path_completion_bases(bases: &mut [PathCompletionBase]) {
    bases.sort_by(|left, right| {
        path_display_dir_is_hidden(&left.display_dir)
            .cmp(&path_display_dir_is_hidden(&right.display_dir))
            .then(left.display_dir.cmp(&right.display_dir))
    });
}

fn path_display_dir_is_hidden(display_dir: &str) -> bool {
    last_path_component(display_dir.trim_end_matches('/')).is_some_and(|component| {
        component.starts_with('.') && component != "." && component != ".."
    })
}

fn last_path_component(path: &str) -> Option<&str> {
    path.rsplit('/').find(|component| !component.is_empty())
}

fn dedupe_path_completion_bases(bases: &mut Vec<PathCompletionBase>) {
    let mut seen = HashSet::new();
    bases.retain(|base| seen.insert((base.search_dir.clone(), base.display_dir.clone())));
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

pub(super) fn order_path_candidates_for_completion(
    candidates: Vec<CompletionCandidate>,
) -> Vec<CompletionCandidate> {
    let (mut directories, files) = split_path_candidates(candidates);
    directories.extend(files);
    directories
}

pub(super) fn split_path_candidates(
    candidates: Vec<CompletionCandidate>,
) -> (Vec<CompletionCandidate>, Vec<CompletionCandidate>) {
    candidates
        .into_iter()
        .partition(|candidate| candidate.source == CompletionSource::Path && candidate.is_dir)
}

pub(super) fn complete_path_executables(
    prefix: &str,
    path_dirs: &[PathBuf],
) -> Vec<CompletionCandidate> {
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
