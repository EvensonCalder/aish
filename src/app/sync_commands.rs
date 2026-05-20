use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::{self, write_private_file};
use crate::encryption::{atomic_gpg_encrypt_bytes, gpg_decrypt_file, gpg_program};
use crate::log::EventLevel;
use crate::sync::{
    GitCommandPlan, StartupSyncDecision, SyncFailureKind, SyncLock, SyncRepositoryContentMetadata,
    SyncRepositoryMetadata, SyncStepOutcome, classify_git_sync_step,
    conservative_sync_plan_for_existing_paths_with_encryption,
    disabled_existing_managed_paths_with_encryption, encryption_fingerprint_is_valid,
    init_repo_plan, log_sync_failure, maintain_managed_gitattributes, maintain_managed_gitignore,
    maintain_sync_readme, push_plan, startup_sync_decision, sync_repository_metadata_file_matches,
    sync_repository_metadata_for, sync_repository_metadata_path, tracked_managed_files_warning,
    write_sync_repository_metadata,
};

use super::{
    AppState, encryption_commands::configured_encryption_key, reports::write_encryption_sync_status,
};

const DEFAULT_SYNC_GIT_USER_NAME: &str = "Aish Sync";
const DEFAULT_SYNC_GIT_USER_EMAIL: &str = "aish-sync@localhost";
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

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
        "resolve-union" => return resolve_interrupted_sync_with_union(state, out),
        "union" => {
            writeln!(out, "usage: #sync resolve-union")?;
            return Ok(());
        }
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
    if local_sync_repository_metadata(state, out)?.is_none() {
        return Ok(());
    }

    maintain_managed_gitignore(root.join(".gitignore"))?;
    maintain_managed_gitattributes(root.join(".gitattributes"))?;
    maintain_sync_readme(root.join("README.md"))?;
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
    if !ensure_sync_origin_remote(state, out, &root, &remote)? {
        return Ok(());
    }
    if !ensure_sync_git_identity(state, out, &root)? {
        return Ok(());
    }
    let current_branch = current_branch(&root)?;
    let mut remote_cache = prepare_remote_sync_cache(&root, &remote, current_branch.as_deref())?;
    if !align_local_branch_to_remote_sync_branch(state, out, &root, remote_cache.branch.as_deref())?
    {
        return Ok(());
    }
    if !adopt_remote_sync_repository_metadata(state, out, &remote_cache)? {
        return Ok(());
    }
    if !prepare_sync_repository_metadata(state, out, &root)? {
        return Ok(());
    }
    warn_disabled_existing_managed_paths(
        &root,
        &state.sync_config,
        state.encryption_config.enabled,
        out,
    )?;
    let Some(mut sync_data_snapshot) = verify_sync_data_for_git_write(state, out, &root)? else {
        return Ok(());
    };

    for command in conservative_sync_plan_for_existing_paths_with_encryption(
        &root,
        &state.sync_config,
        state.encryption_config.enabled,
    )
    .commands
    {
        if initialized_repo && is_pull_command(&command) {
            if remote_cache.branch.is_some() {
                if !run_verified_sync_cache_merge_step(
                    state,
                    out,
                    &root,
                    &remote_cache,
                    true,
                    &mut sync_data_snapshot,
                )? {
                    return Ok(());
                }
                if !prepare_sync_repository_metadata_after_pull(state, out, &root)? {
                    return Ok(());
                }
            } else {
                writeln!(
                    out,
                    "sync step skipped: remote cache has no branch to merge for new repository"
                )?;
            }
            continue;
        }
        if is_pull_command(&command) {
            if remote_cache.branch.is_some() {
                if !run_verified_sync_cache_merge_step(
                    state,
                    out,
                    &root,
                    &remote_cache,
                    false,
                    &mut sync_data_snapshot,
                )? {
                    return Ok(());
                }
                if !prepare_sync_repository_metadata_after_pull(state, out, &root)? {
                    return Ok(());
                }
            } else {
                writeln!(
                    out,
                    "sync step skipped: remote cache has no branch to merge because remote has no branch"
                )?;
            }
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
            if !run_sync_push_step(
                state,
                out,
                &root,
                &command,
                &mut remote_cache,
                &mut sync_data_snapshot,
            )? {
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

fn align_local_branch_to_remote_sync_branch(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
    remote_branch: Option<&str>,
) -> Result<bool> {
    let Some(remote_branch) = remote_branch else {
        return Ok(true);
    };
    let Some(local_branch) = current_branch(root)? else {
        return Ok(true);
    };
    if local_branch == remote_branch {
        return Ok(true);
    }
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "branch".to_string(),
            "-M".to_string(),
            remote_branch.to_string(),
        ],
    };
    run_sync_git_step(state, out, root, &command)
}

fn warn_disabled_existing_managed_paths(
    root: &Path,
    config: &config::SyncConfig,
    encryption_enabled: bool,
    out: &mut impl Write,
) -> Result<()> {
    for path in disabled_existing_managed_paths_with_encryption(root, config, encryption_enabled) {
        writeln!(
            out,
            "warning: sync.{}=false; not staging existing Aish file {}; run {} to include it",
            path.category, path.path, path.enable_command
        )?;
    }
    Ok(())
}

fn adopt_remote_sync_repository_metadata(
    state: &mut AppState,
    out: &mut impl Write,
    remote_cache: &RemoteSyncCache,
) -> Result<bool> {
    let metadata = match remote_cache.metadata() {
        Ok(Some(metadata)) => metadata,
        Ok(None) => return Ok(true),
        Err(err) => {
            writeln!(out, "remote sync metadata is invalid; refusing to sync")?;
            writeln!(out, "{err:#}")?;
            writeln!(
                out,
                "Fix the remote .aish-sync.toml on the sync branch, or remove it if this remote should be initialized by this machine, then run #sync now."
            )?;
            return Ok(false);
        }
    };
    let Some(local) = local_sync_repository_metadata(state, out)? else {
        return Ok(false);
    };
    if !sync_repository_encryption_matches(&metadata, &local) {
        write_sync_repository_metadata_mismatch(out, &metadata, &local)?;
        return Ok(false);
    }
    if metadata.content != local.content {
        apply_repository_sync_content_options(state, out, &metadata.content)?;
    }
    Ok(true)
}

#[derive(Debug, Clone)]
struct RemoteSyncCache {
    workspace: PathBuf,
    remote: String,
    branch: Option<String>,
}

impl RemoteSyncCache {
    fn fetched_ref(&self) -> Option<String> {
        self.branch
            .as_ref()
            .map(|branch| format!("refs/remotes/origin/{branch}"))
    }

    fn metadata(&self) -> Result<Option<SyncRepositoryMetadata>> {
        let Some(fetched_ref) = self.fetched_ref() else {
            return Ok(None);
        };
        let pathspec = format!("{fetched_ref}:{}", sync_repository_metadata_path());
        let show = GitCommandPlan {
            program: "git".to_string(),
            args: vec!["show".to_string(), pathspec],
        };
        let result = run_git_command(&self.workspace, &show)?;
        if !result.success {
            return Ok(None);
        }
        crate::sync::parse_sync_repository_metadata(&result.stdout)
            .map(Some)
            .context("remote sync metadata is invalid")
    }
}

#[derive(Debug, Clone)]
struct RemoteSyncRefs {
    head_branch: Option<String>,
    branches: Vec<String>,
}

fn prepare_remote_sync_cache(
    root: &Path,
    remote: &str,
    current_branch: Option<&str>,
) -> Result<RemoteSyncCache> {
    let workspace = prepare_remote_cache_workspace(root, remote)?;
    let refs = remote_sync_refs(&workspace)?;
    let branch = select_remote_sync_branch(&refs, current_branch);
    if let Some(branch) = &branch {
        fetch_remote_cache_branch(&workspace, branch)?;
    }
    Ok(RemoteSyncCache {
        workspace,
        remote: remote.to_string(),
        branch,
    })
}

fn refresh_remote_sync_cache(root: &Path, previous: &RemoteSyncCache) -> Result<RemoteSyncCache> {
    let current_branch = current_branch(root)?;
    prepare_remote_sync_cache(root, &previous.remote, current_branch.as_deref())
}

fn fetch_remote_cache_branch(workspace: &Path, branch: &str) -> Result<()> {
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let fetch = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["fetch".to_string(), "origin".to_string(), refspec],
    };
    let result = run_git_command(workspace, &fetch)?;
    if !result.success {
        bail!(
            "failed to fetch remote sync cache: {}",
            result.combined_output()
        );
    }
    Ok(())
}

fn remote_sync_refs(workspace: &Path) -> Result<RemoteSyncRefs> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "ls-remote".to_string(),
            "--symref".to_string(),
            "origin".to_string(),
            "HEAD".to_string(),
            "refs/heads/*".to_string(),
        ],
    };
    let result = run_git_command(workspace, &command)?;
    if !result.success {
        return Ok(RemoteSyncRefs {
            head_branch: None,
            branches: Vec::new(),
        });
    }
    Ok(parse_remote_sync_refs(&result.stdout))
}

fn parse_remote_sync_refs(raw: &str) -> RemoteSyncRefs {
    let mut head_branch = None;
    let mut branches = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("ref: refs/heads/")
            && let Some((branch, target)) = rest.split_once('\t')
            && target == "HEAD"
            && valid_remote_branch_name(branch)
        {
            head_branch = Some(branch.to_string());
            continue;
        }
        let Some((_, refname)) = line.split_once('\t') else {
            continue;
        };
        let Some(branch) = refname.strip_prefix("refs/heads/") else {
            continue;
        };
        if valid_remote_branch_name(branch) {
            branches.push(branch.to_string());
        }
    }
    branches.sort();
    branches.dedup();
    RemoteSyncRefs {
        head_branch,
        branches,
    }
}

fn valid_remote_branch_name(branch: &str) -> bool {
    !branch.is_empty() && !branch.chars().any(char::is_control)
}

fn select_remote_sync_branch(
    refs: &RemoteSyncRefs,
    current_branch: Option<&str>,
) -> Option<String> {
    if let Some(branch) = &refs.head_branch
        && refs.branches.iter().any(|candidate| candidate == branch)
    {
        return Some(branch.clone());
    }
    if let Some(branch) = current_branch
        && refs.branches.iter().any(|candidate| candidate == branch)
    {
        return Some(branch.to_string());
    }
    if refs.branches.len() == 1 {
        return refs.branches.first().cloned();
    }
    None
}

fn prepare_remote_cache_workspace(root: &Path, remote: &str) -> Result<PathBuf> {
    let workspace = root.join("cache/runtime/sync-remote-cache");
    match fs::remove_dir_all(&workspace) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to clear remote sync cache workspace {}",
                    workspace.display()
                )
            });
        }
    }
    fs::create_dir_all(&workspace).with_context(|| {
        format!(
            "failed to create remote sync cache workspace {}",
            workspace.display()
        )
    })?;
    let init = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["init".to_string()],
    };
    let init_result = run_git_command(&workspace, &init)?;
    if !init_result.success {
        bail!(
            "failed to initialize remote sync cache workspace: {}",
            init_result.combined_output()
        );
    }
    let remote_add = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "remote".to_string(),
            "add".to_string(),
            "origin".to_string(),
            remote.to_string(),
        ],
    };
    let remote_result = run_git_command(&workspace, &remote_add)?;
    if !remote_result.success {
        bail!(
            "failed to configure remote sync cache workspace: {}",
            remote_result.combined_output()
        );
    }
    Ok(workspace)
}

fn fetch_remote_cache_into_active_repo(
    state: &AppState,
    out: &mut impl Write,
    root: &Path,
    remote_cache: &RemoteSyncCache,
) -> Result<bool> {
    let Some(fetched_ref) = remote_cache.fetched_ref() else {
        return Ok(false);
    };
    let show = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "fetch".to_string(),
            remote_cache.workspace.display().to_string(),
            fetched_ref,
        ],
    };
    let result = run_git_command(root, &show)?;
    if result.success {
        writeln!(out, "sync step ok: {}", describe_git_command(&show))?;
        return Ok(true);
    }
    handle_failed_sync_step(state, out, root, &show, result)?;
    Ok(false)
}

fn prepare_sync_repository_metadata(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
) -> Result<bool> {
    let Some(expected) = local_sync_repository_metadata(state, out)? else {
        return Ok(false);
    };
    let path = root.join(sync_repository_metadata_path());
    if !sync_repository_metadata_file_matches(&path, &expected)? {
        write_sync_repository_metadata(&path, &expected)?;
    }
    Ok(true)
}

fn prepare_sync_repository_metadata_after_pull(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
) -> Result<bool> {
    if !prepare_sync_repository_metadata(state, out, root)? {
        return Ok(false);
    }
    warn_disabled_existing_managed_paths(
        root,
        &state.sync_config,
        state.encryption_config.enabled,
        out,
    )?;
    let Some(add) = conservative_sync_plan_for_existing_paths_with_encryption(
        root,
        &state.sync_config,
        state.encryption_config.enabled,
    )
    .commands
    .into_iter()
    .next() else {
        return Ok(true);
    };
    if !run_sync_git_step(state, out, root, &add)? {
        return Ok(false);
    }
    if git_has_staged_changes(root)? {
        let commit = crate::sync::default_sync_commit_plan();
        if !run_sync_git_step(state, out, root, &commit)? {
            return Ok(false);
        }
    } else {
        writeln!(out, "sync step skipped: metadata unchanged")?;
    }
    Ok(true)
}

fn local_sync_repository_metadata(
    state: &AppState,
    out: &mut impl Write,
) -> Result<Option<SyncRepositoryMetadata>> {
    if !state.encryption_config.enabled {
        return Ok(Some(sync_repository_metadata_for(
            &state.sync_config,
            false,
            "",
        )));
    }

    let key = configured_encryption_key(&state.encryption_config);
    if !encryption_fingerprint_is_valid(key) {
        writeln!(
            out,
            "sync encryption key is not configured as a full GPG fingerprint; refusing to sync encrypted data"
        )?;
        if key.is_empty() {
            writeln!(out, "local key_fingerprint=<empty>")?;
        } else {
            writeln!(out, "local key_fingerprint={key}")?;
        }
        writeln!(
            out,
            "run #encrypt rotate <full-key-fingerprint> after #unlock if encrypted storage needs a passphrase, then run #sync now"
        )?;
        return Ok(None);
    }
    Ok(Some(sync_repository_metadata_for(
        &state.sync_config,
        true,
        key,
    )))
}

fn sync_repository_encryption_matches(
    repository: &SyncRepositoryMetadata,
    local: &SyncRepositoryMetadata,
) -> bool {
    repository.version == local.version
        && repository.encryption.enabled == local.encryption.enabled
        && repository.encryption.key_fingerprint == local.encryption.key_fingerprint
        && (!repository.encryption.enabled
            || encryption_fingerprint_is_valid(&repository.encryption.key_fingerprint))
}

fn apply_repository_sync_content_options(
    state: &mut AppState,
    out: &mut impl Write,
    content: &SyncRepositoryContentMetadata,
) -> Result<()> {
    writeln!(
        out,
        "warning: repository sync content options differ; using repository sync options"
    )?;
    writeln!(out, "sync.ai={}", content.ai)?;
    writeln!(out, "sync.history={}", content.history)?;
    writeln!(out, "sync.templates={}", content.templates)?;
    writeln!(out, "sync.drafts={}", content.drafts)?;
    update_sync_config(state, |config| {
        config.sync.ai = content.ai;
        config.sync.history = content.history;
        config.sync.templates = content.templates;
        config.sync.drafts = content.drafts;
    })?;
    Ok(())
}

fn write_sync_repository_metadata_mismatch(
    out: &mut impl Write,
    repository: &SyncRepositoryMetadata,
    local: &SyncRepositoryMetadata,
) -> Result<()> {
    writeln!(
        out,
        "sync encryption key mismatch; refusing to sync until one repository key is chosen"
    )?;
    writeln!(
        out,
        "Aish checks remote sync metadata before choosing a merge or decryption path."
    )?;
    writeln!(
        out,
        "repository encryption={}",
        repository.encryption.enabled
    )?;
    writeln!(
        out,
        "repository key_fingerprint={}",
        metadata_fingerprint_display(repository)
    )?;
    writeln!(out, "local encryption={}", local.encryption.enabled)?;
    writeln!(
        out,
        "local key_fingerprint={}",
        metadata_fingerprint_display(local)
    )?;
    if repository.version != local.version {
        writeln!(out, "repository metadata version={}", repository.version)?;
    }
    if repository.encryption.enabled
        && !encryption_fingerprint_is_valid(&repository.encryption.key_fingerprint)
    {
        writeln!(
            out,
            "repository .aish-sync.toml has an invalid encryption fingerprint"
        )?;
    }
    writeln!(
        out,
        "Aish will not merge different encryption fingerprints automatically."
    )?;
    writeln!(
        out,
        "If this machine can decrypt both local and repository data, run #unlock if needed, then run #encrypt rotate <chosen-full-key-fingerprint> and #sync now."
    )?;
    writeln!(
        out,
        "To use the repository key here, import that private key first; if this machine cannot decrypt the data, resolve the key change on a machine that can."
    )?;
    Ok(())
}

fn metadata_fingerprint_display(metadata: &SyncRepositoryMetadata) -> &str {
    if metadata.encryption.key_fingerprint.is_empty() {
        "<none>"
    } else {
        &metadata.encryption.key_fingerprint
    }
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
    let Some(mut sync_data_snapshot) = verify_sync_data_for_git_write(state, out, root)? else {
        return Ok(());
    };
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
    let remote = state.sync_config.remote.trim().to_string();
    let current_branch = current_branch(root)?;
    let mut remote_cache = prepare_remote_sync_cache(root, &remote, current_branch.as_deref())?;
    if !run_sync_push_step(
        state,
        out,
        root,
        &push,
        &mut remote_cache,
        &mut sync_data_snapshot,
    )? {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncDataSnapshot {
    records: Vec<SyncDataRecordCount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncDataRecordCount {
    label: &'static str,
    path: String,
    count: usize,
    content_hash: u64,
    plaintext_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct ManagedSyncDataFile {
    label: &'static str,
    logical_path: &'static str,
}

fn verify_sync_data_for_git_write(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
) -> Result<Option<SyncDataSnapshot>> {
    let config = state.sync_config.clone();
    let encryption_enabled = state.encryption_config.enabled;
    let result = if encryption_enabled {
        state.run_unlock_passthrough(|_| collect_sync_data_snapshot(root, &config, true))
    } else {
        collect_sync_data_snapshot(root, &config, false)
    };
    match result {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(err) => {
            if encryption_enabled {
                writeln!(
                    out,
                    "sync encrypted data cannot be verified; refusing to stage, commit, or push"
                )?;
                writeln!(
                    out,
                    "Aish must decrypt enabled managed *.jsonl.gpg files before syncing them."
                )?;
                writeln!(out, "{err:#}")?;
                writeln!(
                    out,
                    "Run #unlock if GPG needs a passphrase, import the matching private key, or resolve the repository key choice with #encrypt rotate <full-key-fingerprint>."
                )?;
            } else {
                writeln!(
                    out,
                    "sync data cannot be verified; refusing to stage, commit, or push"
                )?;
                writeln!(out, "{err:#}")?;
            }
            Ok(None)
        }
    }
}

fn collect_sync_data_snapshot(
    root: &Path,
    config: &config::SyncConfig,
    encryption_enabled: bool,
) -> Result<SyncDataSnapshot> {
    let gpg = encryption_enabled.then(gpg_program);
    let mut records = Vec::new();
    for file in managed_sync_data_files(config) {
        records.push(count_managed_sync_data_file(
            root,
            file,
            encryption_enabled,
            gpg.as_deref(),
        )?);
    }
    Ok(SyncDataSnapshot { records })
}

fn managed_sync_data_files(config: &config::SyncConfig) -> Vec<ManagedSyncDataFile> {
    let mut files = Vec::new();
    if config.ai {
        files.push(ManagedSyncDataFile {
            label: "history/ai",
            logical_path: "history/ai.jsonl",
        });
    }
    if config.history {
        files.push(ManagedSyncDataFile {
            label: "history/notes",
            logical_path: "history/notes.jsonl",
        });
        files.push(ManagedSyncDataFile {
            label: "history/regular",
            logical_path: "history/regular.jsonl",
        });
    }
    if config.templates {
        files.push(ManagedSyncDataFile {
            label: "templates",
            logical_path: "templates/templates.jsonl",
        });
    }
    if config.drafts {
        files.push(ManagedSyncDataFile {
            label: "history/draft",
            logical_path: "history/draft.jsonl",
        });
    }
    files
}

fn count_managed_sync_data_file(
    root: &Path,
    file: ManagedSyncDataFile,
    encryption_enabled: bool,
    gpg_program: Option<&str>,
) -> Result<SyncDataRecordCount> {
    let path = if encryption_enabled {
        format!("{}.gpg", file.logical_path)
    } else {
        file.logical_path.to_string()
    };
    let absolute = root.join(&path);
    let bytes = if !absolute.exists() {
        Vec::new()
    } else if encryption_enabled {
        let Some(program) = gpg_program else {
            bail!("GPG program is not configured for encrypted sync");
        };
        gpg_decrypt_file(program, &absolute)
            .with_context(|| format!("failed to decrypt managed sync file {path}"))?
    } else {
        fs::read(&absolute).with_context(|| format!("failed to read managed sync file {path}"))?
    };
    let count = count_jsonl_records(&path, &bytes)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(SyncDataRecordCount {
        label: file.label,
        path,
        count,
        content_hash: hasher.finish(),
        plaintext_bytes: bytes,
    })
}

fn count_jsonl_records(path: &str, bytes: &[u8]) -> Result<usize> {
    let raw = std::str::from_utf8(bytes)
        .with_context(|| format!("managed sync file {path} is not valid UTF-8 JSONL"))?;
    Ok(raw.lines().filter(|line| !line.trim().is_empty()).count())
}

fn reconcile_sync_data_after_pull(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
    before: &SyncDataSnapshot,
    after: SyncDataSnapshot,
) -> Result<Option<SyncDataSnapshot>> {
    let mut restored = false;
    for after_record in &after.records {
        let before_record = before
            .records
            .iter()
            .find(|record| record.label == after_record.label);
        let before_count = before_record.map_or(0, |record| record.count);
        if before_count == after_record.count {
            if before_record.is_some_and(|record| record.content_hash != after_record.content_hash)
            {
                writeln!(
                    out,
                    "sync data summary: {} records {} -> {} (content changed)",
                    after_record.label, before_count, after_record.count
                )?;
            }
            continue;
        }
        let delta = after_record.count as isize - before_count as isize;
        if delta < 0 {
            writeln!(
                out,
                "warning: sync data count decreased: {} records {} -> {} ({})",
                after_record.label,
                before_count,
                after_record.count,
                format_record_delta(delta)
            )?;
            let Some(before_record) = before_record else {
                continue;
            };
            let merged = union_jsonl_bytes(
                &before_record.path,
                &before_record.plaintext_bytes,
                &after_record.plaintext_bytes,
            )?;
            write_union_restored_sync_data(state, root, after_record, &merged)?;
            let restored_count = count_jsonl_records(&after_record.path, &merged)?;
            let restored_delta = restored_count as isize - after_record.count as isize;
            writeln!(
                out,
                "sync data union-restored: {} records {} -> {} ({})",
                after_record.label,
                after_record.count,
                restored_count,
                format_record_delta(restored_delta)
            )?;
            restored = true;
        } else {
            writeln!(
                out,
                "sync data summary: {} records {} -> {} ({})",
                after_record.label,
                before_count,
                after_record.count,
                format_record_delta(delta)
            )?;
        }
    }
    if restored {
        return verify_sync_data_for_git_write(state, out, root);
    }
    Ok(Some(after))
}

fn format_record_delta(delta: isize) -> String {
    if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    }
}

fn union_jsonl_bytes(path: &str, before: &[u8], after: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let before_raw = std::str::from_utf8(before)
        .with_context(|| format!("managed sync file {path} is not valid UTF-8 JSONL"))?;
    let after_raw = std::str::from_utf8(after)
        .with_context(|| format!("managed sync file {path} is not valid UTF-8 JSONL"))?;
    let mut before_counts = HashMap::<String, usize>::new();

    for line in before_raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        *before_counts.entry(line.to_string()).or_default() += 1;
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');
    }

    let mut after_counts = HashMap::<String, usize>::new();
    for line in after_raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let count = after_counts.entry(line.to_string()).or_default();
        *count += 1;
        if *count <= before_counts.get(line).copied().unwrap_or_default() {
            continue;
        }
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn write_union_restored_sync_data(
    state: &AppState,
    root: &Path,
    record: &SyncDataRecordCount,
    plaintext: &[u8],
) -> Result<()> {
    let path = root.join(&record.path);
    if state.encryption_config.enabled {
        atomic_gpg_encrypt_bytes(
            gpg_program(),
            configured_encryption_key(&state.encryption_config),
            &path,
            plaintext,
        )
        .with_context(|| {
            format!(
                "failed to write union-restored encrypted sync file {}",
                record.path
            )
        })?;
    } else {
        write_private_file(&path, plaintext)
            .with_context(|| format!("failed to write union-restored sync file {}", record.path))?;
    }
    Ok(())
}

fn run_sync_push_step(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
    command: &GitCommandPlan,
    remote_cache: &mut RemoteSyncCache,
    snapshot: &mut SyncDataSnapshot,
) -> Result<bool> {
    let result = run_git_command(root, command)?;
    if result.success {
        writeln!(out, "sync step ok: {}", describe_git_command(command))?;
        return Ok(true);
    }
    if git_output_suggests_remote_changed(&result.combined_output()) {
        writeln!(
            out,
            "sync push needs remote updates; refreshing remote cache and merging"
        )?;
        *remote_cache = refresh_remote_sync_cache(root, remote_cache)?;
        if !adopt_remote_sync_repository_metadata(state, out, remote_cache)? {
            return Ok(false);
        }
        if remote_cache.branch.is_none() {
            handle_failed_sync_step(state, out, root, command, result)?;
            return Ok(false);
        }
        if !run_verified_sync_cache_merge_step(state, out, root, remote_cache, false, snapshot)? {
            return Ok(false);
        }
        if !prepare_sync_repository_metadata_after_pull(state, out, root)? {
            return Ok(false);
        }
        let retry = run_git_command(root, command)?;
        if retry.success {
            writeln!(out, "sync step ok: {}", describe_git_command(command))?;
            return Ok(true);
        }
        handle_failed_sync_step(state, out, root, command, retry)?;
        return Ok(false);
    }
    handle_failed_sync_step(state, out, root, command, result)?;
    Ok(false)
}

fn run_verified_sync_cache_merge_step(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
    remote_cache: &RemoteSyncCache,
    allow_unrelated: bool,
    snapshot: &mut SyncDataSnapshot,
) -> Result<bool> {
    let before = snapshot.clone();
    if !fetch_remote_cache_into_active_repo(state, out, root, remote_cache)? {
        return Ok(false);
    }
    if !run_sync_merge_step(state, out, root, allow_unrelated)? {
        return Ok(false);
    }
    let Some(after) = verify_sync_data_for_git_write(state, out, root)? else {
        return Ok(false);
    };
    let Some(reconciled) = reconcile_sync_data_after_pull(state, out, root, &before, after)? else {
        return Ok(false);
    };
    *snapshot = reconciled;
    Ok(true)
}

fn run_sync_merge_step(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
    allow_unrelated: bool,
) -> Result<bool> {
    let command = merge_fetch_head_plan(allow_unrelated);
    let result = run_git_command(root, &command)?;
    if result.success {
        writeln!(out, "sync step ok: {}", describe_git_command(&command))?;
        return Ok(true);
    }
    let detail = result.combined_output();
    if git_output_suggests_unrelated_histories(&detail) && !allow_unrelated {
        writeln!(
            out,
            "sync merge needs unrelated-history merge; retrying with --allow-unrelated-histories"
        )?;
        return run_sync_merge_step(state, out, root, true);
    }
    if matches!(
        classify_git_sync_step(false, &result.stdout, &result.stderr),
        SyncStepOutcome::AbortConflict { .. }
    ) && try_resolve_sync_metadata_pull_conflict(state, out, root)?
    {
        writeln!(out, "sync step ok: {}", describe_git_command(&command))?;
        return Ok(true);
    }
    handle_failed_sync_step(state, out, root, &command, result)?;
    Ok(false)
}

fn merge_fetch_head_plan(allow_unrelated: bool) -> GitCommandPlan {
    let mut args = vec![
        "merge".to_string(),
        "--no-edit".to_string(),
        "FETCH_HEAD".to_string(),
    ];
    if allow_unrelated {
        args.insert(2, "--allow-unrelated-histories".to_string());
    }
    GitCommandPlan {
        program: "git".to_string(),
        args,
    }
}

fn try_resolve_sync_metadata_pull_conflict(
    state: &mut AppState,
    out: &mut impl Write,
    root: &Path,
) -> Result<bool> {
    if !has_interrupted_merge(root) {
        return Ok(false);
    }
    let metadata_path = sync_repository_metadata_path();
    let unmerged = unmerged_paths(root)?;
    if !unmerged.iter().any(|path| path == metadata_path) {
        return Ok(false);
    }
    let Some(repository_metadata) = git_stage_sync_repository_metadata(root, 3)? else {
        return Ok(false);
    };
    let Some(local_metadata) = local_sync_repository_metadata(state, out)? else {
        return Ok(false);
    };
    if !sync_repository_encryption_matches(&repository_metadata, &local_metadata) {
        write_sync_repository_metadata_mismatch(out, &repository_metadata, &local_metadata)?;
        return Ok(false);
    }
    if repository_metadata.content != local_metadata.content {
        apply_repository_sync_content_options(state, out, &repository_metadata.content)?;
    }
    let resolved = local_sync_repository_metadata(state, out)?
        .expect("validated encryption config remains available");
    write_sync_repository_metadata(root.join(metadata_path), &resolved)?;
    let add = GitCommandPlan {
        program: "git".to_string(),
        args: git_add_args(&[metadata_path.to_string()]),
    };
    if !run_sync_git_step(state, out, root, &add)? {
        return Ok(false);
    }
    let remaining = unmerged_paths(root)?;
    if !remaining.is_empty() {
        writeln!(
            out,
            "sync metadata conflict resolved using repository sync options; remaining conflicts need user resolution"
        )?;
        return Ok(false);
    }
    let commit = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["commit".to_string(), "--no-edit".to_string()],
    };
    if !run_sync_git_step(state, out, root, &commit)? {
        return Ok(false);
    }
    Ok(true)
}

fn git_stage_sync_repository_metadata(
    root: &Path,
    stage: u8,
) -> Result<Option<SyncRepositoryMetadata>> {
    let pathspec = format!(":{stage}:{}", sync_repository_metadata_path());
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["show".to_string(), pathspec],
    };
    let result = run_git_command(root, &command)?;
    if !result.success {
        return Ok(None);
    }
    crate::sync::parse_sync_repository_metadata(&result.stdout)
        .map(Some)
        .with_context(|| format!("failed to parse git stage {stage} sync metadata"))
}

fn git_output_suggests_remote_changed(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("stale info")
}

fn git_output_suggests_unrelated_histories(output: &str) -> bool {
    output
        .to_ascii_lowercase()
        .contains("refusing to merge unrelated histories")
}

fn ensure_sync_origin_remote(
    state: &AppState,
    out: &mut impl Write,
    root: &Path,
    remote: &str,
) -> Result<bool> {
    match remote_origin_url(root)? {
        Some(current) if current == remote => Ok(true),
        Some(_) => {
            let command = GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "remote".to_string(),
                    "set-url".to_string(),
                    "origin".to_string(),
                    remote.to_string(),
                ],
            };
            run_sync_git_step(state, out, root, &command)
        }
        None => {
            let command = GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "remote".to_string(),
                    "add".to_string(),
                    "origin".to_string(),
                    remote.to_string(),
                ],
            };
            run_sync_git_step(state, out, root, &command)
        }
    }
}

fn remote_origin_url(root: &Path) -> Result<Option<String>> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "remote".to_string(),
            "get-url".to_string(),
            "origin".to_string(),
        ],
    };
    let result = run_git_command(root, &command)?;
    if result.success {
        let value = result.stdout.trim();
        return Ok((!value.is_empty()).then(|| value.to_string()));
    }
    Ok(None)
}

fn ensure_sync_git_identity(state: &AppState, out: &mut impl Write, root: &Path) -> Result<bool> {
    for (key, value) in [
        ("user.name", DEFAULT_SYNC_GIT_USER_NAME),
        ("user.email", DEFAULT_SYNC_GIT_USER_EMAIL),
    ] {
        if git_config_value(root, key)?.is_none() {
            let command = GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "config".to_string(),
                    "--local".to_string(),
                    key.to_string(),
                    value.to_string(),
                ],
            };
            if !run_sync_git_step(state, out, root, &command)? {
                return Ok(false);
            }
        }
    }
    if git_config_value(root, "commit.gpgsign")?.as_deref() != Some("false") {
        let command = GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "config".to_string(),
                "--local".to_string(),
                "commit.gpgsign".to_string(),
                "false".to_string(),
            ],
        };
        if !run_sync_git_step(state, out, root, &command)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn git_config_value(root: &Path, key: &str) -> Result<Option<String>> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["config".to_string(), "--get".to_string(), key.to_string()],
    };
    let result = run_git_command(root, &command)?;
    if result.success {
        let value = result.stdout.trim();
        return Ok((!value.is_empty()).then(|| value.to_string()));
    }
    if result.exit_code == Some(1) {
        return Ok(None);
    }
    bail!(
        "failed to inspect git config {key}: {}",
        result.combined_output()
    )
}

fn current_branch(root: &Path) -> Result<Option<String>> {
    let command = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["branch".to_string(), "--show-current".to_string()],
    };
    let result = run_git_command(root, &command)?;
    if !result.success {
        return Ok(None);
    }
    let branch = result.stdout.trim();
    Ok((!branch.is_empty() && !branch.chars().any(char::is_control)).then(|| branch.to_string()))
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
        writeln!(out, "startup sync enabled; running #sync now")?;
        return run_manual_sync_push(state, out);
    }
    match startup_sync_decision(
        &state.sync_config,
        now,
        read_last_sync_attempt(&last_attempt_path)?,
    ) {
        StartupSyncDecision::Due => {
            write_last_sync_attempt(&last_attempt_path, now)?;
            writeln!(out, "startup sync due; running #sync now")?;
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
    writeln!(out, "exit sync enabled; running #sync now")?;
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
    handle_failed_sync_step(state, out, root, command, result)?;
    Ok(false)
}

fn handle_failed_sync_step(
    state: &AppState,
    out: &mut impl Write,
    root: &Path,
    command: &GitCommandPlan,
    result: GitStepResult,
) -> Result<()> {
    let detail = result.combined_output();
    let outcome = classify_git_sync_step(false, &result.stdout, &result.stderr);
    match &outcome {
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
    if matches!(outcome, SyncStepOutcome::AbortConflict { .. }) {
        match unmerged_paths(root) {
            Ok(paths) if !paths.is_empty() => {
                writeln!(out, "sync unresolved conflicts: {}", paths.len())?;
                for path in paths {
                    writeln!(out, "conflict: {path}")?;
                }
            }
            Ok(_) => {}
            Err(err) => {
                writeln!(out, "sync conflict path listing failed: {err:#}")?;
            }
        }
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
    run_git_command_with_timeout(root, command, GIT_COMMAND_TIMEOUT)
}

fn run_git_command_with_timeout(
    root: &Path,
    command: &GitCommandPlan,
    timeout: Duration,
) -> Result<GitStepResult> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run {}", describe_git_command(command)))?;

    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .with_context(|| format!("failed to wait for {}", describe_git_command(command)))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .with_context(|| format!("failed to collect {}", describe_git_command(command)))?;
            return Ok(GitStepResult {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output().with_context(|| {
                format!(
                    "failed to collect timed out {}",
                    describe_git_command(command)
                )
            })?;
            let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if !stderr.ends_with('\n') && !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "git command timed out after {}s",
                timeout.as_millis().div_ceil(1000)
            ));
            return Ok(GitStepResult {
                success: false,
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr,
                exit_code: output.status.code(),
            });
        }
        thread::sleep(Duration::from_millis(50));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn git_command_timeout_returns_prompt_control_quickly() {
        let temp = tempfile::tempdir().unwrap();
        let command = GitCommandPlan {
            program: "sleep".to_string(),
            args: vec!["5".to_string()],
        };

        let started = Instant::now();
        let result =
            run_git_command_with_timeout(temp.path(), &command, Duration::from_millis(100))
                .unwrap();

        assert!(!result.success);
        assert!(result.stderr.contains("git command timed out after 1s"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
