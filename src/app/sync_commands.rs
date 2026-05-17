use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::config;
use crate::log::EventLevel;
use crate::sync::{
    GitCommandPlan, StartupSyncDecision, SyncFailureKind, SyncLock, SyncStepOutcome,
    classify_git_sync_step, conservative_sync_plan_for_existing_paths_with_encryption,
    init_repo_plan, log_sync_failure, maintain_managed_gitignore, startup_sync_decision,
    tracked_managed_files_warning,
};

use super::{AppState, reports::write_encryption_sync_status};

pub(super) fn set_sync_remote(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let remote = args.trim();
    if remote.is_empty() {
        writeln!(out, "usage: #set-remote <git-url>")?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; sync config not saved")?;
        return Ok(());
    }

    update_sync_config(state, |config| {
        config.sync.remote = remote.to_string();
    })?;
    writeln!(out, "sync.remote={remote}")?;
    writeln!(out, "no git command run")?;
    Ok(())
}

pub(super) fn set_sync_schedule(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let args = args.trim();
    if args.is_empty() {
        write_encryption_sync_status(state, out)?;
        writeln!(out, "no git command run")?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; sync config not saved")?;
        return Ok(());
    }
    if args == "off" {
        update_sync_config(state, |config| {
            config.sync.enabled = false;
            config.sync.schedule.clear();
        })?;
        writeln!(out, "sync.enabled=false")?;
        writeln!(out, "no scheduler file created")?;
        return Ok(());
    }
    if let Some((category, enabled)) = parse_sync_category_toggle(args) {
        update_sync_config(state, |config| match category {
            "ai" => config.sync.ai = enabled,
            "history" => config.sync.history = enabled,
            "templates" => config.sync.templates = enabled,
            "drafts" => config.sync.drafts = enabled,
            _ => unreachable!("validated category"),
        })?;
        writeln!(out, "sync.{category}={enabled}")?;
        writeln!(out, "no git command run")?;
        return Ok(());
    }
    if is_malformed_sync_category_toggle(args) {
        writeln!(out, "usage: #sync ai|history|templates|drafts on|off")?;
        return Ok(());
    }

    update_sync_config(state, |config| {
        config.sync.enabled = true;
        config.sync.schedule = args.to_string();
    })?;
    writeln!(out, "sync.enabled=true")?;
    writeln!(out, "sync.schedule={args}")?;
    writeln!(out, "no scheduler file created")?;
    Ok(())
}

pub(super) fn run_manual_sync_push(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let remote = state.sync_config.remote.trim().to_string();
    if remote.is_empty() {
        writeln!(
            out,
            "sync remote is not configured; run #set-remote <git-url> first"
        )?;
        return Ok(());
    }
    let Some(root) = sync_root(state) else {
        writeln!(out, "config path is not configured; sync push cannot run")?;
        return Ok(());
    };
    if state.has_pending_locked_writes() {
        state.unlock_encrypted_storage_interactively()?;
    }
    state.flush_encrypted_writes()?;
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };

    maintain_managed_gitignore(root.join(".gitignore"))?;
    let mut initialized_repo = false;
    if root.join(".git").is_dir() {
        warn_tracked_managed_paths(&root, out)?;
    } else if let Some(plan) = init_repo_plan(&remote) {
        for command in &plan.commands {
            run_sync_git_step(state, out, &root, command)?;
        }
        initialized_repo = true;
    }

    for command in conservative_sync_plan_for_existing_paths_with_encryption(
        &root,
        &state.sync_config,
        state.encryption_config.enabled,
    )
    .commands
    {
        if initialized_repo && is_pull_rebase_command(&command) {
            writeln!(
                out,
                "sync step skipped: git pull --rebase for new repository"
            )?;
            continue;
        }
        if is_commit_command(&command) {
            let result = run_git_command(&root, &command)?;
            if result.success || git_output_is_nothing_to_commit(&result.combined_output()) {
                if result.success {
                    writeln!(out, "sync step ok: git commit")?;
                } else {
                    writeln!(out, "sync step skipped: nothing to commit")?;
                }
                continue;
            }
            handle_failed_sync_step(state, out, &command, result)?;
            return Ok(());
        }
        if !run_sync_git_step(state, out, &root, &command)? {
            return Ok(());
        }
    }
    state.append_event(EventLevel::Info, "sync push completed")?;
    writeln!(out, "sync push completed")?;
    Ok(())
}

pub(super) fn run_startup_sync_check(
    state: &mut AppState,
    root: &Path,
    out: &mut impl Write,
) -> Result<()> {
    let last_attempt_path = root.join("cache/runtime/sync.last_attempt");
    let now = (state.clock)();
    match startup_sync_decision(
        &state.sync_config,
        now,
        read_last_sync_attempt(&last_attempt_path)?,
    ) {
        StartupSyncDecision::Due => {
            write_last_sync_attempt(&last_attempt_path, now)?;
            writeln!(out, "startup sync due; running #push")?;
            run_manual_sync_push(state, out)?;
        }
        StartupSyncDecision::UnsupportedSchedule(schedule) => {
            state.append_event(
                EventLevel::Warn,
                &format!("startup sync unsupported schedule: {schedule}"),
            )?;
        }
        StartupSyncDecision::Disabled
        | StartupSyncDecision::MissingRemote
        | StartupSyncDecision::MissingSchedule
        | StartupSyncDecision::NotDue { .. } => {}
    }
    Ok(())
}

fn read_last_sync_attempt(path: &Path) -> Result<Option<i64>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(raw.trim().parse::<i64>().ok()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err)
            .with_context(|| format!("failed to read startup sync timestamp {}", path.display())),
    }
}

pub(super) fn write_last_sync_attempt(path: &Path, value: i64) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create startup sync timestamp directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(path, format!("{value}\n"))
        .with_context(|| format!("failed to write startup sync timestamp {}", path.display()))
}

fn sync_root(state: &AppState) -> Option<PathBuf> {
    state.config_path.as_ref()?.parent().map(Path::to_path_buf)
}

fn warn_tracked_managed_paths(root: &Path, out: &mut impl Write) -> Result<()> {
    let plan = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["ls-files".to_string()],
    };
    let result = run_git_command(root, &plan)?;
    if let Some(warning) = tracked_managed_files_warning(result.stdout.lines()) {
        writeln!(out, "{}", warning.message)?;
        for path in warning.paths {
            writeln!(out, "tracked: {path}")?;
        }
    }
    Ok(())
}

fn run_sync_git_step(
    state: &AppState,
    out: &mut impl Write,
    root: &Path,
    command: &GitCommandPlan,
) -> Result<bool> {
    let result = run_git_command(root, command)?;
    if result.success {
        writeln!(out, "sync step ok: {}", describe_git_command(command))?;
        return Ok(true);
    }
    handle_failed_sync_step(state, out, command, result)?;
    Ok(false)
}

fn handle_failed_sync_step(
    state: &AppState,
    out: &mut impl Write,
    command: &GitCommandPlan,
    result: GitStepResult,
) -> Result<()> {
    let detail = result.combined_output();
    match classify_git_sync_step(false, &result.stdout, &result.stderr) {
        SyncStepOutcome::AbortConflict { .. } => {
            writeln!(
                out,
                "sync aborted on conflict: {}",
                describe_git_command(command)
            )?;
            if let Some(path) = &state.events_path {
                log_sync_failure(path, (state.clock)(), SyncFailureKind::Conflict, &detail)?;
            }
        }
        SyncStepOutcome::AbortFailure { .. } => {
            writeln!(out, "sync failed: {}", describe_git_command(command))?;
            if let Some(path) = &state.events_path {
                log_sync_failure(path, (state.clock)(), SyncFailureKind::Failure, &detail)?;
            }
        }
        SyncStepOutcome::Continue => unreachable!("failed git step cannot continue"),
    }
    let detail = detail.trim();
    if !detail.is_empty() {
        writeln!(out, "{detail}")?;
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct GitStepResult {
    pub(super) success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl GitStepResult {
    pub(super) fn combined_output(&self) -> String {
        let stdout = self.stdout.trim();
        let stderr = self.stderr.trim();
        match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => String::new(),
            (false, true) => stdout.to_string(),
            (true, false) => stderr.to_string(),
            (false, false) => format!("{stdout}\n{stderr}"),
        }
    }
}

pub(super) fn run_git_command(root: &Path, command: &GitCommandPlan) -> Result<GitStepResult> {
    let output = Command::new(&command.program)
        .args(&command.args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run {}", describe_git_command(command)))?;
    Ok(GitStepResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn is_commit_command(command: &GitCommandPlan) -> bool {
    command.program == "git" && command.args.first().is_some_and(|arg| arg == "commit")
}

fn is_pull_rebase_command(command: &GitCommandPlan) -> bool {
    command.program == "git"
        && command
            .args
            .iter()
            .map(String::as_str)
            .eq(["pull", "--rebase"])
}

fn git_output_is_nothing_to_commit(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("nothing to commit") || lower.contains("no changes added to commit")
}

pub(super) fn describe_git_command(command: &GitCommandPlan) -> String {
    let mut parts = Vec::with_capacity(command.args.len() + 1);
    parts.push(command.program.as_str());
    parts.extend(command.args.iter().map(String::as_str));
    parts.join(" ")
}

fn parse_sync_category_toggle(args: &str) -> Option<(&str, bool)> {
    let mut parts = args.split_whitespace();
    let category = parts.next()?;
    let value = parts.next()?;
    if parts.next().is_some() || !is_sync_category(category) {
        return None;
    }
    match value {
        "on" => Some((category, true)),
        "off" => Some((category, false)),
        _ => None,
    }
}

fn is_malformed_sync_category_toggle(args: &str) -> bool {
    let mut parts = args.split_whitespace();
    let Some(category) = parts.next() else {
        return false;
    };
    is_sync_category(category)
}

fn is_sync_category(value: &str) -> bool {
    matches!(value, "ai" | "history" | "templates" | "drafts")
}

fn update_sync_config(
    state: &mut AppState,
    update: impl FnOnce(&mut config::Config),
) -> Result<()> {
    let Some(path) = &state.config_path else {
        anyhow::bail!("config path is not configured; sync config not saved");
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config);
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.sync_config = config.sync;
    state.append_event(EventLevel::Info, "sync config changed")?;
    Ok(())
}
