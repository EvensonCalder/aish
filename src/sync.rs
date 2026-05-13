use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::SyncConfig;
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event};

const GITIGNORE_BEGIN: &str = "# BEGIN AISH MANAGED";
const GITIGNORE_END: &str = "# END AISH MANAGED";
const MANAGED_GITIGNORE_LINES: &[&str] = &["cache/", "logs/", "secrets/", "*.tmp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedManagedFilesWarning {
    pub paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFailureKind {
    Conflict,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupSyncDecision {
    Due,
    Disabled,
    MissingRemote,
    MissingSchedule,
    NotDue { next_due_at: i64 },
    UnsupportedSchedule(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStepOutcome {
    Continue,
    AbortConflict { detail: String },
    AbortFailure { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAddPlan {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct SyncLock {
    path: PathBuf,
    held: bool,
}

impl SyncLock {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create sync lock directory {}", parent.display())
            })?;
        }

        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => {
                write_lock_metadata(file)?;
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    held: true,
                }))
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(err) => {
                Err(err).with_context(|| format!("failed to create sync lock {}", path.display()))
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        if self.held {
            let _ = fs::remove_file(&self.path);
            self.held = false;
        }
    }
}

fn write_lock_metadata(mut file: File) -> Result<()> {
    use std::io::Write;

    writeln!(file, "pid={}", std::process::id()).context("failed to write sync lock metadata")
}

pub fn maintain_managed_gitignore(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let existing = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read gitignore {}", path.display()))?
        }
    };
    let next = replace_managed_gitignore_section(&existing);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create gitignore directory {}", parent.display())
        })?;
    }
    fs::write(path, next).with_context(|| format!("failed to write gitignore {}", path.display()))
}

fn replace_managed_gitignore_section(existing: &str) -> String {
    let managed = managed_gitignore_section();
    let lines: Vec<&str> = existing.lines().collect();
    let start = lines.iter().position(|line| line.trim() == GITIGNORE_BEGIN);
    let end = lines.iter().position(|line| line.trim() == GITIGNORE_END);

    if let (Some(start), Some(end)) = (start, end)
        && start <= end
    {
        let mut output = String::new();
        for line in &lines[..start] {
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(&managed);
        for line in &lines[end + 1..] {
            output.push_str(line);
            output.push('\n');
        }
        return output;
    }

    let mut output = existing.trim_end_matches('\n').to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(&managed);
    output
}

fn managed_gitignore_section() -> String {
    let mut output = String::new();
    output.push_str(GITIGNORE_BEGIN);
    output.push('\n');
    for line in MANAGED_GITIGNORE_LINES {
        output.push_str(line);
        output.push('\n');
    }
    output.push_str(GITIGNORE_END);
    output.push('\n');
    output
}

pub fn tracked_managed_files_warning<I, S>(tracked_paths: I) -> Option<TrackedManagedFilesWarning>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut paths: Vec<String> = tracked_paths
        .into_iter()
        .filter_map(|path| managed_tracked_path(path.as_ref()))
        .collect();
    paths.sort();
    paths.dedup();

    if paths.is_empty() {
        return None;
    }

    Some(TrackedManagedFilesWarning {
        message: format!(
            "warning: {} Aish-managed path(s) may already be tracked; not running git rm --cached automatically",
            paths.len()
        ),
        paths,
    })
}

fn managed_tracked_path(path: &str) -> Option<String> {
    let normalized = path.trim_start_matches("./");
    if normalized.starts_with("cache/")
        || normalized.starts_with("logs/")
        || normalized.starts_with("secrets/")
        || normalized.ends_with(".tmp")
    {
        Some(normalized.to_string())
    } else {
        None
    }
}

pub fn log_sync_failure(
    log_path: impl AsRef<Path>,
    t: i64,
    kind: SyncFailureKind,
    detail: &str,
) -> Result<()> {
    let label = match kind {
        SyncFailureKind::Conflict => "sync conflict",
        SyncFailureKind::Failure => "sync failed",
    };
    append_event(
        log_path.as_ref(),
        t,
        EventLevel::Error,
        &format!("{label}: {detail}"),
        DEFAULT_MAX_EVENTS,
    )
}

pub fn startup_sync_decision(
    config: &SyncConfig,
    now_unix: i64,
    last_attempt_unix: Option<i64>,
) -> StartupSyncDecision {
    if !config.enabled {
        return StartupSyncDecision::Disabled;
    }
    if config.remote.trim().is_empty() {
        return StartupSyncDecision::MissingRemote;
    }
    let schedule = config.schedule.trim();
    if schedule.is_empty() {
        return StartupSyncDecision::MissingSchedule;
    }
    let Some(interval_seconds) = conservative_schedule_interval_seconds(schedule) else {
        return StartupSyncDecision::UnsupportedSchedule(schedule.to_string());
    };
    let Some(last_attempt_unix) = last_attempt_unix else {
        return StartupSyncDecision::Due;
    };
    let next_due_at = last_attempt_unix.saturating_add(interval_seconds);
    if now_unix >= next_due_at {
        StartupSyncDecision::Due
    } else {
        StartupSyncDecision::NotDue { next_due_at }
    }
}

fn conservative_schedule_interval_seconds(schedule: &str) -> Option<i64> {
    match schedule {
        "@hourly" => Some(60 * 60),
        "@daily" => Some(24 * 60 * 60),
        _ => five_field_cron_interval_seconds(schedule),
    }
}

fn five_field_cron_interval_seconds(schedule: &str) -> Option<i64> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    match fields.as_slice() {
        [minute, "*", "*", "*", "*"] if minute.starts_with("*/") => {
            cron_step_interval_seconds(minute, 60)
        }
        ["0", hour, "*", "*", "*"] if hour.starts_with("*/") => {
            cron_step_interval_seconds(hour, 60 * 60)
        }
        ["0", "0", "*", "*", "*"] => Some(24 * 60 * 60),
        ["0", "0", day, "*", "*"] if day.starts_with("*/") => {
            cron_step_interval_seconds(day, 24 * 60 * 60)
        }
        _ => None,
    }
}

fn cron_step_interval_seconds(field: &str, unit_seconds: i64) -> Option<i64> {
    let step = field.strip_prefix("*/")?.parse::<i64>().ok()?;
    if step <= 0 {
        return None;
    }
    Some(step * unit_seconds)
}

pub fn classify_git_sync_step(success: bool, stdout: &str, stderr: &str) -> SyncStepOutcome {
    if success {
        return SyncStepOutcome::Continue;
    }

    let detail = combined_git_output(stdout, stderr);
    if git_output_looks_conflicted(&detail) {
        SyncStepOutcome::AbortConflict { detail }
    } else {
        SyncStepOutcome::AbortFailure { detail }
    }
}

pub fn managed_add_plan(config: &SyncConfig) -> ManagedAddPlan {
    let mut paths = vec![".gitignore".to_string()];
    if config.ai {
        paths.push("history/ai.jsonl".to_string());
    }
    if config.history {
        paths.push("history/notes.jsonl".to_string());
        paths.push("history/regular.jsonl".to_string());
    }
    if config.templates {
        paths.push("templates/templates.jsonl".to_string());
    }
    if config.drafts {
        paths.push("history/draft.jsonl".to_string());
    }
    paths.sort();
    paths.dedup();
    ManagedAddPlan { paths }
}

pub fn pull_rebase_plan() -> GitCommandPlan {
    GitCommandPlan {
        program: "git".to_string(),
        args: vec!["pull".to_string(), "--rebase".to_string()],
    }
}

pub fn commit_plan(message: &str) -> Option<GitCommandPlan> {
    let message = sanitize_commit_message(message);
    if message.is_empty() {
        return None;
    }
    Some(GitCommandPlan {
        program: "git".to_string(),
        args: vec!["commit".to_string(), "-m".to_string(), message],
    })
}

pub fn default_sync_commit_plan() -> GitCommandPlan {
    commit_plan("sync aish data").expect("default sync commit message is non-empty")
}

fn sanitize_commit_message(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(72)
        .collect()
}

fn combined_git_output(stdout: &str, stderr: &str) -> String {
    let mut parts = Vec::new();
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    if !stdout.is_empty() {
        parts.push(stdout);
    }
    if !stderr.is_empty() {
        parts.push(stderr);
    }
    if parts.is_empty() {
        "git command failed without output".to_string()
    } else {
        parts.join("\n")
    }
}

fn git_output_looks_conflicted(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("merge conflict")
        || lower.contains("conflict (content)")
        || lower.contains("fix conflicts and then")
        || lower.contains("cannot rebase")
        || lower.contains("could not apply")
        || lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("failed to push some refs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_lock_allows_single_holder_and_removes_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime/sync.lock");

        let lock = SyncLock::acquire(&path)
            .unwrap()
            .expect("first lock acquired");
        assert_eq!(lock.path(), path.as_path());
        assert!(path.exists());
        assert!(SyncLock::acquire(&path).unwrap().is_none());

        drop(lock);

        assert!(!path.exists());
        assert!(SyncLock::acquire(&path).unwrap().is_some());
    }

    #[test]
    fn sync_lock_creates_parent_directory() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/locks/sync.lock");

        let _lock = SyncLock::acquire(&path).unwrap().expect("lock acquired");

        assert!(path.exists());
    }

    #[test]
    fn managed_gitignore_preserves_user_content_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".gitignore");
        fs::write(&path, "user-file\n").unwrap();

        maintain_managed_gitignore(&path).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        maintain_managed_gitignore(&path).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("user-file\n"));
        assert!(first.contains(GITIGNORE_BEGIN));
        assert!(first.contains("cache/\n"));
        assert!(first.contains("logs/\n"));
        assert!(first.contains("secrets/\n"));
        assert!(first.contains(GITIGNORE_END));
    }

    #[test]
    fn managed_gitignore_replaces_existing_managed_section() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".gitignore");
        fs::write(
            &path,
            "before\n# BEGIN AISH MANAGED\nold\n# END AISH MANAGED\nafter\n",
        )
        .unwrap();

        maintain_managed_gitignore(&path).unwrap();
        let updated = fs::read_to_string(&path).unwrap();

        assert!(updated.contains("before\n"));
        assert!(updated.contains("after\n"));
        assert!(!updated.contains("old\n"));
        assert_eq!(updated.matches(GITIGNORE_BEGIN).count(), 1);
        assert_eq!(updated.matches(GITIGNORE_END).count(), 1);
    }

    #[test]
    fn tracked_managed_files_warning_lists_managed_tracked_paths() {
        let warning = tracked_managed_files_warning([
            "README.md",
            "cache/model.json",
            "./logs/events.jsonl",
            "secrets/key.json.gpg",
            "notes.tmp",
            "cache/model.json",
        ])
        .expect("tracked managed paths are warned");

        assert_eq!(
            warning.paths,
            vec![
                "cache/model.json",
                "logs/events.jsonl",
                "notes.tmp",
                "secrets/key.json.gpg"
            ]
        );
        assert!(warning.message.contains("4 Aish-managed path(s)"));
        assert!(warning.message.contains("not running git rm --cached"));
    }

    #[test]
    fn tracked_managed_files_warning_ignores_unmanaged_paths() {
        assert!(tracked_managed_files_warning(["README.md", "src/main.rs", "tmp/notes"]).is_none());
    }

    #[test]
    fn log_sync_failure_records_error_event() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("logs/events.jsonl");

        log_sync_failure(&path, 7, SyncFailureKind::Failure, "git push exited 1").unwrap();

        let loaded = crate::log::load_events(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].t, 7);
        assert_eq!(loaded.items[0].level, EventLevel::Error);
        assert_eq!(loaded.items[0].msg, "sync failed: git push exited 1");
    }

    #[test]
    fn log_sync_conflict_redacts_secret_like_detail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("logs/events.jsonl");

        log_sync_failure(
            &path,
            8,
            SyncFailureKind::Conflict,
            "merge conflict near sk-secret-token",
        )
        .unwrap();

        let loaded = crate::log::load_events(&path).unwrap();
        assert_eq!(loaded.items[0].level, EventLevel::Error);
        assert_eq!(
            loaded.items[0].msg,
            "sync conflict: merge conflict near [redacted]"
        );
    }

    #[test]
    fn startup_sync_decision_skips_when_not_configured() {
        let mut config = SyncConfig::default();
        assert_eq!(
            startup_sync_decision(&config, 100, None),
            StartupSyncDecision::Disabled
        );

        config.enabled = true;
        assert_eq!(
            startup_sync_decision(&config, 100, None),
            StartupSyncDecision::MissingRemote
        );

        config.remote = "git@example.test:aish.git".to_string();
        assert_eq!(
            startup_sync_decision(&config, 100, None),
            StartupSyncDecision::MissingSchedule
        );
    }

    #[test]
    fn startup_sync_decision_handles_supported_schedules_conservatively() {
        let config = SyncConfig {
            enabled: true,
            remote: "git@example.test:aish.git".to_string(),
            schedule: "*/15 * * * *".to_string(),
            ai: false,
            history: false,
            templates: false,
            drafts: false,
        };

        assert_eq!(
            startup_sync_decision(&config, 100, None),
            StartupSyncDecision::Due
        );
        assert_eq!(
            startup_sync_decision(&config, 1000, Some(200)),
            StartupSyncDecision::NotDue { next_due_at: 1100 }
        );
        assert_eq!(
            startup_sync_decision(&config, 1100, Some(200)),
            StartupSyncDecision::Due
        );
    }

    #[test]
    fn startup_sync_decision_rejects_unsupported_cron_without_side_effects() {
        let config = SyncConfig {
            enabled: true,
            remote: "git@example.test:aish.git".to_string(),
            schedule: "5 4 * * mon".to_string(),
            ai: false,
            history: false,
            templates: false,
            drafts: false,
        };

        assert_eq!(
            startup_sync_decision(&config, 100, Some(0)),
            StartupSyncDecision::UnsupportedSchedule("5 4 * * mon".to_string())
        );
    }

    #[test]
    fn classify_git_sync_step_continues_on_success() {
        assert_eq!(
            classify_git_sync_step(true, "already up to date", ""),
            SyncStepOutcome::Continue
        );
    }

    #[test]
    fn classify_git_sync_step_aborts_on_conflict_like_output() {
        let outcome = classify_git_sync_step(
            false,
            "CONFLICT (content): Merge conflict in history/regular.jsonl",
            "error: could not apply abc123",
        );

        assert_eq!(
            outcome,
            SyncStepOutcome::AbortConflict {
                detail: "CONFLICT (content): Merge conflict in history/regular.jsonl\nerror: could not apply abc123".to_string()
            }
        );
    }

    #[test]
    fn classify_git_sync_step_aborts_on_non_conflict_failure() {
        assert_eq!(
            classify_git_sync_step(false, "", "fatal: unable to access remote"),
            SyncStepOutcome::AbortFailure {
                detail: "fatal: unable to access remote".to_string()
            }
        );
    }

    #[test]
    fn managed_add_plan_keeps_private_categories_off_by_default() {
        let config = SyncConfig::default();

        assert_eq!(
            managed_add_plan(&config),
            ManagedAddPlan {
                paths: vec![".gitignore".to_string()]
            }
        );
    }

    #[test]
    fn managed_add_plan_includes_enabled_category_paths_sorted() {
        let config = SyncConfig {
            ai: true,
            history: true,
            templates: true,
            drafts: true,
            ..SyncConfig::default()
        };

        assert_eq!(
            managed_add_plan(&config).paths,
            vec![
                ".gitignore",
                "history/ai.jsonl",
                "history/draft.jsonl",
                "history/notes.jsonl",
                "history/regular.jsonl",
                "templates/templates.jsonl",
            ]
        );
    }

    #[test]
    fn pull_rebase_plan_uses_fixed_git_arguments() {
        assert_eq!(
            pull_rebase_plan(),
            GitCommandPlan {
                program: "git".to_string(),
                args: vec!["pull".to_string(), "--rebase".to_string()]
            }
        );
    }

    #[test]
    fn default_sync_commit_plan_uses_fixed_git_arguments() {
        assert_eq!(
            default_sync_commit_plan(),
            GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "commit".to_string(),
                    "-m".to_string(),
                    "sync aish data".to_string()
                ]
            }
        );
    }

    #[test]
    fn commit_plan_sanitizes_message_without_shell_interpolation() {
        assert_eq!(
            commit_plan("\n  sync now && rm -rf /\nsecond line").unwrap(),
            GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "commit".to_string(),
                    "-m".to_string(),
                    "sync now && rm -rf /".to_string()
                ]
            }
        );
    }

    #[test]
    fn commit_plan_rejects_empty_message() {
        assert_eq!(commit_plan("\n\t\n"), None);
    }
}
