use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{SyncConfig, create_private_dir_all, set_private_file_handle_permissions};
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event};

const GITIGNORE_BEGIN: &str = "# BEGIN AISH MANAGED";
const GITIGNORE_END: &str = "# END AISH MANAGED";
const GITATTRIBUTES_BEGIN: &str = "# BEGIN AISH MANAGED";
const GITATTRIBUTES_END: &str = "# END AISH MANAGED";
const MANAGED_GITIGNORE_LINES: &[&str] = &["cache/", "logs/", "secrets/", "config.toml", "*.tmp"];
const MANAGED_GITATTRIBUTES_LINES: &[&str] = &[
    "history/*.jsonl merge=union",
    "templates/*.jsonl merge=union",
];
const SYNC_README_PATH: &str = "SYNC.md";
const SYNC_README_CONTENT: &str = r#"# Aish Sync Repository

This Git repository is managed by Aish sync.

It stores Aish-managed shell history, notes, AI history, drafts, templates, and
sync metadata for one user across machines.

Do not use this repository as a normal source-code checkout. Aish updates it
with `#sync now` and `#push`.

Local-only files such as `config.toml`, cache, logs, secrets, and temporary
files are intentionally ignored.

Plaintext Aish JSONL files use Git's union merge driver so independent appends
from multiple machines usually keep both sides. Encrypted `*.jsonl.gpg` files
must not be text-union merged because that can corrupt ciphertext.
"#;

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
pub struct ExistingManagedAddPlan {
    pub paths: Vec<String>,
    pub missing_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitRepoPlan {
    pub commands: Vec<GitCommandPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConservativeSyncPlan {
    pub commands: Vec<GitCommandPlan>,
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
            create_private_dir_all(parent).with_context(|| {
                format!("failed to create sync lock directory {}", parent.display())
            })?;
        }

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(file) => {
                set_private_file_handle_permissions(&file, path)?;
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

pub fn maintain_managed_gitattributes(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let existing = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read gitattributes {}", path.display()))?
        }
    };
    let next = replace_managed_section(
        &existing,
        GITATTRIBUTES_BEGIN,
        GITATTRIBUTES_END,
        &managed_gitattributes_section(),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create gitattributes directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(path, next)
        .with_context(|| format!("failed to write gitattributes {}", path.display()))
}

pub fn maintain_sync_readme(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create sync readme directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(path, SYNC_README_CONTENT)
        .with_context(|| format!("failed to write sync readme {}", path.display()))
}

fn replace_managed_gitignore_section(existing: &str) -> String {
    replace_managed_section(
        existing,
        GITIGNORE_BEGIN,
        GITIGNORE_END,
        &managed_gitignore_section(),
    )
}

fn replace_managed_section(
    existing: &str,
    begin_marker: &str,
    end_marker: &str,
    managed: &str,
) -> String {
    let lines: Vec<&str> = existing.lines().collect();
    let start = lines.iter().position(|line| line.trim() == begin_marker);
    let end = lines.iter().position(|line| line.trim() == end_marker);

    if let (Some(start), Some(end)) = (start, end)
        && start <= end
    {
        let mut output = String::new();
        for line in &lines[..start] {
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(managed);
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
    output.push_str(managed);
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

fn managed_gitattributes_section() -> String {
    let mut output = String::new();
    output.push_str(GITATTRIBUTES_BEGIN);
    output.push('\n');
    for line in MANAGED_GITATTRIBUTES_LINES {
        output.push_str(line);
        output.push('\n');
    }
    output.push_str(GITATTRIBUTES_END);
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
        || normalized == "config.toml"
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
    managed_add_plan_with_encryption(config, false)
}

pub fn managed_add_plan_with_encryption(
    config: &SyncConfig,
    encryption_enabled: bool,
) -> ManagedAddPlan {
    let mut paths = vec![
        ".gitattributes".to_string(),
        ".gitignore".to_string(),
        SYNC_README_PATH.to_string(),
    ];
    if config.ai {
        paths.push(managed_storage_path("history/ai.jsonl", encryption_enabled));
    }
    if config.history {
        paths.push(managed_storage_path(
            "history/notes.jsonl",
            encryption_enabled,
        ));
        paths.push(managed_storage_path(
            "history/regular.jsonl",
            encryption_enabled,
        ));
    }
    if config.templates {
        paths.push(managed_storage_path(
            "templates/templates.jsonl",
            encryption_enabled,
        ));
    }
    if config.drafts {
        paths.push(managed_storage_path(
            "history/draft.jsonl",
            encryption_enabled,
        ));
    }
    paths.sort();
    paths.dedup();
    ManagedAddPlan { paths }
}

fn managed_storage_path(path: &str, encryption_enabled: bool) -> String {
    if encryption_enabled {
        format!("{path}.gpg")
    } else {
        path.to_string()
    }
}

pub fn existing_managed_add_plan(
    root: impl AsRef<Path>,
    config: &SyncConfig,
) -> ExistingManagedAddPlan {
    existing_managed_add_plan_with_encryption(root, config, false)
}

pub fn existing_managed_add_plan_with_encryption(
    root: impl AsRef<Path>,
    config: &SyncConfig,
    encryption_enabled: bool,
) -> ExistingManagedAddPlan {
    let root = root.as_ref();
    let mut paths = Vec::new();
    let mut missing_paths = Vec::new();
    for path in managed_add_plan_with_encryption(config, encryption_enabled).paths {
        if path == ".gitignore"
            || path == ".gitattributes"
            || path == SYNC_README_PATH
            || root.join(&path).exists()
        {
            paths.push(path);
        } else {
            missing_paths.push(path);
        }
    }
    ExistingManagedAddPlan {
        paths,
        missing_paths,
    }
}

pub fn pull_rebase_plan() -> GitCommandPlan {
    GitCommandPlan {
        program: "git".to_string(),
        args: vec!["pull".to_string(), "--rebase".to_string()],
    }
}

pub fn pull_merge_plan() -> GitCommandPlan {
    GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "pull".to_string(),
            "--no-rebase".to_string(),
            "--no-edit".to_string(),
        ],
    }
}

pub fn pull_merge_allow_unrelated_plan() -> GitCommandPlan {
    GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "pull".to_string(),
            "--no-rebase".to_string(),
            "--no-edit".to_string(),
            "--allow-unrelated-histories".to_string(),
        ],
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
    GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "commit".to_string(),
            "-m".to_string(),
            "sync aish data".to_string(),
        ],
    }
}

pub fn push_plan() -> GitCommandPlan {
    GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            "HEAD".to_string(),
        ],
    }
}

pub fn conservative_sync_plan(config: &SyncConfig) -> ConservativeSyncPlan {
    let add_plan = managed_add_plan(config);
    let commands = vec![
        GitCommandPlan {
            program: "git".to_string(),
            args: git_add_args(&add_plan.paths),
        },
        default_sync_commit_plan(),
        pull_merge_plan(),
        push_plan(),
    ];
    ConservativeSyncPlan { commands }
}

pub fn conservative_sync_plan_for_existing_paths(
    root: impl AsRef<Path>,
    config: &SyncConfig,
) -> ConservativeSyncPlan {
    conservative_sync_plan_for_existing_paths_with_encryption(root, config, false)
}

pub fn conservative_sync_plan_for_existing_paths_with_encryption(
    root: impl AsRef<Path>,
    config: &SyncConfig,
    encryption_enabled: bool,
) -> ConservativeSyncPlan {
    let add_plan = existing_managed_add_plan_with_encryption(root, config, encryption_enabled);
    let commands = vec![
        GitCommandPlan {
            program: "git".to_string(),
            args: git_add_args(&add_plan.paths),
        },
        default_sync_commit_plan(),
        pull_merge_plan(),
        push_plan(),
    ];
    ConservativeSyncPlan { commands }
}

fn git_add_args(paths: &[String]) -> Vec<String> {
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(paths.iter().cloned());
    args
}

pub fn init_repo_plan(remote: &str) -> Option<InitRepoPlan> {
    let remote = sanitize_remote(remote)?;
    Some(InitRepoPlan {
        commands: vec![
            GitCommandPlan {
                program: "git".to_string(),
                args: vec!["init".to_string()],
            },
            GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "remote".to_string(),
                    "add".to_string(),
                    "origin".to_string(),
                    remote,
                ],
            },
        ],
    })
}

fn sanitize_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() || remote.chars().any(char::is_control) {
        return None;
    }
    Some(remote.to_string())
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
mod tests;
