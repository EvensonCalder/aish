use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config;
use crate::log::EventLevel;
use crate::sync::{
    GitCommandPlan, StartupSyncDecision, SyncFailureKind, SyncLock, SyncStepOutcome,
    classify_git_sync_step, conservative_sync_plan_for_existing_paths_with_encryption,
    init_repo_plan, log_sync_failure, maintain_managed_gitattributes, maintain_managed_gitignore,
    pull_merge_plan, push_plan, startup_sync_decision, tracked_managed_files_warning,
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
    match args {
        "now" => return run_manual_sync_push(state, out),
        "abort" => return abort_interrupted_sync(state, out),
        "continue" => return continue_interrupted_sync(state, out),
        "resolve-union" | "union" => return resolve_interrupted_sync_with_union(state, out),
        _ => {}
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
    if let Some((trigger, enabled)) = parse_sync_trigger_toggle(args) {
        update_sync_config(state, |config| match trigger {
            "startup" => config.sync.startup = enabled,
            "exit" => config.sync.exit = enabled,
            _ => unreachable!("validated trigger"),
        })?;
        writeln!(out, "sync.{trigger}={enabled}")?;
        writeln!(out, "no scheduler file created")?;
        return Ok(());
    }
    if is_malformed_sync_trigger_toggle(args) {
        writeln!(out, "usage: #sync startup|exit on|off")?;
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
    state.flush_encrypted_writes()?;
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };

    maintain_managed_gitignore(root.join(".gitignore"))?;
    maintain_managed_gitattributes(root.join(".gitattributes"))?;
    let mut initialized_repo = false;
    if root.join(".git").is_dir() {
        warn_tracked_managed_paths(&root, out)?;
    } else if let Some(plan) = init_repo_plan(&remote) {
        for command in &plan.commands {
            if !run_sync_git_step(state, out, &root, command)? {
                return Ok(());
            }
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
        if initialized_repo && is_pull_command(&command) {
            writeln!(
                out,
                "sync step skipped: git pull --no-rebase --no-edit for new repository"
            )?;
            continue;
        }
        if is_commit_command(&command) {
            if git_has_staged_changes(&root)? {
                if !run_sync_git_step(state, out, &root, &command)? {
                    return Ok(());
                }
            } else {
                writeln!(out, "sync step skipped: nothing to commit")?;
            }
            continue;
        }
        if is_push_command(&command) {
            if !run_sync_push_step(state, out, &root, &command)? {
                return Ok(());
            }
            continue;
        }
        if !run_sync_git_step(state, out, &root, &command)? {
            return Ok(());
        }
    }
    state.append_event(EventLevel::Info, "sync push completed")?;
    writeln!(out, "sync push completed")?;
    Ok(())
}

fn abort_interrupted_sync(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(root) = sync_root(state) else {
        writeln!(out, "config path is not configured; sync abort cannot run")?;
        return Ok(());
    };
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };

    if has_interrupted_merge(&root) {
        let command = GitCommandPlan {
            program: "git".to_string(),
            args: vec!["merge".to_string(), "--abort".to_string()],
        };
        if run_sync_git_step(state, out, &root, &command)? {
            writeln!(out, "sync abort completed")?;
        }
        return Ok(());
    }
    if has_interrupted_rebase(&root) {
        let command = GitCommandPlan {
            program: "git".to_string(),
            args: vec!["rebase".to_string(), "--abort".to_string()],
        };
        if run_sync_git_step(state, out, &root, &command)? {
            writeln!(out, "sync abort completed")?;
        }
        return Ok(());
    }

    writeln!(out, "no interrupted sync to abort")?;
    Ok(())
}

fn continue_interrupted_sync(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(root) = sync_root(state) else {
        writeln!(
            out,
            "config path is not configured; sync continue cannot run"
        )?;
        return Ok(());
    };
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };

    if has_interrupted_merge(&root) {
        return commit_interrupted_merge_and_push(state, out, &root);
    }
    if has_interrupted_rebase(&root) {
        writeln!(
            out,
            "interrupted rebase detected; run git rebase --continue manually after resolving conflicts, or run #sync abort"
        )?;
        return Ok(());
    }

    writeln!(out, "no interrupted sync to continue")?;
    Ok(())
}

fn resolve_interrupted_sync_with_union(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(root) = sync_root(state) else {
        writeln!(
            out,
            "config path is not configured; sync resolve-union cannot run"
        )?;
        return Ok(());
    };
    let lock_path = root.join("cache/runtime/sync.lock");
    let Some(_lock) = SyncLock::acquire(&lock_path)? else {
        writeln!(out, "sync is already running")?;
        return Ok(());
    };
    if !has_interrupted_merge(&root) {
        if has_interrupted_rebase(&root) {
            writeln!(
                out,
                "interrupted rebase detected; #sync resolve-union supports merge conflicts only; run #sync abort or resolve manually"
            )?;
        } else {
            writeln!(out, "no interrupted sync to resolve")?;
        }
        return Ok(());
    }

    let paths = unmerged_paths(&root)?;
    if paths.is_empty() {
        writeln!(out, "no unresolved sync conflicts found")?;
        return Ok(());
    }
    let unsafe_paths: Vec<&String> = paths
        .iter()
        .filter(|path| !auto_union_allowed_path(path))
        .collect();
    if !unsafe_paths.is_empty() {
        writeln!(
            out,
            "sync resolve-union refused non-plaintext or unmanaged conflict(s)"
        )?;
        for path in unsafe_paths {
            writeln!(out, "manual: {path}")?;
        }
        writeln!(
            out,
            "resolve those files manually, then run #sync continue; or run #sync abort"
        )?;
        return Ok(());
    }

    for path in &paths {
        resolve_conflict_file_by_union(&root.join(path))
            .with_context(|| format!("failed to union-resolve {}", path))?;
        writeln!(out, "sync conflict union-resolved: {path}")?;
    }
    let add_command = GitCommandPlan {
        program: "git".to_string(),
        args: git_add_args(&paths),
    };
    if !run_sync_git_step(state, out, &root, &add_command)? {
        return Ok(());
    }
    commit_interrupted_merge_and_push(state, out, &root)
}

fn commit_interrupted_merge_and_push(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
) -> Result<()> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["commit".to_string(), "--no-edit".to_string()],
    };
    if !run_sync_git_step(state, out, root, &command)? {
        let unresolved = unmerged_paths(root)?;
        if !unresolved.is_empty() {
            writeln!(
                out,
                "resolve conflicts manually and run git add, then #sync continue; or run #sync resolve-union for plaintext Aish files; or run #sync abort"
            )?;
        }
        return Ok(());
    }
    let push = push_plan();
    if !run_sync_push_step(state, out, root, &push)? {
        return Ok(());
    }
    state.append_event(EventLevel::Info, "sync push completed")?;
    writeln!(out, "sync push completed")?;
    Ok(())
}

fn has_interrupted_merge(root: &Path) -> bool {
    root.join(".git/MERGE_HEAD").exists()
}

fn has_interrupted_rebase(root: &Path) -> bool {
    root.join(".git/rebase-merge").exists() || root.join(".git/rebase-apply").exists()
}

fn unmerged_paths(root: &Path) -> Result<Vec<String>> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "diff".to_string(),
            "--name-only".to_string(),
            "--diff-filter=U".to_string(),
            "--".to_string(),
        ],
    };
    let result = run_git_command(root, &command)?;
    if !result.success {
        bail!(
            "failed to list unresolved git conflicts: {}",
            result.combined_output()
        );
    }
    Ok(result
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn auto_union_allowed_path(path: &str) -> bool {
    (path == ".gitignore"
        || path == ".gitattributes"
        || (path.starts_with("history/") && path.ends_with(".jsonl"))
        || (path.starts_with("templates/") && path.ends_with(".jsonl")))
        && !path.ends_with(".gpg")
}

fn resolve_conflict_file_by_union(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read conflicted file {}", path.display()))?;
    let Some(resolved) = union_conflict_markers(&raw) else {
        bail!("file has no standard git conflict markers");
    };
    fs::write(path, resolved)
        .with_context(|| format!("failed to write union-resolved file {}", path.display()))
}

fn union_conflict_markers(raw: &str) -> Option<String> {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Side {
        Ours,
        Base,
        Theirs,
    }

    let mut output = String::new();
    let mut lines = raw.split_inclusive('\n');
    let mut changed = false;
    while let Some(line) = lines.next() {
        if !line.starts_with("<<<<<<<") {
            output.push_str(line);
            continue;
        }

        changed = true;
        let mut ours = Vec::new();
        let mut theirs = Vec::new();
        let mut side = Side::Ours;
        let mut saw_separator = false;
        let mut saw_end = false;
        for conflict_line in lines.by_ref() {
            if conflict_line.starts_with("|||||||") && side == Side::Ours {
                side = Side::Base;
                continue;
            }
            if conflict_line.starts_with("=======") {
                side = Side::Theirs;
                saw_separator = true;
                continue;
            }
            if conflict_line.starts_with(">>>>>>>") {
                saw_end = true;
                break;
            }
            match side {
                Side::Ours => ours.push(conflict_line),
                Side::Base => {}
                Side::Theirs => theirs.push(conflict_line),
            }
        }
        if !saw_separator || !saw_end {
            return None;
        }
        for kept in ours.into_iter().chain(theirs) {
            output.push_str(kept);
        }
    }

    changed.then_some(output)
}

fn git_add_args(paths: &[String]) -> Vec<String> {
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(paths.iter().cloned());
    args
}

fn git_has_staged_changes(root: &Path) -> Result<bool> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "diff".to_string(),
            "--cached".to_string(),
            "--quiet".to_string(),
            "--exit-code".to_string(),
        ],
    };
    let result = run_git_command(root, &command)?;
    match result.exit_code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => bail!(
            "failed to inspect staged sync changes: {}",
            result.combined_output()
        ),
    }
}

fn run_sync_push_step(
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
    if git_output_suggests_remote_changed(&result.combined_output()) {
        writeln!(
            out,
            "sync push needs remote updates; running git pull --no-rebase --no-edit"
        )?;
        let pull = pull_merge_plan();
        if !run_sync_git_step(state, out, root, &pull)? {
            return Ok(false);
        }
        let retry = run_git_command(root, command)?;
        if retry.success {
            writeln!(out, "sync step ok: {}", describe_git_command(command))?;
            return Ok(true);
        }
        handle_failed_sync_step(state, out, command, retry)?;
        return Ok(false);
    }
    handle_failed_sync_step(state, out, command, result)?;
    Ok(false)
}

fn git_output_suggests_remote_changed(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("stale info")
}

pub(super) fn run_startup_sync_check(
    state: &mut AppState,
    root: &Path,
    out: &mut impl Write,
) -> Result<()> {
    let last_attempt_path = root.join("cache/runtime/sync.last_attempt");
    let now = (state.clock)();
    if state.sync_config.startup {
        write_last_sync_attempt(&last_attempt_path, now)?;
        writeln!(out, "startup sync enabled; running #push")?;
        return run_manual_sync_push(state, out);
    }
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

pub(crate) fn run_exit_sync_if_enabled(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    if !state.sync_config.exit {
        return Ok(());
    }
    writeln!(out, "exit sync enabled; running #push")?;
    run_manual_sync_push(state, out)
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
    if matches!(
        classify_git_sync_step(false, &result.stdout, &result.stderr),
        SyncStepOutcome::AbortConflict { .. }
    ) {
        writeln!(
            out,
            "options: #sync resolve-union for plaintext Aish files, #sync continue after manual resolution, or #sync abort"
        )?;
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct GitStepResult {
    pub(super) success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: Option<i32>,
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
        exit_code: output.status.code(),
    })
}

fn is_commit_command(command: &GitCommandPlan) -> bool {
    command.program == "git" && command.args.first().is_some_and(|arg| arg == "commit")
}

fn is_pull_command(command: &GitCommandPlan) -> bool {
    command.program == "git" && command.args.first().is_some_and(|arg| arg == "pull")
}

fn is_push_command(command: &GitCommandPlan) -> bool {
    command.program == "git" && command.args.first().is_some_and(|arg| arg == "push")
}

pub(super) fn describe_git_command(command: &GitCommandPlan) -> String {
    let mut parts = Vec::with_capacity(command.args.len() + 1);
    parts.push(command.program.as_str());
    parts.extend(command.args.iter().map(String::as_str));
    parts.join(" ")
}

fn parse_sync_trigger_toggle(args: &str) -> Option<(&str, bool)> {
    parse_named_bool_toggle(args, is_sync_trigger)
}

fn is_malformed_sync_trigger_toggle(args: &str) -> bool {
    let mut parts = args.split_whitespace();
    let Some(trigger) = parts.next() else {
        return false;
    };
    is_sync_trigger(trigger)
}

fn is_sync_trigger(value: &str) -> bool {
    matches!(value, "startup" | "exit")
}

fn parse_sync_category_toggle(args: &str) -> Option<(&str, bool)> {
    parse_named_bool_toggle(args, is_sync_category)
}

fn parse_named_bool_toggle(
    args: &str,
    is_allowed_name: impl Fn(&str) -> bool,
) -> Option<(&str, bool)> {
    let mut parts = args.split_whitespace();
    let name = parts.next()?;
    let value = parts.next()?;
    if parts.next().is_some() || !is_allowed_name(name) {
        return None;
    }
    match value {
        "on" => Some((name, true)),
        "off" => Some((name, false)),
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
