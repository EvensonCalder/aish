use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::{create_private_dir_all, set_private_file_permissions, write_private_file};
use crate::history::{JsonlLineError, JsonlLoad};

pub fn encryption_git_history_warning() -> &'static str {
    "Encryption is now enabled for future writes.\nAish will sync encrypted files from now on.\nGit history may still contain plaintext data or encrypted data written for an older key.\nAish will not rewrite git history automatically; history rewrite requires an explicit backup and old-key re-encryption flow."
}

pub fn gpg_program() -> String {
    std::env::var("AISH_GPG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gpg".to_string())
}

pub fn encrypted_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut encrypted = path.to_path_buf();
    let next_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!("{extension}.gpg"))
        .unwrap_or_else(|| "gpg".to_string());
    encrypted.set_extension(next_extension);
    encrypted
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpgPublicKey {
    pub fingerprint: String,
    pub user_ids: Vec<String>,
}

pub fn resolve_gpg_key_fingerprint(gpg_program: impl AsRef<str>, selector: &str) -> Result<String> {
    let selector = selector.trim();
    if selector.is_empty() {
        bail!("GPG key selector is empty; use a key fingerprint or a unique email/user ID");
    }

    let keys = list_matching_gpg_public_keys(gpg_program, selector)?;
    if keys.is_empty() {
        bail!("GPG key not found for selector: {selector}");
    }

    let normalized_selector = normalize_fingerprint(selector);
    let exact_matches: Vec<&GpgPublicKey> = keys
        .iter()
        .filter(|key| normalize_fingerprint(&key.fingerprint) == normalized_selector)
        .collect();
    if exact_matches.len() == 1 {
        return Ok(exact_matches[0].fingerprint.clone());
    }

    if keys.len() == 1 {
        return Ok(keys[0].fingerprint.clone());
    }

    let matches = keys
        .iter()
        .map(|key| key.fingerprint.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "GPG key selector is ambiguous; use a full fingerprint. Matching fingerprints: {matches}"
    );
}

pub fn list_matching_gpg_public_keys(
    gpg_program: impl AsRef<str>,
    selector: &str,
) -> Result<Vec<GpgPublicKey>> {
    let program = gpg_program.as_ref();
    let output = Command::new(program)
        .args([
            "--batch",
            "--with-colons",
            "--fingerprint",
            "--list-keys",
            selector,
        ])
        .output()
        .with_context(|| format!("failed to run GPG command: {program}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let summary = stderr
            .lines()
            .next()
            .unwrap_or("GPG key lookup failed")
            .trim();
        bail!("GPG key lookup failed: {summary}");
    }
    Ok(parse_gpg_public_keys(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_gpg_public_keys(output: &str) -> Vec<GpgPublicKey> {
    let mut keys = Vec::new();
    let mut current: Option<GpgPublicKey> = None;

    for line in output.lines() {
        let record_type = colon_field(line, 0);
        match record_type {
            "pub" => {
                push_key_if_complete(&mut keys, current.take());
                current = Some(GpgPublicKey {
                    fingerprint: String::new(),
                    user_ids: Vec::new(),
                });
            }
            "fpr" => {
                if let Some(key) = current.as_mut()
                    && key.fingerprint.is_empty()
                {
                    key.fingerprint = colon_field(line, 9).to_string();
                }
            }
            "uid" => {
                if let Some(key) = current.as_mut() {
                    let user_id = colon_field(line, 9);
                    if !user_id.is_empty() {
                        key.user_ids.push(user_id.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    push_key_if_complete(&mut keys, current);
    keys
}

fn push_key_if_complete(keys: &mut Vec<GpgPublicKey>, key: Option<GpgPublicKey>) {
    if let Some(key) = key
        && !key.fingerprint.is_empty()
    {
        keys.push(key);
    }
}

fn colon_field(line: &str, index: usize) -> &str {
    line.split(':').nth(index).unwrap_or("")
}

fn normalize_fingerprint(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpgEncryptPlan {
    pub program: String,
    pub args: Vec<String>,
}

pub fn gpg_encrypt_plan(
    gpg_program: impl Into<String>,
    recipient: &str,
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> GpgEncryptPlan {
    GpgEncryptPlan {
        program: gpg_program.into(),
        args: vec![
            "--batch".to_string(),
            "--yes".to_string(),
            "--no-tty".to_string(),
            "--trust-model".to_string(),
            "always".to_string(),
            "--encrypt".to_string(),
            "--recipient".to_string(),
            recipient.to_string(),
            "--output".to_string(),
            output.as_ref().display().to_string(),
            input.as_ref().display().to_string(),
        ],
    }
}

pub fn run_gpg_encrypt_plan(plan: &GpgEncryptPlan) -> Result<()> {
    let output = Command::new(&plan.program)
        .args(&plan.args)
        .output()
        .with_context(|| format!("failed to run GPG command: {}", plan.program))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let summary = stderr
            .lines()
            .next()
            .unwrap_or("GPG encryption failed")
            .trim();
        bail!("GPG encryption failed: {summary}");
    }

    Ok(())
}

pub fn gpg_decrypt_file(gpg_program: impl AsRef<str>, input: impl AsRef<Path>) -> Result<Vec<u8>> {
    let input = input.as_ref();
    let program = gpg_program.as_ref();
    let input_arg = input.display().to_string();
    let terminal = enter_gpg_terminal_passthrough()?;
    let mut command = Command::new(program);
    command.args(["--yes", "--decrypt", &input_arg]);
    terminal.prepare_command(&mut command);
    let output = command
        .output()
        .with_context(|| format!("failed to run GPG command: {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let summary = stderr
            .lines()
            .next()
            .unwrap_or("GPG decryption failed")
            .trim();
        bail!("GPG decryption failed: {summary}");
    }

    Ok(output.stdout)
}

pub fn gpg_decrypt_file_noninteractive(
    gpg_program: impl AsRef<str>,
    input: impl AsRef<Path>,
) -> Result<Vec<u8>> {
    let input = input.as_ref();
    let program = gpg_program.as_ref();
    let input_arg = input.display().to_string();
    let output = Command::new(program)
        .args([
            "--batch",
            "--yes",
            "--pinentry-mode",
            "error",
            "--decrypt",
            &input_arg,
        ])
        .output()
        .with_context(|| format!("failed to run GPG command: {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let summary = stderr
            .lines()
            .next()
            .unwrap_or("GPG noninteractive decryption failed")
            .trim();
        bail!("GPG noninteractive decryption failed: {summary}");
    }

    Ok(output.stdout)
}

pub(crate) struct GpgTerminalPassthrough {
    _raw_mode_pause: RawModePause,
    tty: Option<String>,
}

impl GpgTerminalPassthrough {
    fn enter() -> Result<Self> {
        let raw_mode_pause = pause_terminal_raw_mode_for_gpg()?;
        let tty = resolve_gpg_tty();
        if let Some(tty) = tty.as_deref() {
            update_gpg_agent_tty(tty);
        }
        Ok(Self {
            _raw_mode_pause: raw_mode_pause,
            tty,
        })
    }

    pub fn prepare_command(&self, command: &mut Command) {
        if let Some(tty) = &self.tty {
            command.env("GPG_TTY", tty);
        }
    }
}

pub(crate) fn enter_gpg_terminal_passthrough() -> Result<GpgTerminalPassthrough> {
    GpgTerminalPassthrough::enter()
}

struct RawModePause {
    was_enabled: bool,
}

impl RawModePause {
    fn new() -> Result<Self> {
        let was_enabled = is_raw_mode_enabled().unwrap_or(false);
        if was_enabled {
            disable_raw_mode().context("failed to leave raw mode for GPG pinentry")?;
        }
        Ok(Self { was_enabled })
    }
}

fn pause_terminal_raw_mode_for_gpg() -> Result<RawModePause> {
    RawModePause::new()
}

impl Drop for RawModePause {
    fn drop(&mut self) {
        if self.was_enabled {
            let _ = enable_raw_mode();
        }
    }
}

fn resolve_gpg_tty() -> Option<String> {
    std::env::var("GPG_TTY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(current_tty)
}

fn current_tty() -> Option<String> {
    let output = Command::new("tty").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (tty.starts_with('/')).then_some(tty)
}

fn update_gpg_agent_tty(tty: &str) {
    let _ = Command::new("gpg-connect-agent")
        .args(["updatestartuptty", "/bye"])
        .env("GPG_TTY", tty)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicGpgWritePaths {
    pub plaintext_tmp: PathBuf,
    pub encrypted_tmp: PathBuf,
    pub final_path: PathBuf,
}

pub fn atomic_gpg_write_paths(final_path: impl AsRef<Path>) -> AtomicGpgWritePaths {
    let final_path = final_path.as_ref().to_path_buf();
    let encrypted_tmp = final_path.with_extension(format!(
        "{}.tmp",
        final_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("gpg")
    ));
    let plaintext_tmp = final_path.with_extension("plain.tmp");
    AtomicGpgWritePaths {
        plaintext_tmp,
        encrypted_tmp,
        final_path,
    }
}

pub fn atomic_gpg_encrypt_bytes(
    gpg_program: impl Into<String>,
    recipient: &str,
    final_path: impl AsRef<Path>,
    plaintext: &[u8],
) -> Result<()> {
    let paths = atomic_gpg_write_paths(final_path);
    if let Some(parent) = paths
        .final_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_private_dir_all(parent).with_context(|| {
            format!(
                "failed to create encrypted output parent: {}",
                parent.display()
            )
        })?;
    }

    write_private_plaintext_tmp(&paths.plaintext_tmp, plaintext)?;
    let plan = gpg_encrypt_plan(
        gpg_program,
        recipient,
        &paths.plaintext_tmp,
        &paths.encrypted_tmp,
    );
    let encrypt_result = run_gpg_encrypt_plan(&plan);
    let _ = fs::remove_file(&paths.plaintext_tmp);
    if let Err(err) = encrypt_result {
        let _ = fs::remove_file(&paths.encrypted_tmp);
        return Err(err);
    }
    fs::rename(&paths.encrypted_tmp, &paths.final_path).with_context(|| {
        format!(
            "failed to move encrypted temp file into place: {} -> {}",
            paths.encrypted_tmp.display(),
            paths.final_path.display()
        )
    })?;
    set_private_file_permissions(&paths.final_path)?;
    Ok(())
}

pub fn load_encrypted_jsonl<T: DeserializeOwned>(
    gpg_program: impl AsRef<str>,
    plaintext_path: impl AsRef<Path>,
) -> Result<JsonlLoad<T>> {
    load_encrypted_jsonl_with_bytes(gpg_program, plaintext_path).map(|(loaded, _bytes)| loaded)
}

pub fn load_encrypted_jsonl_with_bytes<T: DeserializeOwned>(
    gpg_program: impl AsRef<str>,
    plaintext_path: impl AsRef<Path>,
) -> Result<(JsonlLoad<T>, Vec<u8>)> {
    let plaintext_path = plaintext_path.as_ref();
    let path = encrypted_path(plaintext_path);
    if !path.exists() {
        return Ok((
            JsonlLoad {
                items: Vec::new(),
                errors: Vec::new(),
            },
            Vec::new(),
        ));
    }
    let bytes = gpg_decrypt_file(gpg_program, &path)?;
    let loaded = load_jsonl_bytes(&path, &bytes)?;
    Ok((loaded, bytes))
}

pub fn load_encrypted_jsonl_with_bytes_noninteractive<T: DeserializeOwned>(
    gpg_program: impl AsRef<str>,
    plaintext_path: impl AsRef<Path>,
) -> Result<(JsonlLoad<T>, Vec<u8>)> {
    let plaintext_path = plaintext_path.as_ref();
    let path = encrypted_path(plaintext_path);
    if !path.exists() {
        return Ok((
            JsonlLoad {
                items: Vec::new(),
                errors: Vec::new(),
            },
            Vec::new(),
        ));
    }
    let bytes = gpg_decrypt_file_noninteractive(gpg_program, &path)?;
    let loaded = load_jsonl_bytes(&path, &bytes)?;
    Ok((loaded, bytes))
}

pub fn append_encrypted_jsonl<T: Serialize>(
    gpg_program: impl Into<String>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
    item: &T,
) -> Result<()> {
    let gpg_program = gpg_program.into();
    let plaintext_path = plaintext_path.as_ref();
    let mut line = Vec::new();
    serde_json::to_writer(&mut line, item).with_context(|| {
        format!(
            "failed to serialize encrypted JSONL item for {}",
            encrypted_path(plaintext_path).display()
        )
    })?;
    append_encrypted_jsonl_bytes(gpg_program, recipient, plaintext_path, &line)
}

pub fn append_encrypted_jsonl_bytes(
    gpg_program: impl Into<String>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
    item_json: &[u8],
) -> Result<()> {
    let gpg_program = gpg_program.into();
    let plaintext_path = plaintext_path.as_ref();
    let mut bytes = existing_jsonl_bytes(&gpg_program, plaintext_path)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(item_json);
    bytes.push(b'\n');
    atomic_gpg_encrypt_bytes(
        &gpg_program,
        recipient,
        encrypted_path(plaintext_path),
        &bytes,
    )?;
    remove_plaintext_if_present(plaintext_path)?;
    Ok(())
}

pub fn rewrite_encrypted_jsonl<T: Serialize>(
    gpg_program: impl Into<String>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
    items: &[T],
) -> Result<()> {
    let plaintext_path = plaintext_path.as_ref();
    let bytes = jsonl_bytes(items, plaintext_path)?;
    rewrite_encrypted_jsonl_bytes(gpg_program, recipient, plaintext_path, &bytes)
}

pub fn rewrite_encrypted_jsonl_bytes(
    gpg_program: impl Into<String>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<()> {
    let plaintext_path = plaintext_path.as_ref();
    atomic_gpg_encrypt_bytes(
        gpg_program,
        recipient,
        encrypted_path(plaintext_path),
        bytes,
    )?;
    remove_plaintext_if_present(plaintext_path)?;
    Ok(())
}

pub fn migrate_plaintext_jsonl_to_gpg(
    gpg_program: impl Into<String>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
) -> Result<bool> {
    let plaintext_path = plaintext_path.as_ref();
    if !plaintext_path.exists() {
        return Ok(false);
    }
    let bytes = fs::read(plaintext_path)
        .with_context(|| format!("failed to read plaintext file {}", plaintext_path.display()))?;
    atomic_gpg_encrypt_bytes(
        gpg_program,
        recipient,
        encrypted_path(plaintext_path),
        &bytes,
    )?;
    remove_plaintext_if_present(plaintext_path)?;
    Ok(true)
}

pub fn reencrypt_gpg_jsonl(
    gpg_program: impl AsRef<str>,
    recipient: &str,
    plaintext_path: impl AsRef<Path>,
) -> Result<bool> {
    let plaintext_path = plaintext_path.as_ref();
    let path = encrypted_path(plaintext_path);
    if !path.exists() {
        return Ok(false);
    }
    let program = gpg_program.as_ref();
    let bytes = gpg_decrypt_file(program, &path)?;
    atomic_gpg_encrypt_bytes(program.to_string(), recipient, &path, &bytes)?;
    Ok(true)
}

pub fn migrate_gpg_jsonl_to_plaintext(
    gpg_program: impl AsRef<str>,
    plaintext_path: impl AsRef<Path>,
) -> Result<bool> {
    let plaintext_path = plaintext_path.as_ref();
    let path = encrypted_path(plaintext_path);
    if !path.exists() {
        return Ok(false);
    }
    let bytes = gpg_decrypt_file(gpg_program, &path)?;
    atomic_plaintext_write(plaintext_path, &bytes)?;
    fs::remove_file(&path)
        .with_context(|| format!("failed to remove encrypted file {}", path.display()))?;
    Ok(true)
}

pub(crate) fn existing_jsonl_bytes(gpg_program: &str, plaintext_path: &Path) -> Result<Vec<u8>> {
    let encrypted = encrypted_path(plaintext_path);
    if encrypted.exists() {
        return gpg_decrypt_file(gpg_program, encrypted);
    }
    match fs::read(plaintext_path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err)
            .with_context(|| format!("failed to read JSONL file {}", plaintext_path.display())),
    }
}

pub(crate) fn jsonl_bytes<T: Serialize>(items: &[T], path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for item in items {
        serde_json::to_writer(&mut bytes, item)
            .with_context(|| format!("failed to serialize JSONL item for {}", path.display()))?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn load_jsonl_bytes<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<JsonlLoad<T>> {
    let reader = BufReader::new(Cursor::new(bytes));
    let mut items = Vec::new();
    let mut errors = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.with_context(|| {
            format!(
                "failed to read line {line_number} from encrypted JSONL file {}",
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(&line) {
            Ok(item) => items.push(item),
            Err(error) => errors.push(JsonlLineError {
                path: path.to_path_buf(),
                line: line_number,
                message: error.to_string(),
            }),
        }
    }

    Ok(JsonlLoad { items, errors })
}

fn atomic_plaintext_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent).with_context(|| {
            format!("failed to create plaintext directory {}", parent.display())
        })?;
    }
    let tmp = path.with_extension(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!("{extension}.tmp"))
            .unwrap_or_else(|| "tmp".to_string()),
    );
    write_private_file(&tmp, bytes)
        .with_context(|| format!("failed to write plaintext temp file {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to replace plaintext file {} with {}",
            path.display(),
            tmp.display()
        )
    })?;
    set_private_file_permissions(path)?;
    Ok(())
}

fn remove_plaintext_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("failed to remove plaintext file {}", path.display()))
        }
    }
}

#[cfg(unix)]
fn write_private_plaintext_tmp(path: &Path, plaintext: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create plaintext temp file: {}", path.display()))?;
    file.write_all(plaintext)
        .with_context(|| format!("failed to write plaintext temp file: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync plaintext temp file: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private_plaintext_tmp(path: &Path, plaintext: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to create plaintext temp file: {}", path.display()))?;
    file.write_all(plaintext)
        .with_context(|| format!("failed to write plaintext temp file: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync plaintext temp file: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_executable(path: &Path, contents: &str) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[test]
    fn plaintext_git_history_warning_is_conservative() {
        let warning = encryption_git_history_warning();

        assert!(warning.contains("plaintext data"));
        assert!(warning.contains("older key"));
        assert!(warning.contains("will not rewrite git history"));
    }

    #[test]
    fn parse_gpg_public_keys_reads_primary_fingerprints_and_uids() {
        let keys = parse_gpg_public_keys(
            "tru::1:0:0:0:0:0:0:0::\n\
             pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:\n\
             fpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:\n\
             uid:u::::1::hash::Test User <test@example.invalid>::::::::::0:\n\
             sub:u:255:18:2222222222222222:1::::::e::::::23:\n\
             fpr:::::::::BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB:\n\
             pub:u:255:22:3333333333333333:1:::u:::scESC::::::23::0:\n\
             fpr:::::::::CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC:\n",
        );

        assert_eq!(
            keys,
            vec![
                GpgPublicKey {
                    fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                    user_ids: vec!["Test User <test@example.invalid>".to_string()],
                },
                GpgPublicKey {
                    fingerprint: "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC".to_string(),
                    user_ids: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn resolve_gpg_key_fingerprint_accepts_unique_selector() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nif [ \"$1\" = \"--batch\" ]; then\n  printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n  printf '%s\\n' 'fpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:'\n  printf '%s\\n' 'uid:u::::1::hash::Test User <test@example.invalid>::::::::::0:'\n  exit 0\nfi\nexit 2\n",
        );

        let fingerprint =
            resolve_gpg_key_fingerprint(fake_gpg.display().to_string(), "test@example.invalid")
                .unwrap();

        assert_eq!(fingerprint, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    }

    #[test]
    fn resolve_gpg_key_fingerprint_rejects_ambiguous_selector() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nprintf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\nprintf '%s\\n' 'fpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:'\nprintf '%s\\n' 'pub:u:255:22:2222222222222222:1:::u:::scESC::::::23::0:'\nprintf '%s\\n' 'fpr:::::::::BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB:'\n",
        );

        let err =
            resolve_gpg_key_fingerprint(fake_gpg.display().to_string(), "test@example.invalid")
                .unwrap_err()
                .to_string();

        assert!(err.contains("ambiguous"));
        assert!(err.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
        assert!(err.contains("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"));
    }

    #[test]
    fn encrypted_path_appends_gpg_extension() {
        assert_eq!(
            encrypted_path("history/regular.jsonl"),
            PathBuf::from("history/regular.jsonl.gpg")
        );
        assert_eq!(encrypted_path("secret"), PathBuf::from("secret.gpg"));
    }

    #[test]
    fn gpg_encrypt_plan_uses_batch_encrypt_arguments() {
        let plan = gpg_encrypt_plan("gpg", "test@example.invalid", "plain.txt", "plain.txt.gpg");

        assert_eq!(plan.program, "gpg");
        assert_eq!(
            plan.args,
            vec![
                "--batch",
                "--yes",
                "--no-tty",
                "--trust-model",
                "always",
                "--encrypt",
                "--recipient",
                "test@example.invalid",
                "--output",
                "plain.txt.gpg",
                "plain.txt"
            ]
        );
    }

    #[test]
    fn gpg_terminal_passthrough_prepares_command_tty_env() {
        let terminal = GpgTerminalPassthrough {
            _raw_mode_pause: RawModePause { was_enabled: false },
            tty: Some("/dev/pts/test".to_string()),
        };
        let mut command = Command::new("gpg");

        terminal.prepare_command(&mut command);

        let gpg_tty = command
            .get_envs()
            .find_map(|(key, value)| (key == "GPG_TTY").then_some(value))
            .flatten();
        assert_eq!(gpg_tty, Some(std::ffi::OsStr::new("/dev/pts/test")));
    }

    #[test]
    fn gpg_decrypt_file_passes_gpg_tty_to_decrypt_command() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let encrypted = temp.path().join("secret.json.gpg");
        fs::write(&encrypted, "decrypted bytes").unwrap();
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nif [ \"${GPG_TTY:-}\" != '/dev/pts/aish-test' ]; then\n  printf 'unexpected GPG_TTY: %s\\n' \"${GPG_TTY:-}\" >&2\n  exit 7\nfi\nif [ \"$1\" = '--yes' ] && [ \"$2\" = '--decrypt' ]; then\n  cat \"$3\"\n  exit 0\nfi\nprintf 'unexpected args\\n' >&2\nexit 2\n",
        );
        let old_gpg_tty = std::env::var_os("GPG_TTY");
        unsafe {
            std::env::set_var("GPG_TTY", "/dev/pts/aish-test");
        }

        let result = gpg_decrypt_file(fake_gpg.display().to_string(), &encrypted);

        unsafe {
            match old_gpg_tty {
                Some(value) => std::env::set_var("GPG_TTY", value),
                None => std::env::remove_var("GPG_TTY"),
            }
        }
        let bytes = result.unwrap();
        assert_eq!(bytes, b"decrypted bytes");
    }

    #[test]
    fn gpg_decrypt_file_noninteractive_never_uses_pinentry() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let encrypted = temp.path().join("secret.json.gpg");
        fs::write(&encrypted, "cached decrypt").unwrap();
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nseen_batch=0\nseen_pinentry=0\nseen_error=0\nfor arg in \"$@\"; do\n  [ \"$arg\" = '--batch' ] && seen_batch=1\n  [ \"$arg\" = '--pinentry-mode' ] && seen_pinentry=1\n  [ \"$arg\" = 'error' ] && seen_error=1\ndone\nif [ \"$seen_batch:$seen_pinentry:$seen_error\" != '1:1:1' ]; then\n  printf 'missing noninteractive flags\\n' >&2\n  exit 8\nfi\ncat \"$6\"\n",
        );

        let bytes =
            gpg_decrypt_file_noninteractive(fake_gpg.display().to_string(), &encrypted).unwrap();

        assert_eq!(bytes, b"cached decrypt");
    }

    #[test]
    fn run_gpg_encrypt_plan_supports_fake_gpg_success() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let input = temp.path().join("plain.txt");
        let output = temp.path().join("plain.txt.gpg");
        fs::write(&input, "secret plaintext").unwrap();
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nout=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then\n    shift\n    out=\"$1\"\n  fi\n  shift\ndone\nprintf 'encrypted-placeholder\\n' > \"$out\"\n",
        );
        let plan = gpg_encrypt_plan(fake_gpg.display().to_string(), "recipient", &input, &output);

        run_gpg_encrypt_plan(&plan).unwrap();

        assert_eq!(
            fs::read_to_string(output).unwrap(),
            "encrypted-placeholder\n"
        );
    }

    #[test]
    fn run_gpg_encrypt_plan_reports_failure_without_stdout_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let input = temp.path().join("plain.txt");
        let output = temp.path().join("plain.txt.gpg");
        fs::write(&input, "secret plaintext").unwrap();
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nprintf 'secret plaintext should not be surfaced\\n'\nprintf 'no public key\\n' >&2\nexit 2\n",
        );
        let plan = gpg_encrypt_plan(fake_gpg.display().to_string(), "recipient", &input, &output);

        let err = run_gpg_encrypt_plan(&plan).unwrap_err().to_string();

        assert!(err.contains("GPG encryption failed: no public key"));
        assert!(!err.contains("secret plaintext"));
        assert!(!output.exists());
    }

    #[test]
    fn atomic_gpg_write_paths_keep_temp_files_next_to_output() {
        let paths = atomic_gpg_write_paths("secrets/key.json.gpg");

        assert_eq!(
            paths.plaintext_tmp,
            PathBuf::from("secrets/key.json.plain.tmp")
        );
        assert_eq!(
            paths.encrypted_tmp,
            PathBuf::from("secrets/key.json.gpg.tmp")
        );
        assert_eq!(paths.final_path, PathBuf::from("secrets/key.json.gpg"));
    }

    #[test]
    fn atomic_gpg_write_paths_support_relative_output_without_parent() {
        let paths = atomic_gpg_write_paths("secret.json.gpg");

        assert_eq!(paths.plaintext_tmp, PathBuf::from("secret.json.plain.tmp"));
        assert_eq!(paths.encrypted_tmp, PathBuf::from("secret.json.gpg.tmp"));
        assert_eq!(paths.final_path, PathBuf::from("secret.json.gpg"));
    }

    #[test]
    fn atomic_gpg_encrypt_bytes_writes_final_output_and_removes_temps() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let final_path = temp.path().join("secret.json.gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nout=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then\n    shift\n    out=\"$1\"\n  fi\n  shift\ndone\nprintf 'encrypted bytes\\n' > \"$out\"\n",
        );

        atomic_gpg_encrypt_bytes(
            fake_gpg.display().to_string(),
            "recipient",
            &final_path,
            b"secret",
        )
        .unwrap();

        let paths = atomic_gpg_write_paths(&final_path);
        assert_eq!(
            fs::read_to_string(&final_path).unwrap(),
            "encrypted bytes\n"
        );
        assert!(!paths.plaintext_tmp.exists());
        assert!(!paths.encrypted_tmp.exists());
        #[cfg(unix)]
        {
            let mode = fs::metadata(&final_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn atomic_gpg_encrypt_bytes_removes_plaintext_tmp_on_failure() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let final_path = temp.path().join("secret.json.gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nprintf 'fake failure\\n' >&2\nexit 2\n",
        );

        let err = atomic_gpg_encrypt_bytes(
            fake_gpg.display().to_string(),
            "recipient",
            &final_path,
            b"secret",
        )
        .unwrap_err()
        .to_string();

        let paths = atomic_gpg_write_paths(&final_path);
        assert!(err.contains("GPG encryption failed: fake failure"));
        assert!(!paths.plaintext_tmp.exists());
        assert!(!paths.encrypted_tmp.exists());
        assert!(!final_path.exists());
    }

    #[test]
    fn encrypted_jsonl_helpers_roundtrip_through_fake_gpg() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let path = temp.path().join("history/regular.jsonl");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --batch|--yes|--no-tty|--trust-model|always|--encrypt|--recipient) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  cp \"$input\" \"$out\"\nfi\n",
        );

        append_encrypted_jsonl(
            fake_gpg.display().to_string(),
            "recipient",
            &path,
            &serde_json::json!({"command": "pwd"}),
        )
        .unwrap();
        append_encrypted_jsonl(
            fake_gpg.display().to_string(),
            "recipient",
            &path,
            &serde_json::json!({"command": "ls"}),
        )
        .unwrap();

        let loaded =
            load_encrypted_jsonl::<serde_json::Value>(fake_gpg.display().to_string(), &path)
                .unwrap();

        assert!(!path.exists());
        assert!(encrypted_path(&path).exists());
        #[cfg(unix)]
        {
            let mode = fs::metadata(encrypted_path(&path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0]["command"], "pwd");
        assert_eq!(loaded.items[1]["command"], "ls");
    }
}
