use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::completion::{
    CompletionCandidate, CompletionOptions, CompletionSource, complete_first_token_with_options,
    complete_non_first_token_for_line_with_options, complete_private_command_line,
    current_token_context, dedupe_completion_candidates, limit_candidates,
    rank_completion_candidates,
};
use crate::completion_worker::{CompletionJob, CompletionTier, CompletionWorker};
use crate::history::HistoryEntry;
use crate::modes::Mode;
use crate::templates::{TemplateEntry, load_templates};

use super::{AppState, PendingCompletion, PendingCompletionUpdate};

impl AppState {
    pub fn completion_candidates(&self) -> Result<Vec<CompletionCandidate>> {
        self.completion_candidates_with_max_results(usize::MAX)
    }

    pub fn completion_panel_candidates(&self) -> Result<Vec<CompletionCandidate>> {
        self.completion_candidates_with_max_results(self.completion_config.max_results)
    }

    pub fn completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled || self.mode != Mode::Draft || self.draft_from_editor {
            return Ok(Vec::new());
        }
        let line = self.draft.as_str();
        let token = current_token_context(line, self.draft.cursor());
        if line.starts_with('#') {
            return Ok(complete_private_command_line(
                line,
                self.draft.cursor(),
                max_results,
            ));
        }
        let templates = self.templates_for_completion()?;
        let history_newest_first: Vec<_> = self.regular_history.iter().rev().cloned().collect();
        let options = CompletionOptions {
            max_results,
            ignore_spaces: self.completion_config.ignore_spaces,
            fuzzy_enabled: self.completion_config.fuzzy,
            match_threshold_percent: self.completion_config.match_threshold_percent,
            typo_threshold_percent: self.completion_config.typo_threshold_percent,
        };

        if token.is_first_token && !token.path_like {
            Ok(complete_first_token_with_options(
                &token.text,
                &templates,
                &history_newest_first,
                &path_dirs(),
                options,
            ))
        } else {
            Ok(complete_non_first_token_for_line_with_options(
                self.draft.as_str(),
                self.draft.cursor(),
                &completion_cwd(&self.current_cwd),
                &history_newest_first,
                &templates,
                options,
            ))
        }
    }

    pub fn start_live_completion_request(
        &mut self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        let now = Instant::now();
        let line = self.draft.as_str().to_string();
        let cursor = self.draft.cursor();
        let candidates = self.immediate_completion_candidates_with_max_results(max_results)?;
        self.pending_completion = None;
        self.pending_completion_update = None;
        let should_enqueue_async = self.should_enqueue_async_completion(&line, cursor);
        let display_deferred = !self.completion_display_ready(now) && !candidates.is_empty();
        let defer_initial_ui = should_enqueue_async
            && self.should_defer_initial_completion_ui(&line, cursor, &candidates);
        let hide_initial_ui = display_deferred || defer_initial_ui;
        if should_enqueue_async || display_deferred {
            self.completion_generation = self.completion_generation.wrapping_add(1).max(1);
            let id = self.completion_generation;
            self.pending_completion = Some(PendingCompletion {
                id,
                line: line.clone(),
                cursor,
                candidates: candidates.clone(),
            });
            if hide_initial_ui && !candidates.is_empty() {
                self.queue_completion_update(
                    id,
                    line.clone(),
                    cursor,
                    candidates.clone(),
                    !should_enqueue_async,
                    now,
                );
            }
            if should_enqueue_async {
                let history_newest_first = self.completion_history_snapshot();
                let templates = self.completion_template_snapshot()?;
                let job = CompletionJob {
                    id,
                    line,
                    cursor,
                    cwd: completion_cwd(&self.current_cwd),
                    path_dirs: Arc::new(path_dirs()),
                    history_newest_first,
                    templates,
                    options: self.completion_options(usize::MAX),
                };
                self.ensure_completion_worker().enqueue(job)?;
            }
        }
        Ok(if hide_initial_ui {
            Vec::new()
        } else {
            candidates
        })
    }

    pub fn drain_live_completion_events(&mut self) -> Option<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled {
            self.pending_completion = None;
            self.pending_completion_update = None;
            return None;
        }
        let events = self
            .completion_worker
            .as_ref()
            .map(|worker| worker.drain_events())
            .unwrap_or_default();
        let now = Instant::now();
        let line = self.draft.as_str().to_string();
        let cursor = self.draft.cursor();
        let fuzzy_enabled = self.completion_config.fuzzy;
        let Some(pending) = self.pending_completion.as_mut() else {
            self.pending_completion_update = None;
            return None;
        };
        if pending.line != line || pending.cursor != cursor {
            self.pending_completion = None;
            self.pending_completion_update = None;
            return None;
        }
        let mut changed = false;
        let mut final_tier_seen = false;
        for event in events {
            if event.id != pending.id {
                continue;
            }
            final_tier_seen |= completion_tier_is_final(event.tier, fuzzy_enabled);
            let previous_candidates = pending.candidates.clone();
            pending.candidates.extend(event.candidates);
            dedupe_completion_candidates(&mut pending.candidates);
            rank_completion_candidates(&mut pending.candidates);
            changed |= pending.candidates != previous_candidates;
        }
        let pending_id = pending.id;
        let pending_line = pending.line.clone();
        let pending_cursor = pending.cursor;
        let pending_candidates = pending.candidates.clone();
        if changed {
            self.queue_completion_update(
                pending_id,
                pending_line,
                pending_cursor,
                pending_candidates,
                final_tier_seen,
                now,
            );
        } else if final_tier_seen
            && let Some(update) = self.pending_completion_update.as_mut()
            && update.id == pending_id
            && update.line == pending_line
            && update.cursor == pending_cursor
        {
            update.final_tier_seen = true;
        }
        self.ready_completion_update(now)
    }

    fn queue_completion_update(
        &mut self,
        id: u64,
        line: String,
        cursor: usize,
        candidates: Vec<CompletionCandidate>,
        final_tier_seen: bool,
        now: Instant,
    ) {
        match self.pending_completion_update.as_mut() {
            Some(update) if update.id == id && update.line == line && update.cursor == cursor => {
                update.candidates = candidates;
                update.final_tier_seen |= final_tier_seen;
            }
            _ => {
                self.pending_completion_update = Some(PendingCompletionUpdate {
                    id,
                    line,
                    cursor,
                    candidates,
                    first_seen: now,
                    final_tier_seen,
                });
            }
        }
    }

    pub(super) fn ready_completion_update(
        &mut self,
        now: Instant,
    ) -> Option<Vec<CompletionCandidate>> {
        let (update_id, update_line, update_cursor, first_seen, final_tier_seen) = {
            let update = self.pending_completion_update.as_ref()?;
            (
                update.id,
                update.line.clone(),
                update.cursor,
                update.first_seen,
                update.final_tier_seen,
            )
        };
        let current_line = self.draft.as_str();
        let current_cursor = self.draft.cursor();
        let pending_matches = self.pending_completion.as_ref().is_some_and(|pending| {
            pending.id == update_id
                && pending.line == update_line
                && pending.cursor == update_cursor
                && update_line == current_line
                && update_cursor == current_cursor
        });
        if !pending_matches {
            self.pending_completion_update = None;
            return None;
        }
        let coalesce_ms = self.completion_config.coalesce_ms;
        let display_ready = self.completion_display_ready(now);
        let ready = display_ready
            && (coalesce_ms == 0
                || final_tier_seen
                || now.saturating_duration_since(first_seen) >= Duration::from_millis(coalesce_ms));
        if ready {
            self.completion_display_not_before = None;
            return self
                .pending_completion_update
                .take()
                .map(|update| update.candidates);
        }
        None
    }

    pub fn cached_live_completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Option<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled {
            return None;
        }
        let pending = self.pending_completion.as_ref()?;
        if pending.line != self.draft.as_str() || pending.cursor != self.draft.cursor() {
            return None;
        }
        Some(limit_candidates(pending.candidates.clone(), max_results))
    }

    pub fn live_completion_candidates_with_max_results(
        &mut self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if let Some(candidates) =
            self.cached_live_completion_candidates_with_max_results(max_results)
        {
            return Ok(candidates);
        }
        let candidates = self.start_live_completion_request(usize::MAX)?;
        Ok(limit_candidates(candidates, max_results))
    }

    pub fn immediate_completion_candidates_with_max_results(
        &self,
        max_results: usize,
    ) -> Result<Vec<CompletionCandidate>> {
        if !self.completion_config.enabled || self.mode != Mode::Draft || self.draft_from_editor {
            return Ok(Vec::new());
        }
        let line = self.draft.as_str();
        let cursor = self.draft.cursor();
        let token = current_token_context(line, cursor);
        if line.starts_with('#') {
            return Ok(complete_private_command_line(line, cursor, max_results));
        }

        let options = self.completion_options(max_results);
        if token.is_first_token && !token.path_like {
            return Ok(Vec::new());
        }
        Ok(complete_non_first_token_for_line_with_options(
            line,
            cursor,
            &completion_cwd(&self.current_cwd),
            &[],
            &[],
            options,
        ))
    }

    fn completion_options(&self, max_results: usize) -> CompletionOptions {
        CompletionOptions {
            max_results,
            ignore_spaces: self.completion_config.ignore_spaces,
            fuzzy_enabled: self.completion_config.fuzzy,
            match_threshold_percent: self.completion_config.match_threshold_percent,
            typo_threshold_percent: self.completion_config.typo_threshold_percent,
        }
    }

    fn ensure_completion_worker(&mut self) -> &CompletionWorker {
        self.completion_worker
            .get_or_insert_with(CompletionWorker::start)
    }

    fn should_enqueue_async_completion(&self, line: &str, cursor: usize) -> bool {
        if !self.completion_config.enabled
            || line.trim().is_empty()
            || line.starts_with('#')
            || cursor != line.len()
        {
            return false;
        }
        !current_token_context(line, cursor).path_like
    }

    fn should_defer_initial_completion_ui(
        &self,
        line: &str,
        cursor: usize,
        candidates: &[CompletionCandidate],
    ) -> bool {
        if self.completion_config.coalesce_ms == 0 || candidates.is_empty() {
            return false;
        }
        let token = current_token_context(line, cursor);
        token.is_first_token
            && !token.path_like
            && candidates
                .iter()
                .all(|candidate| candidate.source == CompletionSource::Executable)
    }

    fn completion_history_snapshot(&mut self) -> Arc<Vec<HistoryEntry>> {
        if self.completion_history_snapshot_len != self.regular_history.len() {
            self.completion_history_snapshot =
                Arc::new(self.regular_history.iter().rev().cloned().collect());
            self.completion_history_snapshot_len = self.regular_history.len();
        }
        Arc::clone(&self.completion_history_snapshot)
    }

    pub(super) fn invalidate_completion_history_snapshot(&mut self) {
        self.completion_history_snapshot_len = usize::MAX;
    }

    fn completion_template_snapshot(&mut self) -> Result<Arc<Vec<TemplateEntry>>> {
        let memory_backed = !self.templates.is_empty() || self.encryption_config.enabled;
        if self.completion_template_snapshot_len == usize::MAX
            || (memory_backed && self.completion_template_snapshot_len != self.templates.len())
        {
            let templates = self.templates_for_completion()?;
            self.completion_template_snapshot_len = if memory_backed {
                self.templates.len()
            } else {
                templates.len()
            };
            self.completion_template_snapshot = Arc::new(templates);
        }
        Ok(Arc::clone(&self.completion_template_snapshot))
    }

    pub(super) fn invalidate_completion_template_snapshot(&mut self) {
        self.completion_template_snapshot_len = usize::MAX;
    }

    pub(crate) fn defer_completion_display(&mut self, now: Instant) {
        if self.completion_config.display_delay_ms == 0 {
            self.completion_display_not_before = None;
            return;
        }
        self.completion_display_not_before =
            Some(now + Duration::from_millis(self.completion_config.display_delay_ms));
    }

    pub(crate) fn clear_completion_display_delay(&mut self) -> bool {
        self.completion_display_not_before.take().is_some()
    }

    fn completion_display_ready(&self, now: Instant) -> bool {
        self.completion_display_not_before
            .is_none_or(|deadline| now >= deadline)
    }

    pub fn clear_completion_ui(&mut self) {
        self.completion_panel.clear();
        self.completion_inline = None;
    }

    pub(crate) fn cancel_live_completion(&mut self) {
        self.clear_completion_ui();
        self.pending_completion = None;
        self.pending_completion_update = None;
        self.completion_display_not_before = None;
    }

    fn templates_for_completion(&self) -> Result<Vec<TemplateEntry>> {
        if !self.templates.is_empty() || self.encryption_config.enabled {
            return Ok(self.templates.clone());
        }
        let Some(path) = &self.template_store_path else {
            return Ok(Vec::new());
        };
        Ok(load_templates(path)?.items)
    }
}

fn completion_tier_is_final(tier: CompletionTier, fuzzy_enabled: bool) -> bool {
    matches!(tier, CompletionTier::Typo)
        || (!fuzzy_enabled && matches!(tier, CompletionTier::History))
}

fn completion_cwd(current_cwd: &Option<PathBuf>) -> PathBuf {
    current_cwd
        .clone()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn path_dirs() -> Vec<PathBuf> {
    env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect())
        .unwrap_or_default()
}
