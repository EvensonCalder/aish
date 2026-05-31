use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::completion::{
    CompletionCandidate, CompletionOptions, CompletionSource, IndexedHistoryEntry,
    IndexedTemplateEntry, complete_first_token_executables_from_names_with_options,
    complete_first_token_history_with_indexed_options,
    complete_first_token_templates_with_indexed_options,
    complete_non_first_token_for_line_with_indexed_options,
    complete_non_first_token_typos_for_line_with_indexed_options, current_token_context,
    dedupe_completion_candidates, index_history_entries, index_template_entries, limit_candidates,
    matches_completion_prefix_with_threshold, rank_completion_candidates, scan_path_executables,
};
use crate::history::HistoryEntry;
use crate::shell_completion::{ShellCompletionRequest, complete_backend_shell};
use crate::templates::TemplateEntry;
use std::path::PathBuf;

const BACKEND_COMPLETION_DEBOUNCE: Duration = Duration::from_millis(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionTier {
    History,
    Typo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionJob {
    pub id: u64,
    pub line: String,
    pub cursor: usize,
    pub cwd: PathBuf,
    pub path_dirs: Arc<Vec<PathBuf>>,
    pub backend_shell: Option<String>,
    pub history_newest_first: Arc<Vec<HistoryEntry>>,
    pub templates: Arc<Vec<TemplateEntry>>,
    pub options: CompletionOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEvent {
    pub id: u64,
    pub tier: CompletionTier,
    pub candidates: Vec<CompletionCandidate>,
}

enum CompletionWorkerMessage {
    Job(CompletionJob),
    Stop,
}

pub struct CompletionWorker {
    sender: mpsc::Sender<CompletionWorkerMessage>,
    events: mpsc::Receiver<CompletionEvent>,
    latest_id: Arc<AtomicU64>,
    backend_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    handle: Option<JoinHandle<()>>,
}

impl fmt::Debug for CompletionWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompletionWorker").finish_non_exhaustive()
    }
}

impl CompletionWorker {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (event_sender, events) = mpsc::channel();
        let latest_id = Arc::new(AtomicU64::new(0));
        let worker_latest_id = Arc::clone(&latest_id);
        let backend_handles = Arc::new(Mutex::new(Vec::new()));
        let worker_backend_handles = Arc::clone(&backend_handles);
        let handle = thread::spawn(move || {
            run_worker(
                receiver,
                event_sender,
                worker_latest_id,
                worker_backend_handles,
            );
        });
        Self {
            sender,
            events,
            latest_id,
            backend_handles,
            handle: Some(handle),
        }
    }

    pub fn enqueue(&self, job: CompletionJob) -> Result<()> {
        self.latest_id.store(job.id, Ordering::Relaxed);
        self.sender
            .send(CompletionWorkerMessage::Job(job))
            .context("completion worker is not running")
    }

    pub fn drain_events(&self) -> Vec<CompletionEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for CompletionWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(CompletionWorkerMessage::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        join_backend_completion_threads(&self.backend_handles);
    }
}

fn run_worker(
    receiver: mpsc::Receiver<CompletionWorkerMessage>,
    events: mpsc::Sender<CompletionEvent>,
    latest_id: Arc<AtomicU64>,
    backend_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    let mut executable_index = ExecutableIndex::default();
    let mut data_index = CompletionDataIndex::default();
    let mut primary_cache = PrimaryTierCache::default();
    while let Ok(message) = receiver.recv() {
        let mut job = match message {
            CompletionWorkerMessage::Job(job) => job,
            CompletionWorkerMessage::Stop => break,
        };
        loop {
            match receiver.try_recv() {
                Ok(CompletionWorkerMessage::Job(next_job)) => job = next_job,
                Ok(CompletionWorkerMessage::Stop) => return,
                Err(_) => break,
            }
        }
        if !is_latest(job.id, &latest_id) {
            continue;
        }

        let history_candidates = complete_primary_tier(
            &job,
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );
        if is_latest(job.id, &latest_id) {
            let _ = events.send(CompletionEvent {
                id: job.id,
                tier: CompletionTier::History,
                candidates: history_candidates,
            });
        }
        spawn_backend_completion_event(&job, &events, &latest_id, &backend_handles);

        if !job.options.fuzzy_enabled || !is_latest(job.id, &latest_id) {
            continue;
        }
        let typo_candidates = complete_typo_tier(&job, &latest_id, &mut data_index);
        if is_latest(job.id, &latest_id) {
            let _ = events.send(CompletionEvent {
                id: job.id,
                tier: CompletionTier::Typo,
                candidates: typo_candidates,
            });
        }
    }
}

#[derive(Default)]
struct CompletionDataIndex {
    history_source: Option<Arc<Vec<HistoryEntry>>>,
    history: Vec<IndexedHistoryEntry>,
    template_source: Option<Arc<Vec<TemplateEntry>>>,
    templates: Vec<IndexedTemplateEntry>,
}

impl CompletionDataIndex {
    fn refresh_history(&mut self, history: &Arc<Vec<HistoryEntry>>) {
        if self
            .history_source
            .as_ref()
            .is_none_or(|source| !Arc::ptr_eq(source, history))
        {
            self.history = index_history_entries(history);
            self.history_source = Some(Arc::clone(history));
        }
    }

    fn refresh_templates(&mut self, templates: &Arc<Vec<TemplateEntry>>) {
        if self
            .template_source
            .as_ref()
            .is_none_or(|source| !Arc::ptr_eq(source, templates))
        {
            self.templates = index_template_entries(templates);
            self.template_source = Some(Arc::clone(templates));
        }
    }

    fn refresh_for_job(&mut self, job: &CompletionJob) {
        self.refresh_history(&job.history_newest_first);
        self.refresh_templates(&job.templates);
    }
}

#[derive(Default)]
struct ExecutableIndex {
    path_dirs: Vec<PathBuf>,
    names: Vec<String>,
}

impl ExecutableIndex {
    fn names_for(&mut self, path_dirs: &[PathBuf]) -> &[String] {
        if self.path_dirs != path_dirs {
            self.names = scan_path_executables(path_dirs);
            self.path_dirs = path_dirs.to_vec();
        }
        &self.names
    }
}

fn complete_primary_tier(
    job: &CompletionJob,
    executable_index: &mut ExecutableIndex,
    data_index: &mut CompletionDataIndex,
    primary_cache: &mut PrimaryTierCache,
) -> Vec<CompletionCandidate> {
    let mut options = job.options;
    options.max_results = usize::MAX;
    let token = current_token_context(&job.line, job.cursor);
    let mut candidates = if let Some(candidates) = primary_cache.filtered_candidates(job) {
        candidates
    } else if token.is_first_token && !token.path_like {
        data_index.refresh_for_job(job);
        let mut candidates = complete_first_token_templates_with_indexed_options(
            &token.text,
            &data_index.templates,
            options,
        );
        candidates.extend(complete_first_token_history_with_indexed_options(
            &token.text,
            &data_index.history,
            options,
        ));
        candidates.extend(complete_first_token_executables_from_names_with_options(
            &token.text,
            executable_index.names_for(&job.path_dirs),
            options,
        ));
        candidates
    } else {
        data_index.refresh_for_job(job);
        complete_non_first_token_for_line_with_indexed_options(
            &job.line,
            job.cursor,
            &job.cwd,
            &data_index.history,
            &data_index.templates,
            options,
        )
    };
    dedupe_completion_candidates(&mut candidates);
    rank_completion_candidates(&mut candidates);
    primary_cache.store(job, candidates.clone());
    limit_candidates(candidates, job.options.max_results)
}

fn spawn_backend_completion_event(
    job: &CompletionJob,
    events: &mpsc::Sender<CompletionEvent>,
    latest_id: &Arc<AtomicU64>,
    backend_handles: &Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    if job.backend_shell.is_none() {
        return;
    }
    let job = job.clone();
    let events = events.clone();
    let latest_id = Arc::clone(latest_id);
    let handle = thread::spawn(move || {
        thread::sleep(BACKEND_COMPLETION_DEBOUNCE);
        if !is_latest(job.id, &latest_id) {
            return;
        }
        let candidates = complete_backend_shell_for_job(&job);
        if candidates.is_empty() || !is_latest(job.id, &latest_id) {
            return;
        }
        let _ = events.send(CompletionEvent {
            id: job.id,
            tier: CompletionTier::History,
            candidates,
        });
    });
    remember_backend_completion_thread(backend_handles, handle);
}

fn remember_backend_completion_thread(
    backend_handles: &Arc<Mutex<Vec<JoinHandle<()>>>>,
    handle: JoinHandle<()>,
) {
    let mut handles = backend_handles
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let handle = handles.swap_remove(index);
            let _ = handle.join();
        } else {
            index += 1;
        }
    }
    handles.push(handle);
}

fn join_backend_completion_threads(backend_handles: &Arc<Mutex<Vec<JoinHandle<()>>>>) {
    let handles = {
        let mut handles = backend_handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        handles.drain(..).collect::<Vec<_>>()
    };
    for handle in handles {
        let _ = handle.join();
    }
}

fn complete_backend_shell_for_job(job: &CompletionJob) -> Vec<CompletionCandidate> {
    let Some(shell) = &job.backend_shell else {
        return Vec::new();
    };
    complete_backend_shell(&ShellCompletionRequest {
        shell: shell.clone(),
        line: job.line.clone(),
        cursor: job.cursor,
        cwd: job.cwd.clone(),
        env: Vec::new(),
    })
}

#[derive(Default)]
struct PrimaryTierCache {
    entry: Option<PrimaryTierCacheEntry>,
}

#[derive(Clone)]
struct PrimaryTierCacheEntry {
    line: String,
    cursor: usize,
    cwd: PathBuf,
    path_dirs: Arc<Vec<PathBuf>>,
    backend_shell: Option<String>,
    history: Arc<Vec<HistoryEntry>>,
    templates: Arc<Vec<TemplateEntry>>,
    options: CompletionOptions,
    candidates: Vec<CompletionCandidate>,
}

impl PrimaryTierCache {
    fn filtered_candidates(&self, job: &CompletionJob) -> Option<Vec<CompletionCandidate>> {
        let entry = self.entry.as_ref()?;
        if !can_filter_primary_cache(entry, job) {
            return None;
        }
        let token = current_token_context(&job.line, job.cursor);
        Some(
            entry
                .candidates
                .iter()
                .filter(|candidate| {
                    first_token_candidate_matches(candidate, &token.text, job.options)
                })
                .cloned()
                .collect(),
        )
    }

    fn store(&mut self, job: &CompletionJob, candidates: Vec<CompletionCandidate>) {
        self.entry = Some(PrimaryTierCacheEntry {
            line: job.line.clone(),
            cursor: job.cursor,
            cwd: job.cwd.clone(),
            path_dirs: Arc::clone(&job.path_dirs),
            backend_shell: job.backend_shell.clone(),
            history: Arc::clone(&job.history_newest_first),
            templates: Arc::clone(&job.templates),
            options: job.options,
            candidates,
        });
    }
}

fn can_filter_primary_cache(entry: &PrimaryTierCacheEntry, job: &CompletionJob) -> bool {
    if entry.options != job.options
        || entry.cwd != job.cwd
        || entry.path_dirs.as_slice() != job.path_dirs.as_slice()
        || entry.backend_shell != job.backend_shell
        || !Arc::ptr_eq(&entry.history, &job.history_newest_first)
        || !Arc::ptr_eq(&entry.templates, &job.templates)
        || entry.cursor != entry.line.len()
        || job.cursor != job.line.len()
        || !job.line.starts_with(&entry.line)
        || job.line.len() <= entry.line.len()
    {
        return false;
    }
    let previous = current_token_context(&entry.line, entry.cursor);
    let current = current_token_context(&job.line, job.cursor);
    previous.is_first_token
        && current.is_first_token
        && !previous.path_like
        && !current.path_like
        && previous.start == current.start
        && !previous.text.is_empty()
        && current.text.starts_with(&previous.text)
}

fn first_token_candidate_matches(
    candidate: &CompletionCandidate,
    prefix: &str,
    options: CompletionOptions,
) -> bool {
    match candidate.source {
        CompletionSource::Template | CompletionSource::History => {
            matches_completion_prefix_with_threshold(
                &candidate.replacement,
                prefix,
                options.ignore_spaces,
                options.match_threshold_percent,
            )
        }
        CompletionSource::Executable | CompletionSource::BackendShell => {
            candidate.replacement.starts_with(prefix)
        }
        _ => false,
    }
}

fn complete_typo_tier(
    job: &CompletionJob,
    latest_id: &Arc<AtomicU64>,
    data_index: &mut CompletionDataIndex,
) -> Vec<CompletionCandidate> {
    data_index.refresh_for_job(job);
    let templates = &data_index.templates;
    let history = &data_index.history;
    let parallelism = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    if parallelism == 1 || history.len() < 256 {
        return complete_non_first_token_typos_for_line_with_indexed_options(
            &job.line,
            job.cursor,
            history,
            templates,
            job.options,
        );
    }

    let chunk_size = history.len().div_ceil(parallelism);
    let mut candidates = complete_non_first_token_typos_for_line_with_indexed_options(
        &job.line,
        job.cursor,
        &[],
        templates,
        job.options,
    );
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in history.chunks(chunk_size) {
            let line = job.line.clone();
            let options = job.options;
            let latest_id = Arc::clone(latest_id);
            let id = job.id;
            handles.push(scope.spawn(move || {
                if !is_latest(id, &latest_id) {
                    return Vec::new();
                }
                complete_non_first_token_typos_for_line_with_indexed_options(
                    &line,
                    job.cursor,
                    chunk,
                    &[],
                    options,
                )
            }));
        }
        for handle in handles {
            if !is_latest(job.id, latest_id) {
                break;
            }
            if let Ok(mut chunk_candidates) = handle.join() {
                candidates.append(&mut chunk_candidates);
            }
        }
    });
    dedupe_completion_candidates(&mut candidates);
    limit_candidates(candidates, job.options.max_results)
}

fn is_latest(id: u64, latest_id: &AtomicU64) -> bool {
    latest_id.load(Ordering::Relaxed) == id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistorySource;
    use std::path::PathBuf;

    fn history(command: &str, t: i64) -> HistoryEntry {
        HistoryEntry {
            t,
            command: command.to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }
    }

    fn job(
        id: u64,
        line: &str,
        max_results: usize,
        history_newest_first: Arc<Vec<HistoryEntry>>,
    ) -> CompletionJob {
        CompletionJob {
            id,
            line: line.to_string(),
            cursor: line.len(),
            cwd: PathBuf::from("/"),
            path_dirs: Arc::new(Vec::new()),
            backend_shell: None,
            history_newest_first,
            templates: Arc::new(Vec::new()),
            options: CompletionOptions {
                max_results,
                ..CompletionOptions::default()
            },
        }
    }

    #[test]
    fn primary_cache_filters_full_candidate_set_not_display_limit() {
        let history = Arc::new(vec![history("cargo build", 2), history("cat alpha", 1)]);
        let mut executable_index = ExecutableIndex::default();
        let mut data_index = CompletionDataIndex::default();
        let mut primary_cache = PrimaryTierCache::default();

        let first = complete_primary_tier(
            &job(1, "c", 1, Arc::clone(&history)),
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].replacement, "cargo build");

        let second = complete_primary_tier(
            &job(2, "cat", 1, Arc::clone(&history)),
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );
        assert_eq!(
            second,
            [CompletionCandidate {
                display: "cat alpha".to_string(),
                replacement: "cat alpha".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            }]
        );
    }

    #[test]
    fn primary_cache_invalidates_on_deletion() {
        let history = Arc::new(vec![history("cargo build", 2), history("cat alpha", 1)]);
        let mut executable_index = ExecutableIndex::default();
        let mut data_index = CompletionDataIndex::default();
        let mut primary_cache = PrimaryTierCache::default();

        let cat = complete_primary_tier(
            &job(1, "cat", usize::MAX, Arc::clone(&history)),
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );
        assert_eq!(cat[0].replacement, "cat alpha");

        let c = complete_primary_tier(
            &job(2, "c", usize::MAX, Arc::clone(&history)),
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );
        assert_eq!(
            c.iter()
                .map(|candidate| candidate.replacement.as_str())
                .collect::<Vec<_>>(),
            ["cargo build", "cat alpha"]
        );
    }

    #[test]
    fn backend_shell_candidates_are_not_part_of_primary_tier() {
        let mut job = job(
            1,
            "git st",
            usize::MAX,
            Arc::new(vec![history("git status", 1)]),
        );
        job.backend_shell = Some("aish-test-backend:status,stash".to_string());
        let mut executable_index = ExecutableIndex::default();
        let mut data_index = CompletionDataIndex::default();
        let mut primary_cache = PrimaryTierCache::default();

        let candidates = complete_primary_tier(
            &job,
            &mut executable_index,
            &mut data_index,
            &mut primary_cache,
        );

        assert!(
            candidates
                .iter()
                .all(|candidate| { candidate.source != CompletionSource::BackendShell })
        );
        assert!(candidates.iter().any(|candidate| {
            candidate.source == CompletionSource::History && candidate.replacement == "status"
        }));
    }

    #[test]
    fn worker_emits_primary_candidates_before_slow_backend_shell_candidates() {
        let mut job = job(
            1,
            "git st",
            usize::MAX,
            Arc::new(vec![history("git status", 1)]),
        );
        job.backend_shell = Some("aish-test-backend-delay-ms:200:status,stash".to_string());
        let worker = CompletionWorker::start();
        worker.enqueue(job).unwrap();

        let start = std::time::Instant::now();
        let mut primary_seen = None;
        let mut backend_seen = None;
        while start.elapsed() < Duration::from_secs(2) {
            for event in worker.drain_events() {
                if event.id != 1 || event.candidates.is_empty() {
                    continue;
                }
                if event
                    .candidates
                    .iter()
                    .any(|candidate| candidate.source == CompletionSource::BackendShell)
                {
                    backend_seen = Some(start.elapsed());
                } else {
                    primary_seen = Some(start.elapsed());
                }
            }
            if primary_seen.is_some() && backend_seen.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let primary_seen = primary_seen.expect("missing primary completion event");
        let backend_seen = backend_seen.expect("missing backend shell completion event");
        assert!(
            primary_seen < backend_seen,
            "primary event {primary_seen:?} should arrive before backend event {backend_seen:?}"
        );
    }
}
