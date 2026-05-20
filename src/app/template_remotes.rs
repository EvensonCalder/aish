use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::{self, TemplateRemoteConfig};
use crate::encryption::{
    atomic_gpg_encrypt_bytes, gpg_decrypt_file, gpg_program, resolve_gpg_key_fingerprint,
};
use crate::git_remote::{sanitize_git_remote, valid_git_branch_name, valid_template_remote_name};
use crate::history::{JsonlLineError, JsonlLoad, rewrite_jsonl};
use crate::log::EventLevel;
use crate::sync::GitCommandPlan;
use crate::templates::{TemplateEntry, load_templates};

use super::sync_commands::{describe_git_command, run_git_command};
use super::{AppState, template_usage};

const TEMPLATE_REMOTE_README_PATH: &str = "README.md";
const TEMPLATE_REMOTE_METADATA_PATH: &str = ".aish-template-remote.toml";
const TEMPLATE_REMOTE_TEMPLATES_PATH: &str = "templates/templates.jsonl";
const TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH: &str = "templates/templates.jsonl.gpg";
const DEFAULT_TEMPLATE_REMOTE_BRANCH: &str = "main";
const FALLBACK_TEMPLATE_REMOTE_BRANCH: &str = "master";
const DEFAULT_TEMPLATE_GIT_USER_NAME: &str = "Aish Template Sharing";
const DEFAULT_TEMPLATE_GIT_USER_EMAIL: &str = "aish-templates@localhost";

const TEMPLATE_REMOTE_README: &str = r#"# Aish Template Remote

This Git repository is managed by Aish template sharing.

It stores shareable command templates only. Do not put private shell history, AI prompts, drafts, notes, config, logs, cache, or secrets in this repository.

## Files

- `.aish-template-remote.toml`: repository metadata used by Aish to recognize this as a template-only remote.
- `templates/templates.jsonl`: one plaintext template per line as JSON, for example `{"body":"git status"}`.
- `templates/templates.jsonl.gpg`: encrypted template payload when the publisher chooses `#template publish <name> --encrypt <key>`.

## Typical Flow

Publish from Aish:

```text
#template remote add shared <git-url>
#template publish shared
```

Import on another machine:

```text
#template remote add shared <git-url>
#template fetch shared
#template analyze shared
#template import shared <id|all>
```

`#template publish` updates only the files above and keeps templates that were already present in the remote. The owner may also edit `templates/templates.jsonl` directly and publish or push later.

Encrypted template remotes keep this README and `.aish-template-remote.toml` readable, but store template records in `templates/templates.jsonl.gpg`. Importers need a local private key that can decrypt that payload.
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateRemoteEncryption {
    Plain,
    Gpg { recipient_fingerprint: String },
}

impl TemplateRemoteEncryption {
    fn metadata_value(&self) -> &'static str {
        match self {
            Self::Plain => "none",
            Self::Gpg { .. } => "gpg",
        }
    }

    fn payload_path(&self) -> &'static str {
        match self {
            Self::Plain => TEMPLATE_REMOTE_TEMPLATES_PATH,
            Self::Gpg { .. } => TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH,
        }
    }

    fn opposite_payload_path(&self) -> &'static str {
        match self {
            Self::Plain => TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH,
            Self::Gpg { .. } => TEMPLATE_REMOTE_TEMPLATES_PATH,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplatePublishOptions {
    name: String,
    encryption: TemplateRemoteEncryption,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct TemplateRemoteMetadata {
    version: u32,
    kind: String,
    content: String,
    encryption: String,
    recipient_fingerprint: String,
}

impl Default for TemplateRemoteMetadata {
    fn default() -> Self {
        Self {
            version: 1,
            kind: "aish-template-remote".to_string(),
            content: "templates-only".to_string(),
            encryption: "none".to_string(),
            recipient_fingerprint: String::new(),
        }
    }
}

pub(super) fn template_remote_command(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let Some((subcommand, rest)) = next_word(args) else {
        writeln!(out, "{}", template_usage())?;
        return Ok(());
    };
    match subcommand {
        "remote" => template_remote_config_command(state, out, rest),
        "publish" => match parse_publish_args(rest, out)? {
            Some(options) => publish_templates(state, out, options),
            None => write_template_usage(out),
        },
        "fetch" => match single_word(rest) {
            Some(name) => fetch_templates(state, out, name),
            None => write_template_usage(out),
        },
        "analyze" => match next_word(rest) {
            Some((name, query)) => {
                let query = query.trim();
                analyze_templates(state, out, name, (!query.is_empty()).then_some(query))
            }
            None => write_template_usage(out),
        },
        "import" => match next_word(rest) {
            Some((name, selector)) => match single_word(selector) {
                Some(selector) => import_templates(state, out, name, selector),
                None => write_template_usage(out),
            },
            None => write_template_usage(out),
        },
        _ => write_template_usage(out),
    }
}

fn template_remote_config_command(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let Some((subcommand, rest)) = next_word(args) else {
        return write_template_usage(out);
    };
    match subcommand {
        "add" => match next_word(rest) {
            Some((name, remote)) if !remote.trim().is_empty() => {
                add_template_remote(state, out, name, remote)
            }
            _ => write_template_usage(out),
        },
        "list" if rest.trim().is_empty() => list_template_remotes(state, out),
        "rm" => match single_word(rest) {
            Some(name) => remove_template_remote(state, out, name),
            None => write_template_usage(out),
        },
        _ => write_template_usage(out),
    }
}

fn add_template_remote(
    state: &mut AppState,
    out: &mut impl Write,
    name: &str,
    remote: &str,
) -> Result<()> {
    let name = name.trim();
    let Some(remote) = sanitize_git_remote(remote) else {
        writeln!(
            out,
            "usage: #template remote add <name> <git-url>; name may contain letters, digits, dash, and underscore"
        )?;
        return Ok(());
    };
    if !valid_template_remote_name(name) {
        writeln!(
            out,
            "usage: #template remote add <name> <git-url>; name may contain letters, digits, dash, and underscore"
        )?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(
            out,
            "config path is not configured; template remote not saved"
        )?;
        return Ok(());
    }
    let cache_is_stale = state
        .template_sharing_config
        .remotes
        .iter()
        .find(|item| item.name == name)
        .map(|item| item.remote != remote)
        .unwrap_or(false);
    update_template_sharing_config(state, |config| {
        if let Some(existing) = config
            .template_sharing
            .remotes
            .iter_mut()
            .find(|item| item.name == name)
        {
            existing.remote = remote.clone();
        } else {
            config.template_sharing.remotes.push(TemplateRemoteConfig {
                name: name.to_string(),
                remote: remote.clone(),
            });
        }
    })?;
    if cache_is_stale {
        clear_template_remote_cache(state, out, name)?;
    }
    writeln!(out, "template.remote.{name}={remote}")?;
    writeln!(out, "no git command run")?;
    Ok(())
}

fn list_template_remotes(state: &AppState, out: &mut impl Write) -> Result<()> {
    if state.template_sharing_config.remotes.is_empty() {
        writeln!(out, "no template remotes configured")?;
        return Ok(());
    }
    for remote in &state.template_sharing_config.remotes {
        writeln!(out, "template remote {}\t{}", remote.name, remote.remote)?;
    }
    Ok(())
}

fn remove_template_remote(state: &mut AppState, out: &mut impl Write, name: &str) -> Result<()> {
    let name = name.trim();
    if !valid_template_remote_name(name) {
        writeln!(out, "usage: #template remote rm <name>")?;
        return Ok(());
    }
    if state.config_path.is_none() {
        writeln!(
            out,
            "config path is not configured; template remote not saved"
        )?;
        return Ok(());
    }
    let mut removed = false;
    update_template_sharing_config(state, |config| {
        let before = config.template_sharing.remotes.len();
        config
            .template_sharing
            .remotes
            .retain(|remote| remote.name != name);
        removed = before != config.template_sharing.remotes.len();
    })?;
    if removed {
        clear_template_remote_cache(state, out, name)?;
        writeln!(out, "template remote removed: {name}")?;
    } else {
        writeln!(out, "template remote not found: {name}")?;
    }
    Ok(())
}

fn publish_templates(
    state: &mut AppState,
    out: &mut impl Write,
    options: TemplatePublishOptions,
) -> Result<()> {
    if state.template_store_path.is_none() {
        writeln!(out, "template storage is not configured")?;
        return Ok(());
    }
    let name = options.name.as_str();
    let remote = match configured_template_remote(state, name) {
        Some(remote) => remote.clone(),
        None => {
            writeln!(out, "template remote not configured: {name}")?;
            return Ok(());
        }
    };
    state.flush_encrypted_writes()?;
    let local = state.load_templates()?;
    let Some(repo) = ensure_template_remote_repo(state, out, &remote)? else {
        return Ok(());
    };
    let Some(mut snapshot) = prepare_template_remote_snapshot(out, &repo)? else {
        return Ok(());
    };
    if !checkout_template_publish_branch(out, &repo, &snapshot)? {
        return Ok(());
    }
    if !validate_template_remote_worktree(out, &repo)? {
        return Ok(());
    }
    let Some(mut published_count) =
        write_template_remote_payload(out, &repo, &local.items, &options.encryption)?
    else {
        return Ok(());
    };
    if !stage_and_commit_template_payload(out, &repo, &options.encryption)? {
        return Ok(());
    }
    let push = template_push_plan();
    match run_template_push_step(out, &repo, &push)? {
        TemplatePushResult::Pushed => {}
        TemplatePushResult::RemoteChanged => {
            let Some(refreshed_snapshot) = prepare_template_remote_snapshot(out, &repo)? else {
                return Ok(());
            };
            snapshot = refreshed_snapshot;
            if !checkout_template_publish_branch(out, &repo, &snapshot)? {
                return Ok(());
            }
            if !validate_template_remote_worktree(out, &repo)? {
                return Ok(());
            }
            let Some(count) =
                write_template_remote_payload(out, &repo, &local.items, &options.encryption)?
            else {
                return Ok(());
            };
            published_count = count;
            if !stage_and_commit_template_payload(out, &repo, &options.encryption)? {
                return Ok(());
            }
            if !matches!(
                run_template_push_step(out, &repo, &push)?,
                TemplatePushResult::Pushed
            ) {
                return Ok(());
            }
        }
        TemplatePushResult::Failed => return Ok(()),
    }
    write_template_load_errors(out, local.errors.len())?;
    writeln!(
        out,
        "template publish completed: {name} (local={}, remote={published_count}, encryption={})",
        local.items.len(),
        options.encryption.metadata_value()
    )?;
    if local.items.is_empty() {
        writeln!(
            out,
            "template remote initialized with README, metadata, and an empty {}",
            options.encryption.payload_path()
        )?;
    }
    Ok(())
}

fn fetch_templates(state: &mut AppState, out: &mut impl Write, name: &str) -> Result<()> {
    let remote = match configured_template_remote(state, name) {
        Some(remote) => remote.clone(),
        None => {
            writeln!(out, "template remote not configured: {name}")?;
            return Ok(());
        }
    };
    let Some(repo) = ensure_template_remote_repo(state, out, &remote)? else {
        return Ok(());
    };
    let Some(snapshot) = prepare_template_remote_snapshot(out, &repo)? else {
        return Ok(());
    };
    if !checkout_template_fetched_branch(out, &repo, &snapshot, name)? {
        return Ok(());
    }
    if !validate_template_remote_worktree(out, &repo)? {
        return Ok(());
    }
    let Some(loaded) = load_pending_templates_for_user(out, &repo)? else {
        return Ok(());
    };
    write_template_load_errors(out, loaded.errors.len())?;
    writeln!(
        out,
        "template fetch completed: {name} (templates={})",
        loaded.items.len()
    )?;
    Ok(())
}

fn checkout_template_fetched_branch(
    out: &mut impl Write,
    repo: &Path,
    snapshot: &TemplateRemoteSnapshot,
    name: &str,
) -> Result<bool> {
    let Some(branch) = snapshot.branch.as_deref() else {
        writeln!(out, "template remote has no branch to fetch: {name}")?;
        return Ok(false);
    };
    let checkout = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "checkout".to_string(),
            "-B".to_string(),
            branch.to_string(),
            snapshot.fetched_ref(branch),
        ],
    };
    run_template_git_step(out, repo, &checkout)
}

fn analyze_templates(
    state: &AppState,
    out: &mut impl Write,
    name: &str,
    query: Option<&str>,
) -> Result<()> {
    if state.template_store_path.is_none() {
        writeln!(out, "template storage is not configured")?;
        return Ok(());
    }
    let Some(remote) = configured_template_remote(state, name) else {
        writeln!(out, "template remote not configured: {name}")?;
        return Ok(());
    };
    let repo = template_remote_repo_path(state, remote)?;
    if !repo.join(".git").is_dir() {
        writeln!(
            out,
            "no fetched templates for remote {name}; run #template fetch {name}"
        )?;
        return Ok(());
    }
    if !validate_template_remote_worktree(out, &repo)? {
        return Ok(());
    }
    let Some(pending) = load_pending_templates_for_user(out, &repo)? else {
        return Ok(());
    };
    if pending.items.is_empty() {
        writeln!(
            out,
            "no fetched templates for remote {name}; run #template fetch {name}"
        )?;
        return Ok(());
    }
    let local = state.load_templates()?;
    let existing: HashSet<String> = local.items.iter().map(TemplateEntry::id).collect();
    let mut matched = 0usize;
    let mut new_count = 0usize;
    let mut present_count = 0usize;
    for template in pending.items.iter().rev() {
        let id = template.id();
        if !query
            .map(|query| id.contains(query) || template.body.contains(query))
            .unwrap_or(true)
        {
            continue;
        }
        matched += 1;
        let status = if existing.contains(&id) {
            present_count += 1;
            "present"
        } else {
            new_count += 1;
            "new"
        };
        writeln!(out, "template {id}\t{status}\t{}", template.body)?;
    }
    if matched == 0 {
        writeln!(out, "no fetched templates matched: {}", query.unwrap_or(""))?;
    }
    write_template_load_errors(out, pending.errors.len())?;
    write_template_load_errors(out, local.errors.len())?;
    writeln!(
        out,
        "template analysis completed: fetched={} matched={matched} new={new_count} present={present_count}",
        pending.items.len()
    )?;
    Ok(())
}

fn import_templates(
    state: &mut AppState,
    out: &mut impl Write,
    name: &str,
    selector: &str,
) -> Result<()> {
    if state.template_store_path.is_none() {
        writeln!(out, "template storage is not configured")?;
        return Ok(());
    }
    let Some(remote) = configured_template_remote(state, name) else {
        writeln!(out, "template remote not configured: {name}")?;
        return Ok(());
    };
    let repo = template_remote_repo_path(state, remote)?;
    if !repo.join(".git").is_dir() {
        writeln!(
            out,
            "no fetched templates for remote {name}; run #template fetch {name}"
        )?;
        return Ok(());
    }
    if !validate_template_remote_worktree(out, &repo)? {
        return Ok(());
    }
    let Some(pending) = load_pending_templates_for_user(out, &repo)? else {
        return Ok(());
    };
    if pending.items.is_empty() {
        writeln!(
            out,
            "no fetched templates for remote {name}; run #template fetch {name}"
        )?;
        return Ok(());
    }
    let selected = select_pending_templates(&pending.items, selector)?;
    if selected.is_empty() {
        writeln!(
            out,
            "template not found in fetched remote {name}: {selector}"
        )?;
        return Ok(());
    }
    state.flush_encrypted_writes()?;
    let local = state.load_templates()?;
    let mut existing: HashSet<String> = local.items.iter().map(TemplateEntry::id).collect();
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for template in selected {
        let id = template.id();
        if existing.contains(&id) {
            skipped += 1;
            writeln!(out, "template already present: {id}")?;
            continue;
        }
        state.append_template(template)?;
        existing.insert(id.clone());
        imported += 1;
        writeln!(out, "template imported: {id}")?;
    }
    write_template_load_errors(out, pending.errors.len())?;
    write_template_load_errors(out, local.errors.len())?;
    writeln!(
        out,
        "template import completed: imported={imported} skipped={skipped}"
    )?;
    Ok(())
}

fn write_template_remote_payload(
    out: &mut impl Write,
    repo: &Path,
    local_templates: &[TemplateEntry],
    encryption: &TemplateRemoteEncryption,
) -> Result<Option<usize>> {
    let Some(remote_templates) = load_pending_templates_for_user(out, repo)? else {
        return Ok(None);
    };
    write_template_load_errors(out, remote_templates.errors.len())?;
    let merged = merge_template_entries(&remote_templates.items, local_templates);

    fs::write(
        repo.join(TEMPLATE_REMOTE_README_PATH),
        TEMPLATE_REMOTE_README,
    )
    .context("failed to write template remote README")?;
    fs::write(
        repo.join(TEMPLATE_REMOTE_METADATA_PATH),
        template_remote_metadata_to_string(encryption),
    )
    .context("failed to write template remote metadata")?;

    write_template_remote_templates(repo, &merged, encryption)?;
    Ok(Some(merged.len()))
}

fn merge_template_entries(
    remote_templates: &[TemplateEntry],
    local_templates: &[TemplateEntry],
) -> Vec<TemplateEntry> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for template in remote_templates.iter().chain(local_templates) {
        if seen.insert(template.id()) {
            merged.push(template.clone());
        }
    }
    merged
}

fn stage_and_commit_template_payload(
    out: &mut impl Write,
    repo: &Path,
    encryption: &TemplateRemoteEncryption,
) -> Result<bool> {
    let add = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "add".to_string(),
            "--".to_string(),
            TEMPLATE_REMOTE_METADATA_PATH.to_string(),
            TEMPLATE_REMOTE_README_PATH.to_string(),
            encryption.payload_path().to_string(),
        ],
    };
    if !run_template_git_step(out, repo, &add)? {
        return Ok(false);
    }
    let remove_opposite = GitCommandPlan {
        program: "git".to_string(),
        args: vec![
            "rm".to_string(),
            "--ignore-unmatch".to_string(),
            "--".to_string(),
            encryption.opposite_payload_path().to_string(),
        ],
    };
    if !run_template_git_step(out, repo, &remove_opposite)? {
        return Ok(false);
    }
    if git_has_staged_changes(repo)? {
        let commit = GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "commit".to_string(),
                "-m".to_string(),
                "publish aish templates".to_string(),
            ],
        };
        if !run_template_git_step(out, repo, &commit)? {
            return Ok(false);
        }
    } else {
        writeln!(out, "template remote step skipped: nothing to publish")?;
    }
    Ok(true)
}

fn select_pending_templates<'a>(
    templates: &'a [TemplateEntry],
    selector: &str,
) -> Result<Vec<&'a TemplateEntry>> {
    if selector == "all" {
        return Ok(templates.iter().collect());
    }
    let matches: Vec<_> = templates
        .iter()
        .filter(|template| template.id() == selector)
        .collect();
    if matches.len() > 1 {
        bail!("template selector matched multiple fetched templates: {selector}");
    }
    Ok(matches)
}

fn configured_template_remote<'a>(
    state: &'a AppState,
    name: &str,
) -> Option<&'a TemplateRemoteConfig> {
    state
        .template_sharing_config
        .remotes
        .iter()
        .find(|remote| remote.name == name)
}

fn ensure_template_remote_repo(
    state: &AppState,
    out: &mut impl Write,
    remote: &TemplateRemoteConfig,
) -> Result<Option<PathBuf>> {
    let repo = template_remote_repo_path(state, remote)?;
    fs::create_dir_all(&repo)
        .with_context(|| format!("failed to create template remote cache {}", repo.display()))?;
    let mut needs_init = !repo.join(".git").is_dir();
    if !needs_init {
        let origin_url = git_config_value(&repo, "remote.origin.url")?;
        if origin_url.as_deref() != Some(remote.remote.as_str()) {
            fs::remove_dir_all(&repo).with_context(|| {
                format!("failed to reset template remote cache {}", repo.display())
            })?;
            fs::create_dir_all(&repo).with_context(|| {
                format!("failed to create template remote cache {}", repo.display())
            })?;
            writeln!(out, "template remote cache reset: {}", remote.name)?;
            needs_init = true;
        }
    }
    if needs_init {
        let init = GitCommandPlan {
            program: "git".to_string(),
            args: vec!["init".to_string()],
        };
        if !run_template_git_step(out, &repo, &init)? {
            return Ok(None);
        }
        let add_remote = GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "remote".to_string(),
                "add".to_string(),
                "origin".to_string(),
                remote.remote.clone(),
            ],
        };
        if !run_template_git_step(out, &repo, &add_remote)? {
            return Ok(None);
        }
    }
    if !ensure_template_git_identity(out, &repo)? {
        return Ok(None);
    }
    Ok(Some(repo))
}

fn template_remote_repo_path(state: &AppState, remote: &TemplateRemoteConfig) -> Result<PathBuf> {
    if !valid_template_remote_name(&remote.name) {
        bail!("invalid template remote name in config: {}", remote.name);
    }
    let Some(config_path) = &state.config_path else {
        bail!("config path is not configured");
    };
    let Some(root) = config_path.parent() else {
        bail!("config path has no parent");
    };
    Ok(root
        .join("cache/template-remotes")
        .join(&remote.name)
        .join("repo"))
}

fn clear_template_remote_cache(state: &AppState, out: &mut impl Write, name: &str) -> Result<()> {
    let config = TemplateRemoteConfig {
        name: name.to_string(),
        remote: String::new(),
    };
    let repo = template_remote_repo_path(state, &config)?;
    match fs::remove_dir_all(&repo) {
        Ok(()) => {
            writeln!(out, "template remote cache reset: {name}")?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to reset template remote cache {}", repo.display())
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct TemplateRemoteSnapshot {
    branch: Option<String>,
}

impl TemplateRemoteSnapshot {
    fn fetched_ref(&self, branch: &str) -> String {
        format!("refs/remotes/origin/{branch}")
    }
}

#[derive(Debug, Clone)]
struct TemplateRemoteRefs {
    head_branch: Option<String>,
    branches: Vec<String>,
}

fn prepare_template_remote_snapshot(
    out: &mut impl Write,
    repo: &Path,
) -> Result<Option<TemplateRemoteSnapshot>> {
    let refs = template_remote_refs(repo)?;
    let branch = select_template_remote_branch(&refs);
    if let Some(branch) = &branch
        && !fetch_template_remote_branch(out, repo, branch)?
    {
        return Ok(None);
    }
    Ok(Some(TemplateRemoteSnapshot { branch }))
}

fn fetch_template_remote_branch(out: &mut impl Write, repo: &Path, branch: &str) -> Result<bool> {
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let fetch = GitCommandPlan {
        program: "git".to_string(),
        args: vec!["fetch".to_string(), "origin".to_string(), refspec],
    };
    run_template_git_step(out, repo, &fetch)
}

fn checkout_template_publish_branch(
    out: &mut impl Write,
    repo: &Path,
    snapshot: &TemplateRemoteSnapshot,
) -> Result<bool> {
    let branch = snapshot
        .branch
        .as_deref()
        .unwrap_or(DEFAULT_TEMPLATE_REMOTE_BRANCH);
    let mut args = vec!["checkout".to_string(), "-B".to_string(), branch.to_string()];
    if snapshot.branch.is_some() {
        args.push(snapshot.fetched_ref(branch));
    }
    let checkout = GitCommandPlan {
        program: "git".to_string(),
        args,
    };
    run_template_git_step(out, repo, &checkout)
}

fn template_remote_refs(repo: &Path) -> Result<TemplateRemoteRefs> {
    let result = run_git_command(
        repo,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "ls-remote".to_string(),
                "--symref".to_string(),
                "origin".to_string(),
                "HEAD".to_string(),
                "refs/heads/*".to_string(),
            ],
        },
    )?;
    if !result.success {
        return Ok(TemplateRemoteRefs {
            head_branch: None,
            branches: Vec::new(),
        });
    }
    Ok(parse_template_remote_refs(&result.stdout))
}

fn parse_template_remote_refs(raw: &str) -> TemplateRemoteRefs {
    let mut head_branch = None;
    let mut branches = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("ref: refs/heads/")
            && let Some((branch, target)) = rest.split_once('\t')
            && target == "HEAD"
            && valid_git_branch_name(branch)
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
        if valid_git_branch_name(branch) {
            branches.push(branch.to_string());
        }
    }
    branches.sort();
    branches.dedup();
    TemplateRemoteRefs {
        head_branch,
        branches,
    }
}

fn select_template_remote_branch(refs: &TemplateRemoteRefs) -> Option<String> {
    if let Some(branch) = &refs.head_branch
        && refs.branches.iter().any(|candidate| candidate == branch)
    {
        return Some(branch.clone());
    }
    if refs
        .branches
        .iter()
        .any(|branch| branch == DEFAULT_TEMPLATE_REMOTE_BRANCH)
    {
        return Some(DEFAULT_TEMPLATE_REMOTE_BRANCH.to_string());
    }
    if refs
        .branches
        .iter()
        .any(|branch| branch == FALLBACK_TEMPLATE_REMOTE_BRANCH)
    {
        return Some(FALLBACK_TEMPLATE_REMOTE_BRANCH.to_string());
    }
    if refs.branches.len() == 1 {
        return refs.branches.first().cloned();
    }
    None
}

fn validate_template_remote_worktree(out: &mut impl Write, repo: &Path) -> Result<bool> {
    if repo
        .join(crate::sync::sync_repository_metadata_path())
        .exists()
    {
        writeln!(
            out,
            "template remote appears to be a private Aish sync repository; use a separate template remote"
        )?;
        return Ok(false);
    }
    let metadata_path = repo.join(TEMPLATE_REMOTE_METADATA_PATH);
    if metadata_path.exists() {
        let raw = fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        if !template_remote_metadata_is_valid(&raw) {
            writeln!(
                out,
                "template remote metadata is invalid; refusing to use this repository"
            )?;
            writeln!(
                out,
                "Fix .aish-template-remote.toml, or use a separate empty template remote, then retry #template fetch or #template publish."
            )?;
            return Ok(false);
        }
        return Ok(true);
    }
    if repo.join(TEMPLATE_REMOTE_TEMPLATES_PATH).exists()
        || repo.join(TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH).exists()
    {
        writeln!(
            out,
            "warning: template remote metadata is missing; using existing template payload"
        )?;
    } else if !tracked_files(repo)?.is_empty() {
        writeln!(
            out,
            "warning: template remote metadata is missing; publishing will add Aish template files without deleting existing files"
        )?;
    }
    Ok(true)
}

fn template_remote_metadata_is_valid(raw: &str) -> bool {
    parse_template_remote_metadata(raw).is_ok()
}

fn tracked_files(repo: &Path) -> Result<Vec<String>> {
    let result = run_git_command(
        repo,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["ls-files".to_string()],
        },
    )?;
    if !result.success {
        return Ok(Vec::new());
    }
    Ok(result
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn ensure_template_git_identity(out: &mut impl Write, repo: &Path) -> Result<bool> {
    for (key, value) in [
        ("user.name", DEFAULT_TEMPLATE_GIT_USER_NAME),
        ("user.email", DEFAULT_TEMPLATE_GIT_USER_EMAIL),
        ("commit.gpgsign", "false"),
    ] {
        let current = git_config_value(repo, key)?;
        if current.as_deref() == Some(value) {
            continue;
        }
        if current.is_none() || key == "commit.gpgsign" {
            let command = GitCommandPlan {
                program: "git".to_string(),
                args: vec![
                    "config".to_string(),
                    "--local".to_string(),
                    key.to_string(),
                    value.to_string(),
                ],
            };
            if !run_template_git_step(out, repo, &command)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn git_config_value(repo: &Path, key: &str) -> Result<Option<String>> {
    let result = run_git_command(
        repo,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["config".to_string(), "--get".to_string(), key.to_string()],
        },
    )?;
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

fn git_has_staged_changes(repo: &Path) -> Result<bool> {
    let result = run_git_command(
        repo,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "diff".to_string(),
                "--cached".to_string(),
                "--quiet".to_string(),
                "--exit-code".to_string(),
            ],
        },
    )?;
    match result.exit_code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => bail!(
            "failed to inspect staged template remote changes: {}",
            result.combined_output()
        ),
    }
}

fn parse_publish_args(args: &str, out: &mut impl Write) -> Result<Option<TemplatePublishOptions>> {
    let Some((name, rest)) = next_word(args) else {
        return Ok(None);
    };
    if !valid_template_remote_name(name) {
        writeln!(out, "usage: #template publish <name> [--encrypt <key>]")?;
        return Ok(None);
    }
    let encryption = match next_word(rest) {
        None => TemplateRemoteEncryption::Plain,
        Some(("--encrypt" | "encrypt" | "encrypted", selector)) => {
            let Some(selector) = single_word(selector) else {
                writeln!(
                    out,
                    "usage: #template publish <name> --encrypt <key-fingerprint|unique-email>"
                )?;
                return Ok(None);
            };
            let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), selector)?;
            TemplateRemoteEncryption::Gpg {
                recipient_fingerprint: fingerprint,
            }
        }
        _ => {
            writeln!(out, "usage: #template publish <name> [--encrypt <key>]")?;
            return Ok(None);
        }
    };
    Ok(Some(TemplatePublishOptions {
        name: name.to_string(),
        encryption,
    }))
}

fn template_remote_metadata_to_string(encryption: &TemplateRemoteEncryption) -> String {
    let mut raw = String::new();
    raw.push_str("version = 1\n");
    raw.push_str("kind = \"aish-template-remote\"\n");
    raw.push_str("content = \"templates-only\"\n");
    raw.push_str(&format!(
        "encryption = \"{}\"\n",
        encryption.metadata_value()
    ));
    if let TemplateRemoteEncryption::Gpg {
        recipient_fingerprint,
    } = encryption
    {
        raw.push_str(&format!(
            "recipient_fingerprint = \"{}\"\n",
            recipient_fingerprint
        ));
    }
    raw
}

fn read_template_remote_metadata(repo: &Path) -> Result<Option<TemplateRemoteMetadata>> {
    let path = repo.join(TEMPLATE_REMOTE_METADATA_PATH);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    parse_template_remote_metadata(&raw).map(Some)
}

fn parse_template_remote_metadata(raw: &str) -> Result<TemplateRemoteMetadata> {
    let mut metadata: TemplateRemoteMetadata =
        toml::from_str(raw).context("invalid template remote metadata")?;
    metadata.kind = metadata.kind.trim().to_string();
    metadata.content = metadata.content.trim().to_string();
    metadata.encryption = metadata.encryption.trim().to_string();
    metadata.recipient_fingerprint = metadata.recipient_fingerprint.trim().to_ascii_uppercase();
    if metadata.version != 1 {
        bail!(
            "unsupported template remote metadata version: {}",
            metadata.version
        );
    }
    if metadata.kind != "aish-template-remote" || metadata.content != "templates-only" {
        bail!("template remote metadata does not describe an Aish template remote");
    }
    if metadata.encryption.is_empty() {
        metadata.encryption = "none".to_string();
    }
    if metadata.encryption != "none" && metadata.encryption != "gpg" {
        bail!(
            "unsupported template remote encryption mode: {}",
            metadata.encryption
        );
    }
    if metadata.encryption == "gpg" && metadata.recipient_fingerprint.is_empty() {
        bail!("encrypted template remote metadata is missing recipient_fingerprint");
    }
    Ok(metadata)
}

fn template_remote_payload_encryption(repo: &Path) -> Result<TemplateRemoteEncryption> {
    let plain_exists = repo.join(TEMPLATE_REMOTE_TEMPLATES_PATH).exists();
    let encrypted_exists = repo.join(TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH).exists();
    if let Some(metadata) = read_template_remote_metadata(repo)? {
        return if metadata.encryption == "gpg" {
            Ok(TemplateRemoteEncryption::Gpg {
                recipient_fingerprint: metadata.recipient_fingerprint,
            })
        } else {
            Ok(TemplateRemoteEncryption::Plain)
        };
    }
    if plain_exists && encrypted_exists {
        bail!("template remote has both plaintext and encrypted payloads without metadata");
    }
    if encrypted_exists {
        Ok(TemplateRemoteEncryption::Gpg {
            recipient_fingerprint: String::new(),
        })
    } else {
        Ok(TemplateRemoteEncryption::Plain)
    }
}

fn write_template_remote_templates(
    repo: &Path,
    templates: &[TemplateEntry],
    encryption: &TemplateRemoteEncryption,
) -> Result<()> {
    match encryption {
        TemplateRemoteEncryption::Plain => {
            rewrite_jsonl(&repo.join(TEMPLATE_REMOTE_TEMPLATES_PATH), templates)?;
            remove_file_if_present(&repo.join(TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH))?;
        }
        TemplateRemoteEncryption::Gpg {
            recipient_fingerprint,
        } => {
            let bytes = template_jsonl_bytes(templates)?;
            atomic_gpg_encrypt_bytes(
                gpg_program(),
                recipient_fingerprint,
                repo.join(TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH),
                &bytes,
            )?;
            remove_file_if_present(&repo.join(TEMPLATE_REMOTE_TEMPLATES_PATH))?;
        }
    }
    Ok(())
}

fn template_jsonl_bytes(templates: &[TemplateEntry]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for template in templates {
        serde_json::to_writer(&mut bytes, template)
            .context("failed to serialize template remote JSONL")?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn load_pending_templates_for_user(
    out: &mut impl Write,
    repo: &Path,
) -> Result<Option<JsonlLoad<TemplateEntry>>> {
    match load_pending_templates(repo) {
        Ok(loaded) => Ok(Some(loaded)),
        Err(err) => {
            writeln!(out, "template remote templates could not be loaded")?;
            writeln!(out, "{err:#}")?;
            writeln!(
                out,
                "If this is an encrypted template remote, import the matching private key locally and retry #template fetch, #template analyze, or #template import."
            )?;
            Ok(None)
        }
    }
}

fn load_pending_templates(repo: &Path) -> Result<JsonlLoad<TemplateEntry>> {
    match template_remote_payload_encryption(repo)? {
        TemplateRemoteEncryption::Plain => {
            load_templates(&repo.join(TEMPLATE_REMOTE_TEMPLATES_PATH))
        }
        TemplateRemoteEncryption::Gpg { .. } => {
            let path = repo.join(TEMPLATE_REMOTE_ENCRYPTED_TEMPLATES_PATH);
            if !path.exists() {
                return Ok(JsonlLoad {
                    items: Vec::new(),
                    errors: Vec::new(),
                });
            }
            let bytes = gpg_decrypt_file(gpg_program(), &path)
                .with_context(|| format!("failed to decrypt {}", path.display()))?;
            load_templates_from_bytes(&path, &bytes)
        }
    }
}

fn load_templates_from_bytes(path: &Path, bytes: &[u8]) -> Result<JsonlLoad<TemplateEntry>> {
    let raw = std::str::from_utf8(bytes)
        .with_context(|| format!("template JSONL is not valid UTF-8: {}", path.display()))?;
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TemplateEntry>(line) {
            Ok(entry) => items.push(entry),
            Err(err) => errors.push(JsonlLineError {
                path: path.to_path_buf(),
                line: index + 1,
                message: err.to_string(),
            }),
        }
    }
    Ok(JsonlLoad { items, errors })
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn run_template_git_step(
    out: &mut impl Write,
    repo: &Path,
    command: &GitCommandPlan,
) -> Result<bool> {
    let result = run_git_command(repo, command)?;
    if result.success {
        writeln!(
            out,
            "template remote step ok: {}",
            describe_git_command(command)
        )?;
        return Ok(true);
    }
    write_failed_template_step(out, command, &result.combined_output())?;
    Ok(false)
}

enum TemplatePushResult {
    Pushed,
    RemoteChanged,
    Failed,
}

fn template_push_plan() -> GitCommandPlan {
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

fn run_template_push_step(
    out: &mut impl Write,
    repo: &Path,
    command: &GitCommandPlan,
) -> Result<TemplatePushResult> {
    let result = run_git_command(repo, command)?;
    if result.success {
        writeln!(
            out,
            "template remote step ok: {}",
            describe_git_command(command)
        )?;
        return Ok(TemplatePushResult::Pushed);
    }
    let detail = result.combined_output();
    if git_output_suggests_remote_changed(&detail) {
        writeln!(
            out,
            "template remote push needs remote updates; refetching and publishing merged templates"
        )?;
        return Ok(TemplatePushResult::RemoteChanged);
    }
    write_failed_template_step(out, command, &detail)?;
    Ok(TemplatePushResult::Failed)
}

fn write_failed_template_step(
    out: &mut impl Write,
    command: &GitCommandPlan,
    detail: &str,
) -> Result<()> {
    writeln!(
        out,
        "template remote failed: {}",
        describe_git_command(command)
    )?;
    let detail = detail.trim();
    if !detail.is_empty() {
        writeln!(out, "{detail}")?;
    }
    Ok(())
}

fn git_output_suggests_remote_changed(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("stale info")
}

fn update_template_sharing_config(
    state: &mut AppState,
    update: impl FnOnce(&mut config::Config),
) -> Result<()> {
    let Some(path) = &state.config_path else {
        anyhow::bail!("config path is not configured; template sharing config not saved");
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
    state.template_sharing_config = config.template_sharing;
    state.append_event(EventLevel::Info, "template sharing config changed")?;
    Ok(())
}

fn write_template_load_errors(out: &mut impl Write, count: usize) -> Result<()> {
    if count > 0 {
        writeln!(out, "warning: skipped {count} invalid template line(s)")?;
    }
    Ok(())
}

fn write_template_usage(out: &mut impl Write) -> Result<()> {
    writeln!(out, "{}", template_usage())?;
    Ok(())
}

fn next_word(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let split_at = input.find(char::is_whitespace).unwrap_or(input.len());
    let (word, rest) = input.split_at(split_at);
    Some((word, rest.trim_start()))
}

fn single_word(input: &str) -> Option<&str> {
    let (word, rest) = next_word(input)?;
    rest.trim().is_empty().then_some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_remote_refs_drop_invalid_branch_names_before_fetching() {
        let refs = parse_template_remote_refs(
            "ref: refs/heads/main\tHEAD\n\
             1111111111111111111111111111111111111111\trefs/heads/main\n\
             2222222222222222222222222222222222222222\trefs/heads/team/templates\n\
             3333333333333333333333333333333333333333\trefs/heads/--upload-pack=/tmp/hook\n\
             4444444444444444444444444444444444444444\trefs/heads/team//templates\n\
             5555555555555555555555555555555555555555\trefs/heads/team templates\n",
        );

        assert_eq!(refs.head_branch, Some("main".to_string()));
        assert_eq!(
            refs.branches,
            vec!["main".to_string(), "team/templates".to_string()]
        );
    }
}
