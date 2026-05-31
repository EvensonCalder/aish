use std::collections::HashSet;

use super::{CompletionCandidate, CompletionSource};

pub(crate) fn dedupe_completion_candidates(candidates: &mut Vec<CompletionCandidate>) {
    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.replacement.clone()));
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
        CompletionSource::BackendShell => 0,
        CompletionSource::PrivateCommand => 1,
        CompletionSource::TemplateTypo => 9,
        CompletionSource::Template => 10,
        CompletionSource::HistoryTypo => 19,
        CompletionSource::History => 20,
        CompletionSource::Executable => 30,
        CompletionSource::TemplatePlaceholder => 40,
        CompletionSource::Path => 50,
    }
}

pub fn limit_candidates(
    mut candidates: Vec<CompletionCandidate>,
    max_results: usize,
) -> Vec<CompletionCandidate> {
    candidates.truncate(max_results);
    candidates
}
