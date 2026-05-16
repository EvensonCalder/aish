use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};

use crate::completion::{
    CompletionCandidate, CompletionOptions, complete_first_token_history_with_options,
    complete_non_first_token_typos_for_line_with_options,
    complete_structural_history_for_line_with_options, current_token_context,
    dedupe_completion_candidates, limit_candidates,
};
use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

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
        let handle = thread::spawn(move || {
            run_worker(receiver, event_sender, worker_latest_id);
        });
        Self {
            sender,
            events,
            latest_id,
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
    }
}

fn run_worker(
    receiver: mpsc::Receiver<CompletionWorkerMessage>,
    events: mpsc::Sender<CompletionEvent>,
    latest_id: Arc<AtomicU64>,
) {
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

        let history_candidates = complete_history_tier(&job);
        if is_latest(job.id, &latest_id) {
            let _ = events.send(CompletionEvent {
                id: job.id,
                tier: CompletionTier::History,
                candidates: history_candidates,
            });
        }

        if !job.options.fuzzy_enabled || !is_latest(job.id, &latest_id) {
            continue;
        }
        let typo_candidates = complete_typo_tier(&job, &latest_id);
        if is_latest(job.id, &latest_id) {
            let _ = events.send(CompletionEvent {
                id: job.id,
                tier: CompletionTier::Typo,
                candidates: typo_candidates,
            });
        }
    }
}

fn complete_history_tier(job: &CompletionJob) -> Vec<CompletionCandidate> {
    let mut options = job.options;
    options.max_results = usize::MAX;
    let token = current_token_context(&job.line, job.cursor);
    let candidates = if token.is_first_token && !token.path_like {
        complete_first_token_history_with_options(&token.text, &job.history_newest_first, options)
    } else {
        complete_structural_history_for_line_with_options(
            &job.line,
            job.cursor,
            &job.history_newest_first,
            options,
        )
    };
    limit_candidates(candidates, job.options.max_results)
}

fn complete_typo_tier(job: &CompletionJob, latest_id: &Arc<AtomicU64>) -> Vec<CompletionCandidate> {
    let parallelism = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    if parallelism == 1 || job.history_newest_first.len() < 256 {
        return complete_non_first_token_typos_for_line_with_options(
            &job.line,
            job.cursor,
            &job.history_newest_first,
            &job.templates,
            job.options,
        );
    }

    let chunk_size = job.history_newest_first.len().div_ceil(parallelism);
    let mut candidates = complete_non_first_token_typos_for_line_with_options(
        &job.line,
        job.cursor,
        &[],
        &job.templates,
        job.options,
    );
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in job.history_newest_first.chunks(chunk_size) {
            let line = job.line.clone();
            let options = job.options;
            let latest_id = Arc::clone(latest_id);
            let id = job.id;
            handles.push(scope.spawn(move || {
                if !is_latest(id, &latest_id) {
                    return Vec::new();
                }
                complete_non_first_token_typos_for_line_with_options(
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
