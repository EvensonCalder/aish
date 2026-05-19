use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::ai::read_api_key_from_env;
use crate::config::{
    self, AiConfig, EncryptionConfig, EncryptionStartupUnlockMode, write_private_file,
};
use crate::encryption::{
    atomic_gpg_encrypt_bytes, encrypted_path, encryption_git_history_warning,
    enter_gpg_terminal_passthrough, existing_jsonl_bytes, gpg_decrypt_file, gpg_program,
    migrate_plaintext_jsonl_to_gpg, reencrypt_gpg_jsonl, resolve_gpg_key_fingerprint,
};
use crate::log::EventLevel;
use crate::sync::GitCommandPlan;

use super::AppState;
use super::sync_commands::{GitStepResult, run_git_command};

pub(super) fn configured_encryption_key(config: &EncryptionConfig) -> &str {
    let fingerprint = config.key_fingerprint.trim();
    if !fingerprint.is_empty() {
        fingerprint
    } else {
        config.recipient.trim()
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct StoredApiKey {
    pub(super) env_key: String,
    pub(super) value: String,
}

pub(super) fn parse_key_command(args: &str) -> Option<&str> {
    let mut parts = args.split_whitespace();
    let command = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(command)
}

pub(super) fn set_stored_key(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(path) = &state.secret_key_path else {
        writeln!(out, "key storage is not configured; no key stored")?;
        return Ok(());
    };
    let key = configured_encryption_key(&state.encryption_config);
    if key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }
    let value = match read_api_key_from_env(&state.ai_config.env_key) {
        Ok(value) => value,
        Err(err) => {
            writeln!(out, "{err}")?;
            return Ok(());
        }
    };
    let record = StoredApiKey {
        env_key: state.ai_config.env_key.clone(),
        value,
    };
    let plaintext =
        serde_json::to_vec(&record).context("failed to serialize encrypted API key record")?;
    atomic_gpg_encrypt_bytes(gpg_program(), key, path, &plaintext)?;
    state.append_event(EventLevel::Info, "stored key encrypted")?;
    writeln!(out, "stored key encrypted")?;
    Ok(())
}

pub(super) fn clear_stored_key(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let Some(path) = &state.secret_key_path else {
        writeln!(out, "key storage is not configured; no key removed")?;
        return Ok(());
    };

    match fs::remove_file(path) {
        Ok(()) => {
            state.append_event(EventLevel::Info, "stored key cleared")?;
            writeln!(out, "stored key cleared")?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            writeln!(out, "no stored key to clear")?;
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn load_stored_api_key(state: &mut AppState) -> Result<Option<String>> {
    state.run_unlock_passthrough(|state| {
        let Some(path) = &state.secret_key_path else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        let bytes = gpg_decrypt_file(gpg_program(), path)?;
        let record: StoredApiKey =
            serde_json::from_slice(&bytes).context("stored API key record is not valid JSON")?;
        if record.value.trim().is_empty() {
            anyhow::bail!("stored API key is empty");
        }
        Ok(Some(record.value))
    })
}

pub(super) fn ai_config_for_request(state: &mut AppState) -> Result<AiConfig> {
    let mut config = state.ai_config.clone();
    config.api_key_override = None;
    if read_api_key_from_env(&config.env_key).is_ok() {
        return Ok(config);
    }
    if let Some(api_key) = load_stored_api_key(state)? {
        config.api_key_override = Some(api_key);
    }
    Ok(config)
}

pub(super) fn update_encryption_config(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let parts: Vec<_> = args.split_whitespace().collect();
    match parts.as_slice() {
        ["unlock-mode"] => {
            writeln!(
                out,
                "encryption.startup_unlock={}",
                state.encryption_config.startup_unlock.as_str()
            )?;
            Ok(())
        }
        ["unlock-mode", mode] => set_startup_unlock_mode(state, out, mode),
        _ if state.encrypted_storage_is_locked() => writeln!(
            out,
            "encrypted storage is still unlocking; run #unlock before changing encryption"
        )
        .map_err(Into::into),
        ["on"] => enable_encryption(state, out, None),
        ["on", key_selector] => enable_encryption(state, out, Some(key_selector)),
        ["rotate", key_selector] => rotate_encryption_key(state, out, Some(key_selector)),
        ["rewrite-history", "plan"] => plan_encryption_history_rewrite(state, out),
        ["rewrite-history", "run", key_selector, "--confirm-rewrite-history"] => {
            run_encryption_history_rewrite(state, out, key_selector)
        }
        ["off"] => disable_encryption(state, out),
        _ => writeln!(
            out,
            "usage: #encrypt on [key-fingerprint|unique-email] | #encrypt rotate <key-fingerprint|unique-email> | #encrypt unlock-mode lazy|prompt | #encrypt rewrite-history plan | #encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history | #encrypt off"
        )
        .map_err(Into::into),
    }
}

fn set_startup_unlock_mode(state: &mut AppState, out: &mut impl Write, mode: &str) -> Result<()> {
    let startup_unlock = match mode {
        "lazy" => EncryptionStartupUnlockMode::Lazy,
        "prompt" => EncryptionStartupUnlockMode::Prompt,
        _ => {
            writeln!(out, "usage: #encrypt unlock-mode lazy|prompt")?;
            return Ok(());
        }
    };
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }
    set_encryption_config(state, |config| {
        config.encryption.startup_unlock = startup_unlock;
    })?;
    writeln!(out, "encryption.startup_unlock={}", startup_unlock.as_str())?;
    Ok(())
}

fn enable_encryption(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector_arg: Option<&str>,
) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }
    let selector = encryption_key_selector(state, key_selector_arg);
    if selector.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), &selector)?;
    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    state.flush_encrypted_writes()?;
    let (encrypted_cache, summary) = state.run_unlock_passthrough(|state| {
        let managed = ManagedStorageSnapshot::capture(state)?;
        let stored_key = if current_key != fingerprint {
            StoredKeySnapshot::capture(state)?
        } else {
            None
        };
        let encrypted_cache = managed.encrypted_writer_cache();
        let summary = run_with_storage_rollback(&managed, stored_key.as_ref(), || {
            let mut summary = managed.rewrite_for_encryption_key(&current_key, &fingerprint)?;
            if let Some(stored_key) = stored_key.as_ref()
                && stored_key.reencrypt_if_needed(&current_key, &fingerprint)?
            {
                summary.stored_key_reencrypted += 1;
            }
            set_encryption_config(state, |config| {
                config.encryption.enabled = true;
                config.encryption.key_fingerprint = fingerprint.clone();
                config.encryption.recipient.clear();
            })?;
            Ok(summary)
        })?;
        Ok((encrypted_cache, summary))
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "{}", encryption_git_history_warning())?;
    writeln!(out, "encryption=on")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &summary)?;
    Ok(())
}

fn encryption_key_selector(state: &AppState, key_selector_arg: Option<&str>) -> String {
    key_selector_arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| configured_encryption_key(&state.encryption_config))
        .to_string()
}

fn rotate_encryption_key(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector_arg: Option<&str>,
) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }
    let Some(selector) = key_selector_arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        writeln!(out, "usage: #encrypt rotate <key-fingerprint|unique-email>")?;
        return Ok(());
    };

    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), selector)?;
    state.flush_encrypted_writes()?;
    let (encrypted_cache, summary) = state.run_unlock_passthrough(|state| {
        let managed = ManagedStorageSnapshot::capture(state)?;
        let stored_key = if current_key != fingerprint {
            StoredKeySnapshot::capture(state)?
        } else {
            None
        };
        let encrypted_cache = managed.encrypted_writer_cache();
        let summary = run_with_storage_rollback(&managed, stored_key.as_ref(), || {
            let mut summary = managed.rewrite_for_encryption_key(&current_key, &fingerprint)?;
            if let Some(stored_key) = stored_key.as_ref()
                && stored_key.reencrypt_if_needed(&current_key, &fingerprint)?
            {
                summary.stored_key_reencrypted += 1;
            }
            set_encryption_config(state, |config| {
                config.encryption.enabled = true;
                config.encryption.key_fingerprint = fingerprint.clone();
                config.encryption.recipient.clear();
            })?;
            Ok(summary)
        })?;
        Ok((encrypted_cache, summary))
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "encryption=on")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &summary)?;
    Ok(())
}

fn disable_encryption(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    if state.config_path.is_none() {
        writeln!(out, "config path is not configured; #encrypt not saved")?;
        return Ok(());
    }

    state.flush_encrypted_writes()?;
    let mut restart_cache = None;
    let result = state.run_unlock_passthrough(|state| {
        let managed = ManagedStorageSnapshot::capture(state)?;
        restart_cache = Some(managed.encrypted_writer_cache());
        state.stop_encrypted_writer();
        run_with_storage_rollback(&managed, None, || {
            managed.migrate_to_plaintext()?;
            set_encryption_config(state, |config| {
                config.encryption.enabled = false;
            })
        })
    });
    if let Err(error) = result {
        if state.encryption_config.enabled
            && let Some(cache) = restart_cache
        {
            state.start_encrypted_writer_with_cache(cache);
        }
        return Err(error);
    }
    writeln!(out, "encryption=off")?;
    writeln!(
        out,
        "plaintext history and templates will be written from now on"
    )?;
    Ok(())
}

fn set_encryption_config(
    state: &mut AppState,
    update: impl FnOnce(&mut config::Config),
) -> Result<()> {
    let Some(path) = &state.config_path else {
        anyhow::bail!("config path is not configured; #encrypt not saved");
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
    state.encryption_config = config.encryption;
    state.append_event(EventLevel::Info, "encryption config changed")?;
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct EncryptionRewriteSummary {
    plaintext_encrypted: usize,
    reencrypted: usize,
    already_encrypted: usize,
    missing: usize,
    stored_key_reencrypted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedStorageFileSnapshot {
    Missing {
        plaintext_path: PathBuf,
        encrypted_path: PathBuf,
    },
    Plaintext {
        plaintext_path: PathBuf,
        encrypted_path: PathBuf,
        plaintext_bytes: Vec<u8>,
    },
    Encrypted {
        plaintext_path: PathBuf,
        encrypted_path: PathBuf,
        encrypted_bytes: Vec<u8>,
        plaintext_bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedStorageSnapshot {
    files: Vec<ManagedStorageFileSnapshot>,
}

impl ManagedStorageSnapshot {
    fn capture(state: &AppState) -> Result<Self> {
        let program = gpg_program();
        let mut files = Vec::new();
        for plaintext_path in encrypted_storage_paths(state) {
            let encrypted_path = encrypted_path(&plaintext_path);
            match (plaintext_path.exists(), encrypted_path.exists()) {
                (true, true) => {
                    anyhow::bail!(
                        "both plaintext and encrypted storage exist for {}; resolve this before changing encryption keys",
                        plaintext_path.display()
                    );
                }
                (true, false) => {
                    let plaintext_bytes = fs::read(&plaintext_path).with_context(|| {
                        format!("failed to read plaintext file {}", plaintext_path.display())
                    })?;
                    files.push(ManagedStorageFileSnapshot::Plaintext {
                        plaintext_path,
                        encrypted_path,
                        plaintext_bytes,
                    });
                }
                (false, true) => {
                    let encrypted_bytes = fs::read(&encrypted_path).with_context(|| {
                        format!("failed to read encrypted file {}", encrypted_path.display())
                    })?;
                    let plaintext_bytes = gpg_decrypt_file(&program, &encrypted_path)
                        .with_context(|| {
                            format!(
                                "failed to decrypt encrypted JSONL file {}",
                                encrypted_path.display()
                            )
                        })?;
                    files.push(ManagedStorageFileSnapshot::Encrypted {
                        plaintext_path,
                        encrypted_path,
                        encrypted_bytes,
                        plaintext_bytes,
                    });
                }
                (false, false) => {
                    files.push(ManagedStorageFileSnapshot::Missing {
                        plaintext_path,
                        encrypted_path,
                    });
                }
            }
        }
        Ok(Self { files })
    }

    fn encrypted_writer_cache(&self) -> HashMap<PathBuf, Vec<u8>> {
        self.files
            .iter()
            .map(|file| match file {
                ManagedStorageFileSnapshot::Missing { plaintext_path, .. } => {
                    (plaintext_path.clone(), Vec::new())
                }
                ManagedStorageFileSnapshot::Plaintext {
                    plaintext_path,
                    plaintext_bytes,
                    ..
                }
                | ManagedStorageFileSnapshot::Encrypted {
                    plaintext_path,
                    plaintext_bytes,
                    ..
                } => (plaintext_path.clone(), plaintext_bytes.clone()),
            })
            .collect()
    }

    fn rewrite_for_encryption_key(
        &self,
        old_key: &str,
        new_key: &str,
    ) -> Result<EncryptionRewriteSummary> {
        let program = gpg_program();
        let mut summary = EncryptionRewriteSummary::default();
        for file in &self.files {
            match file {
                ManagedStorageFileSnapshot::Missing { .. } => {
                    summary.missing += 1;
                }
                ManagedStorageFileSnapshot::Plaintext {
                    plaintext_path,
                    encrypted_path,
                    plaintext_bytes,
                } => {
                    atomic_gpg_encrypt_bytes(
                        program.clone(),
                        new_key,
                        encrypted_path,
                        plaintext_bytes,
                    )?;
                    remove_file_if_present(plaintext_path)?;
                    summary.plaintext_encrypted += 1;
                }
                ManagedStorageFileSnapshot::Encrypted {
                    encrypted_path,
                    plaintext_bytes,
                    ..
                } if old_key != new_key => {
                    atomic_gpg_encrypt_bytes(
                        program.clone(),
                        new_key,
                        encrypted_path,
                        plaintext_bytes,
                    )?;
                    summary.reencrypted += 1;
                }
                ManagedStorageFileSnapshot::Encrypted { .. } => {
                    summary.already_encrypted += 1;
                }
            }
        }
        Ok(summary)
    }

    fn migrate_to_plaintext(&self) -> Result<()> {
        for file in &self.files {
            match file {
                ManagedStorageFileSnapshot::Missing { .. } => {}
                ManagedStorageFileSnapshot::Plaintext { .. } => {}
                ManagedStorageFileSnapshot::Encrypted {
                    plaintext_path,
                    encrypted_path,
                    plaintext_bytes,
                    ..
                } => {
                    write_private_file(plaintext_path, plaintext_bytes)?;
                    remove_file_if_present(encrypted_path)?;
                }
            }
        }
        Ok(())
    }

    fn restore(&self) -> Result<()> {
        for file in &self.files {
            match file {
                ManagedStorageFileSnapshot::Missing {
                    plaintext_path,
                    encrypted_path,
                } => {
                    remove_file_if_present(plaintext_path)?;
                    remove_file_if_present(encrypted_path)?;
                }
                ManagedStorageFileSnapshot::Plaintext {
                    plaintext_path,
                    encrypted_path,
                    plaintext_bytes,
                } => {
                    write_private_file(plaintext_path, plaintext_bytes)?;
                    remove_file_if_present(encrypted_path)?;
                }
                ManagedStorageFileSnapshot::Encrypted {
                    plaintext_path,
                    encrypted_path,
                    encrypted_bytes,
                    ..
                } => {
                    write_private_file(encrypted_path, encrypted_bytes)?;
                    remove_file_if_present(plaintext_path)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredKeySnapshot {
    Missing {
        path: PathBuf,
    },
    Encrypted {
        path: PathBuf,
        encrypted_bytes: Vec<u8>,
        plaintext_bytes: Vec<u8>,
    },
}

impl StoredKeySnapshot {
    fn capture(state: &AppState) -> Result<Option<Self>> {
        let Some(path) = &state.secret_key_path else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(Some(Self::Missing { path: path.clone() }));
        }
        let encrypted_bytes = fs::read(path)
            .with_context(|| format!("failed to read encrypted stored key {}", path.display()))?;
        let plaintext_bytes = gpg_decrypt_file(gpg_program(), path)
            .with_context(|| format!("failed to decrypt stored key {}", path.display()))?;
        Ok(Some(Self::Encrypted {
            path: path.clone(),
            encrypted_bytes,
            plaintext_bytes,
        }))
    }

    fn reencrypt_if_needed(&self, old_key: &str, new_key: &str) -> Result<bool> {
        if old_key == new_key {
            return Ok(false);
        }
        let Self::Encrypted {
            path,
            plaintext_bytes,
            ..
        } = self
        else {
            return Ok(false);
        };
        atomic_gpg_encrypt_bytes(gpg_program(), new_key, path, plaintext_bytes)?;
        Ok(true)
    }

    fn restore(&self) -> Result<()> {
        match self {
            Self::Missing { path } => remove_file_if_present(path),
            Self::Encrypted {
                path,
                encrypted_bytes,
                ..
            } => write_private_file(path, encrypted_bytes),
        }
    }
}

fn run_with_storage_rollback<T>(
    managed: &ManagedStorageSnapshot,
    stored_key: Option<&StoredKeySnapshot>,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    match operation() {
        Ok(value) => Ok(value),
        Err(error) => {
            let restore_result = managed.restore().and_then(|()| {
                if let Some(stored_key) = stored_key {
                    stored_key.restore()?;
                }
                Ok(())
            });
            if let Err(restore_error) = restore_result {
                anyhow::bail!(
                    "{error}; additionally failed to restore encryption storage after the error: {restore_error}"
                );
            }
            Err(error)
        }
    }
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove file {}", path.display())),
    }
}

fn write_encryption_rewrite_summary(
    out: &mut impl Write,
    summary: &EncryptionRewriteSummary,
) -> Result<()> {
    writeln!(
        out,
        "encrypted_plaintext_files={}",
        summary.plaintext_encrypted
    )?;
    writeln!(out, "reencrypted_files={}", summary.reencrypted)?;
    writeln!(out, "already_encrypted_files={}", summary.already_encrypted)?;
    writeln!(
        out,
        "stored_key_reencrypted={}",
        summary.stored_key_reencrypted
    )?;
    Ok(())
}

fn plan_encryption_history_rewrite(state: &AppState, out: &mut impl Write) -> Result<()> {
    let Some(config_path) = &state.config_path else {
        writeln!(
            out,
            "config path is not configured; cannot plan history rewrite"
        )?;
        return Ok(());
    };
    let Some(root) = config_path.parent() else {
        writeln!(
            out,
            "config path has no parent; cannot plan history rewrite"
        )?;
        return Ok(());
    };
    let key = configured_encryption_key(&state.encryption_config);
    if key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    writeln!(out, "history rewrite plan")?;
    writeln!(out, "repo={}", root.display())?;
    writeln!(out, "target_key_fingerprint={key}")?;
    writeln!(
        out,
        "risk=rewrites commit ids and requires a force push for any shared remote"
    )?;
    writeln!(
        out,
        "scope=current branch; managed history, draft, note, AI, and template storage paths"
    )?;
    writeln!(
        out,
        "next=#encrypt rewrite-history run <key-fingerprint> --confirm-rewrite-history"
    )?;
    writeln!(
        out,
        "note=the run command must decrypt old encrypted blobs with the old private key, then encrypt each rewritten blob for the target key"
    )?;
    Ok(())
}

fn run_encryption_history_rewrite(
    state: &mut AppState,
    out: &mut impl Write,
    key_selector: &str,
) -> Result<()> {
    let Some(config_path) = &state.config_path else {
        writeln!(out, "config path is not configured; cannot rewrite history")?;
        return Ok(());
    };
    let Some(root) = config_path.parent().map(Path::to_path_buf) else {
        writeln!(out, "config path has no parent; cannot rewrite history")?;
        return Ok(());
    };
    if !root.join(".git").is_dir() {
        writeln!(
            out,
            "git repository is not initialized; run #push before rewriting history"
        )?;
        return Ok(());
    }
    let current_key = configured_encryption_key(&state.encryption_config).to_string();
    if current_key.is_empty() {
        writeln!(
            out,
            "encryption key is not configured; run #encrypt on <key-fingerprint>"
        )?;
        return Ok(());
    }

    state.flush_encrypted_writes()?;
    let encrypted_cache =
        state.run_unlock_passthrough(|state| encrypted_writer_cache_from_storage(state))?;
    let clean = run_git_command(
        &root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["status".to_string(), "--porcelain".to_string()],
        },
    )?;
    if !clean.success || !clean.stdout.trim().is_empty() {
        writeln!(
            out,
            "history rewrite requires a clean git worktree; commit, stash, or discard changes first"
        )?;
        return Ok(());
    }

    let fingerprint = resolve_gpg_key_fingerprint(gpg_program(), key_selector)?;
    let stored_key = if current_key != fingerprint {
        state.run_unlock_passthrough(|state| StoredKeySnapshot::capture(state))?
    } else {
        None
    };
    let script_path = write_history_rewrite_script(&root, state)?;
    let backup_ref = format!(
        "aish/rewrite-backup/{}-{}",
        (state.clock)(),
        std::process::id()
    );
    let backup = run_git_command(
        &root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec!["branch".to_string(), backup_ref.clone(), "HEAD".to_string()],
        },
    )?;
    if !backup.success {
        let _ = fs::remove_file(&script_path);
        anyhow::bail!(
            "failed to create rewrite backup branch: {}",
            backup.combined_output()
        );
    }

    let filter_result = state.run_unlock_passthrough(|_| {
        run_git_filter_branch_reencrypt(&root, &script_path, &fingerprint)
    });
    let _ = fs::remove_file(&script_path);
    let filter_result = filter_result?;
    if !filter_result.success {
        anyhow::bail!(
            "git history rewrite failed: {}",
            filter_result.combined_output()
        );
    }

    let mut untracked = state.run_unlock_passthrough(|state| {
        rewrite_untracked_storage_for_encryption_key(state, &root, &current_key, &fingerprint)
    })?;
    if let Some(stored_key) = stored_key.as_ref()
        && stored_key.reencrypt_if_needed(&current_key, &fingerprint)?
    {
        untracked.stored_key_reencrypted += 1;
    }
    set_encryption_config(state, |config| {
        config.encryption.enabled = true;
        config.encryption.key_fingerprint = fingerprint.clone();
        config.encryption.recipient.clear();
    })?;
    state.start_encrypted_writer_with_cache(encrypted_cache);
    writeln!(out, "history rewrite completed")?;
    writeln!(out, "backup_branch={backup_ref}")?;
    writeln!(out, "encryption.key_fingerprint={fingerprint}")?;
    write_encryption_rewrite_summary(out, &untracked)?;
    writeln!(
        out,
        "next=verify the rewritten history, push with --force-with-lease if appropriate, then remove backup refs and expire reflogs only after an external backup"
    )?;
    Ok(())
}

pub(super) fn write_history_rewrite_script(root: &Path, state: &AppState) -> Result<PathBuf> {
    let script_dir = root.join("cache/runtime");
    config::create_private_dir_all(&script_dir).with_context(|| {
        format!(
            "failed to create rewrite script directory {}",
            script_dir.display()
        )
    })?;
    let script_path = script_dir.join("encrypt-rewrite-history.sh");
    let mut script = String::from(
        "#!/bin/sh\nset -eu\numask 077\ngpg_program=${AISH_REWRITE_GPG:-gpg}\nrecipient=${AISH_REWRITE_RECIPIENT:?}\ntmp_dir=$(mktemp -d \"${TMPDIR:-/tmp}/aish-rewrite.XXXXXX\")\ncleanup() {\n  rm -rf \"$tmp_dir\"\n}\ntrap cleanup EXIT HUP INT TERM\nreencrypt_file() {\n  plain=$1\n  enc=$plain.gpg\n  if [ -f \"$plain\" ] && [ -f \"$enc\" ]; then\n    printf '%s\\n' \"both plaintext and encrypted files exist: $plain\" >&2\n    exit 3\n  fi\n  if [ -f \"$plain\" ]; then\n    \"$gpg_program\" --batch --yes --no-tty --trust-model always --encrypt --recipient \"$recipient\" --output \"$enc.tmp\" \"$plain\"\n    mv \"$enc.tmp\" \"$enc\"\n    rm -f \"$plain\"\n  elif [ -f \"$enc\" ]; then\n    tmp=\"$tmp_dir/plain\"\n    rm -f \"$tmp\"\n    \"$gpg_program\" --yes --decrypt \"$enc\" > \"$tmp\"\n    \"$gpg_program\" --batch --yes --no-tty --trust-model always --encrypt --recipient \"$recipient\" --output \"$enc.tmp\" \"$tmp\"\n    rm -f \"$tmp\"\n    mv \"$enc.tmp\" \"$enc\"\n  fi\n}\n",
    );
    for relative in managed_relative_storage_paths(root, state)? {
        script.push_str("reencrypt_file ");
        script.push_str(&shell_single_quote(&relative));
        script.push('\n');
    }
    write_private_file(&script_path, script.as_bytes())
        .with_context(|| format!("failed to write rewrite script {}", script_path.display()))?;
    Ok(script_path)
}

fn run_git_filter_branch_reencrypt(
    root: &Path,
    script_path: &Path,
    fingerprint: &str,
) -> Result<GitStepResult> {
    let filter_command = format!(
        "sh {}",
        shell_single_quote(&script_path.display().to_string())
    );
    let terminal = enter_gpg_terminal_passthrough()?;
    let mut command = Command::new("git");
    command
        .args([
            "filter-branch",
            "-f",
            "--tree-filter",
            &filter_command,
            "--",
            "HEAD",
        ])
        .current_dir(root)
        .env("FILTER_BRANCH_SQUELCH_WARNING", "1")
        .env("AISH_REWRITE_GPG", gpg_program())
        .env("AISH_REWRITE_RECIPIENT", fingerprint);
    terminal.prepare_command(&mut command);
    let output = command
        .output()
        .context("failed to run git filter-branch")?;
    Ok(GitStepResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

fn rewrite_untracked_storage_for_encryption_key(
    state: &AppState,
    root: &Path,
    old_key: &str,
    new_key: &str,
) -> Result<EncryptionRewriteSummary> {
    let mut summary = EncryptionRewriteSummary::default();
    for path in encrypted_storage_paths(state) {
        let relative = match path.strip_prefix(root) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        if git_path_is_tracked(root, relative)? {
            continue;
        }
        let encrypted = encrypted_path(&path);
        match (path.exists(), encrypted.exists()) {
            (true, true) => {
                anyhow::bail!(
                    "both plaintext and encrypted storage exist for {}; resolve this before changing encryption keys",
                    path.display()
                );
            }
            (true, false) => {
                if migrate_plaintext_jsonl_to_gpg(gpg_program(), new_key, &path)? {
                    summary.plaintext_encrypted += 1;
                }
            }
            (false, true) if old_key != new_key => {
                if reencrypt_gpg_jsonl(gpg_program(), new_key, &path)? {
                    summary.reencrypted += 1;
                }
            }
            (false, true) => summary.already_encrypted += 1,
            (false, false) => summary.missing += 1,
        }
    }
    Ok(summary)
}

fn git_path_is_tracked(root: &Path, relative: &Path) -> Result<bool> {
    let result = run_git_command(
        root,
        &GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "ls-files".to_string(),
                "--error-unmatch".to_string(),
                "--".to_string(),
                relative.display().to_string(),
            ],
        },
    )?;
    Ok(result.success)
}

fn managed_relative_storage_paths(root: &Path, state: &AppState) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for path in encrypted_storage_paths(state) {
        let relative = path.strip_prefix(root).with_context(|| {
            format!(
                "managed storage path is outside git root: {}",
                path.display()
            )
        })?;
        paths.push(relative.display().to_string());
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn encrypted_storage_paths(state: &AppState) -> Vec<PathBuf> {
    [
        &state.regular_history_path,
        &state.ai_history_path,
        &state.draft_history_path,
        &state.notes_path,
        &state.template_store_path,
    ]
    .into_iter()
    .filter_map(|path| path.clone())
    .collect()
}

fn encrypted_writer_cache_from_storage(state: &AppState) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let program = gpg_program();
    let mut cache = HashMap::new();
    for path in encrypted_storage_paths(state) {
        let bytes = existing_jsonl_bytes(&program, &path)?;
        cache.insert(path, bytes);
    }
    Ok(cache)
}
