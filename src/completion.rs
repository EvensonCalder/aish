use std::path::Path;

use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

mod arguments;
mod first_token;
mod index;
mod matching;
mod parser;
mod path;
mod private;
mod ranking;
mod render;
mod structural;
mod types;
mod typo;

pub use first_token::{
    complete_first_token, complete_first_token_executables_with_options,
    complete_first_token_history_with_options, complete_first_token_templates_with_options,
    complete_first_token_with_options,
};
pub(crate) use first_token::{
    complete_first_token_executables_from_names_with_options,
    complete_first_token_history_with_indexed_options,
    complete_first_token_templates_with_indexed_options,
};
pub(crate) use index::{
    IndexedHistoryEntry, IndexedTemplateEntry, index_history_entries, index_template_entries,
};
pub use matching::{matches_completion_prefix, matches_completion_prefix_with_threshold};
pub(crate) use parser::shell_like_words;
pub use parser::{current_token_context, is_path_like_token};
pub use path::complete_path;
pub use private::{complete_private_command_line, complete_private_commands};
pub use ranking::limit_candidates;
pub use render::{
    accept_completion, accept_completion_with_mode, completion_edit_for_candidate,
    ghost_completion_suffix, render_completion_candidates, render_completion_candidates_for_width,
    truncate_with_ellipsis,
};
pub use structural::{
    complete_structural_history_for_line_with_options,
    complete_structural_templates_for_line_with_options,
};
pub use types::{
    AcceptedCompletion, CompletionCandidate, CompletionEdit, CompletionOptions, CompletionSource,
    TokenContext,
};
pub(crate) use typo::complete_non_first_token_typos_for_line_with_indexed_options;
pub use typo::{
    complete_first_token_typos_with_options, complete_non_first_token_typos_for_line_with_options,
    complete_typo_candidates_for_line_with_options,
};

use arguments::{
    complete_history_arguments, complete_history_arguments_indexed, complete_template_placeholders,
    complete_template_placeholders_indexed,
};
#[cfg(test)]
use parser::{command_arguments, split_shell_like_words};
pub(crate) use path::scan_path_executables;
use path::{
    complete_path_with_options, order_path_candidates_for_completion, split_path_candidates,
};
pub(crate) use ranking::{dedupe_completion_candidates, rank_completion_candidates};
use structural::{
    complete_structural_history_for_line_indexed, complete_structural_templates_for_line_indexed,
};

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
    let mut structural_history_candidates = complete_structural_history_for_line_indexed(
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
        retain_path_like_structural_candidates(&token, &mut structural_candidates, options);
        retain_path_like_structural_candidates(&token, &mut structural_history_candidates, options);
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

fn retain_path_like_structural_candidates(
    token: &TokenContext,
    candidates: &mut Vec<CompletionCandidate>,
    options: CompletionOptions,
) {
    if !token.path_like {
        return;
    }
    candidates.retain(|candidate| {
        matches_completion_prefix_with_threshold(
            &candidate.replacement,
            &token.text,
            options.ignore_spaces,
            options.match_threshold_percent,
        )
    });
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

#[cfg(test)]
mod tests;
