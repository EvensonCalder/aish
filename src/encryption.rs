use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::{
    create_private_dir_all, set_private_file_handle_permissions, set_private_file_permissions,
    write_private_file,
};
use crate::history::{JsonlLineError, JsonlLoad};

mod keys;

#[cfg(test)]
use keys::parse_gpg_public_keys;
pub use keys::{GpgPublicKey, list_matching_gpg_public_keys, resolve_gpg_key_fingerprint};

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
            "--".to_string(),
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
        let summary = gpg_stderr_summary(&output.stderr, "GPG encryption failed");
        bail!("GPG encryption failed: {summary}");
    }

    Ok(())
}

pub fn gpg_decrypt_file(gpg_program: impl AsRef<str>, input: impl AsRef<Path>) -> Result<Vec<u8>> {
    let input = input.as_ref();
    let program = gpg_program.as_ref();
    let terminal = enter_gpg_terminal_passthrough()?;
    let output = run_gpg_decrypt_command(program, input, DecryptMode::Interactive, Some(&terminal))
        .with_context(|| format!("failed to run GPG command: {program}"))?;

    if !output.status.success() {
        if gpg_stderr_mentions_multiple_plaintexts(&output.stderr)
            && let Some(bytes) = decrypt_concatenated_openpgp_messages(
                program,
                input,
                DecryptMode::Interactive,
                Some(&terminal),
            )?
        {
            return Ok(bytes);
        }
        let summary = gpg_stderr_summary(&output.stderr, "GPG decryption failed");
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
    let output = run_gpg_decrypt_command(program, input, DecryptMode::Noninteractive, None)
        .with_context(|| format!("failed to run GPG command: {program}"))?;

    if !output.status.success() {
        if gpg_stderr_mentions_multiple_plaintexts(&output.stderr)
            && let Some(bytes) = decrypt_concatenated_openpgp_messages(
                program,
                input,
                DecryptMode::Noninteractive,
                None,
            )?
        {
            return Ok(bytes);
        }
        let summary = gpg_stderr_summary(&output.stderr, "GPG noninteractive decryption failed");
        bail!("GPG noninteractive decryption failed: {summary}");
    }

    Ok(output.stdout)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecryptMode {
    Interactive,
    Noninteractive,
}

fn run_gpg_decrypt_command(
    program: &str,
    input: &Path,
    mode: DecryptMode,
    terminal: Option<&GpgTerminalPassthrough>,
) -> Result<Output> {
    let input_arg = input.display().to_string();
    let mut command = Command::new(program);
    match mode {
        DecryptMode::Interactive => {
            command.args(["--yes", "--decrypt", "--", &input_arg]);
        }
        DecryptMode::Noninteractive => {
            command.args([
                "--batch",
                "--yes",
                "--pinentry-mode",
                "error",
                "--decrypt",
                "--",
                &input_arg,
            ]);
        }
    }
    if let Some(terminal) = terminal {
        terminal.prepare_command(&mut command);
    }
    command.output().map_err(Into::into)
}

fn gpg_stderr_mentions_multiple_plaintexts(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("multiple plaintexts seen")
}

fn gpg_stderr_summary(stderr: &[u8], fallback: &str) -> String {
    let lines: Vec<String> = String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if lines.is_empty() {
        return fallback.to_string();
    }

    let relevant: Vec<String> = lines
        .iter()
        .filter(|line| {
            !line.starts_with("gpg: encrypted with ")
                && !line.starts_with('"')
                && !line.starts_with("“")
        })
        .cloned()
        .collect();
    let source = if relevant.is_empty() {
        lines.as_slice()
    } else {
        relevant.as_slice()
    };
    let start = source.len().saturating_sub(3);
    let mut summary = source[start..].join("; ");
    const MAX_SUMMARY_CHARS: usize = 500;
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        summary = summary.chars().take(MAX_SUMMARY_CHARS).collect();
        summary.push_str("...");
    }
    summary
}

fn decrypt_concatenated_openpgp_messages(
    program: &str,
    input: &Path,
    mode: DecryptMode,
    terminal: Option<&GpgTerminalPassthrough>,
) -> Result<Option<Vec<u8>>> {
    let encrypted = fs::read(input)
        .with_context(|| format!("failed to read encrypted file {}", input.display()))?;
    let Some(messages) = split_concatenated_openpgp_messages(&encrypted) else {
        return Ok(None);
    };

    let mut plaintext = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let part_path = decrypt_part_temp_path(input, index);
        let decrypt_result = (|| -> Result<Output> {
            write_private_file(&part_path, message).with_context(|| {
                format!(
                    "failed to write temporary GPG message {}",
                    part_path.display()
                )
            })?;
            run_gpg_decrypt_command(program, &part_path, mode, terminal)
                .with_context(|| format!("failed to run GPG command: {program}"))
        })();
        let _ = fs::remove_file(&part_path);
        let output = decrypt_result?;
        if !output.status.success() {
            let summary = gpg_stderr_summary(&output.stderr, "GPG decryption failed");
            bail!(
                "failed to decrypt OpenPGP message {} of {} in {}: {summary}",
                index + 1,
                messages.len(),
                input.display()
            );
        }
        plaintext.extend_from_slice(&output.stdout);
    }

    Ok(Some(plaintext))
}

fn decrypt_part_temp_path(input: &Path, index: usize) -> PathBuf {
    let filename = input
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "message.gpg".into());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let part_name = format!(
        ".{filename}.{}.{}.{}.part.gpg",
        std::process::id(),
        nonce,
        index
    );
    input.with_file_name(part_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenPgpPacket {
    offset: usize,
    tag: u8,
    end: usize,
}

fn split_concatenated_openpgp_messages(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let mut packets = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let packet = parse_openpgp_packet(bytes, offset)?;
        offset = packet.end;
        packets.push(packet);
    }

    let mut starts = vec![0];
    let mut previous_was_encrypted_data = false;
    for packet in packets.iter().skip(1) {
        if previous_was_encrypted_data && is_session_key_packet(packet.tag) {
            starts.push(packet.offset);
        }
        previous_was_encrypted_data = is_encrypted_data_packet(packet.tag);
    }
    if starts.len() < 2 {
        return None;
    }

    starts.push(bytes.len());
    Some(
        starts
            .windows(2)
            .map(|window| &bytes[window[0]..window[1]])
            .collect(),
    )
}

fn parse_openpgp_packet(bytes: &[u8], offset: usize) -> Option<OpenPgpPacket> {
    let ctb = *bytes.get(offset)?;
    if ctb & 0x80 == 0 {
        return None;
    }
    if ctb & 0x40 != 0 {
        parse_new_openpgp_packet(bytes, offset, ctb)
    } else {
        parse_old_openpgp_packet(bytes, offset, ctb)
    }
}

fn parse_new_openpgp_packet(bytes: &[u8], offset: usize, ctb: u8) -> Option<OpenPgpPacket> {
    let tag = ctb & 0x3f;
    let mut cursor = offset.checked_add(1)?;
    loop {
        let length_octet = *bytes.get(cursor)?;
        cursor = cursor.checked_add(1)?;
        let length = match length_octet {
            0..=191 => usize::from(length_octet),
            192..=223 => {
                let second = usize::from(*bytes.get(cursor)?);
                cursor = cursor.checked_add(1)?;
                ((usize::from(length_octet) - 192) << 8) + second + 192
            }
            224..=254 => 1_usize.checked_shl(u32::from(length_octet & 0x1f))?,
            255 => {
                let raw = bytes.get(cursor..cursor.checked_add(4)?)?;
                cursor = cursor.checked_add(4)?;
                u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
            }
        };
        cursor = cursor.checked_add(length)?;
        if cursor > bytes.len() {
            return None;
        }
        if !(224..=254).contains(&length_octet) {
            return Some(OpenPgpPacket {
                offset,
                tag,
                end: cursor,
            });
        }
    }
}

fn parse_old_openpgp_packet(bytes: &[u8], offset: usize, ctb: u8) -> Option<OpenPgpPacket> {
    let tag = (ctb >> 2) & 0x0f;
    let length_type = ctb & 0x03;
    let mut cursor = offset.checked_add(1)?;
    let length = match length_type {
        0 => usize::from(*bytes.get(cursor)?),
        1 => {
            let raw = bytes.get(cursor..cursor.checked_add(2)?)?;
            u16::from_be_bytes([raw[0], raw[1]]) as usize
        }
        2 => {
            let raw = bytes.get(cursor..cursor.checked_add(4)?)?;
            u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
        }
        3 => bytes.len().checked_sub(cursor)?,
        _ => unreachable!(),
    };
    cursor = cursor.checked_add(match length_type {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 0,
        _ => unreachable!(),
    })?;
    let end = cursor.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    Some(OpenPgpPacket { offset, tag, end })
}

fn is_session_key_packet(tag: u8) -> bool {
    matches!(tag, 1 | 3)
}

fn is_encrypted_data_packet(tag: u8) -> bool {
    matches!(tag, 9 | 18 | 20)
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

pub fn append_gpg_encrypt_bytes(
    gpg_program: impl Into<String>,
    recipient: &str,
    final_path: impl AsRef<Path>,
    plaintext: &[u8],
) -> Result<()> {
    let gpg_program = gpg_program.into();
    let final_path = final_path.as_ref();
    let mut bytes = if final_path.exists() {
        gpg_decrypt_file(&gpg_program, final_path)
            .with_context(|| format!("failed to decrypt {}", final_path.display()))?
    } else {
        Vec::new()
    };
    bytes.extend_from_slice(plaintext);
    atomic_gpg_encrypt_bytes(gpg_program, recipient, final_path, &bytes)
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
    let bytes = gpg_decrypt_file(gpg_program, &path)
        .with_context(|| format!("failed to decrypt encrypted JSONL file {}", path.display()))?;
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
    let bytes = gpg_decrypt_file_noninteractive(gpg_program, &path)
        .with_context(|| format!("failed to decrypt encrypted JSONL file {}", path.display()))?;
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
    let mut bytes = Vec::with_capacity(item_json.len() + 1);
    bytes.extend_from_slice(&existing_jsonl_bytes(&gpg_program, plaintext_path)?);
    bytes.extend_from_slice(item_json);
    bytes.push(b'\n');
    rewrite_encrypted_jsonl_bytes(gpg_program, recipient, plaintext_path, &bytes)?;
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
    let bytes = gpg_decrypt_file(program, &path)
        .with_context(|| format!("failed to decrypt encrypted JSONL file {}", path.display()))?;
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
    let bytes = gpg_decrypt_file(gpg_program, &path)
        .with_context(|| format!("failed to decrypt encrypted JSONL file {}", path.display()))?;
    atomic_plaintext_write(plaintext_path, &bytes)?;
    fs::remove_file(&path)
        .with_context(|| format!("failed to remove encrypted file {}", path.display()))?;
    Ok(true)
}

pub(crate) fn existing_jsonl_bytes(gpg_program: &str, plaintext_path: &Path) -> Result<Vec<u8>> {
    let encrypted = encrypted_path(plaintext_path);
    if encrypted.exists() {
        return gpg_decrypt_file(gpg_program, &encrypted).with_context(|| {
            format!(
                "failed to decrypt encrypted JSONL file {}",
                encrypted.display()
            )
        });
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
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to create plaintext temp file: {}", path.display()))?;
    set_private_file_handle_permissions(&file, path)?;
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
        let tmp = path.with_extension("tmp");
        let mut file = fs::File::create(&tmp).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(&tmp, path).unwrap();
        if let Some(parent) = path.parent() {
            let _ = fs::File::open(parent).and_then(|dir| dir.sync_all());
        }
    }
    #[test]
    fn plaintext_git_history_warning_is_conservative() {
        let warning = encryption_git_history_warning();

        assert!(warning.contains("plaintext data"));
        assert!(warning.contains("older key"));
        assert!(warning.contains("will not rewrite git history"));
    }

    #[test]
    fn gpg_stderr_summary_prefers_actionable_failure_lines() {
        let stderr = b"gpg: encrypted with cv25519 key, ID 1234\n      \"Test User\"\ngpg: WARNING: multiple plaintexts seen\ngpg: handle plaintext failed: Unexpected error\ngpg: decryption failed: Bad data\n";

        let summary = gpg_stderr_summary(stderr, "fallback");

        assert!(summary.contains("multiple plaintexts seen"));
        assert!(summary.contains("decryption failed: Bad data"));
        assert!(!summary.contains("encrypted with cv25519 key"));
    }

    #[test]
    fn openpgp_splitter_finds_concatenated_encrypted_messages() {
        let first = [0x84, 0x01, 0xaa, 0xd4, 0x01, 0xbb];
        let second = [0x84, 0x01, 0xcc, 0xd4, 0x01, 0xdd];
        let mut concatenated = Vec::new();
        concatenated.extend_from_slice(&first);
        concatenated.extend_from_slice(&second);

        let messages = split_concatenated_openpgp_messages(&concatenated).unwrap();

        assert_eq!(messages, vec![first.as_slice(), second.as_slice()]);
    }

    #[test]
    fn openpgp_splitter_leaves_single_message_alone() {
        let message = [0x84, 0x01, 0xaa, 0xd4, 0x01, 0xbb];

        assert!(split_concatenated_openpgp_messages(&message).is_none());
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
    fn resolve_gpg_key_fingerprint_passes_selector_after_option_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--list-keys' ]; then\n    shift\n    [ \"$1\" = '--' ] || { printf 'missing option boundary\\n' >&2; exit 9; }\n    shift\n    [ \"$1\" = '--looks-like-option' ] || { printf 'unexpected selector: %s\\n' \"$1\" >&2; exit 10; }\n    printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n    printf '%s\\n' 'fpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:'\n    exit 0\n  fi\n  shift\ndone\nexit 2\n",
        );

        let fingerprint =
            resolve_gpg_key_fingerprint(fake_gpg.display().to_string(), "--looks-like-option")
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

        assert!(err.contains("ambiguous"), "unexpected error: {err}");
        assert!(err.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
        assert!(err.contains("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"));
    }

    #[test]
    fn encrypted_jsonl_append_rewrites_existing_file_as_single_message() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        write_executable(
            &fake_gpg,
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"recipient\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model|--pinentry-mode) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always|error) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  sed '1{/^recipient:/d;}' \"$input\"\nelse\n  { printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; } > \"$out\"\nfi\n",
        );
        let plaintext_path = temp.path().join("history/regular.jsonl");
        let encrypted = encrypted_path(&plaintext_path);
        fs::create_dir_all(encrypted.parent().unwrap()).unwrap();
        fs::write(&encrypted, b"recipient:old-key\n{\"old\":true}\n").unwrap();

        append_encrypted_jsonl_bytes(
            fake_gpg.display().to_string(),
            "new-key",
            &plaintext_path,
            br#"{"new":true}"#,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(encrypted).unwrap(),
            "recipient:new-key\n{\"old\":true}\n{\"new\":true}\n"
        );
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
                "--",
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
            "#!/bin/sh\nif [ \"${GPG_TTY:-}\" != '/dev/pts/aish-test' ]; then\n  printf 'unexpected GPG_TTY: %s\\n' \"${GPG_TTY:-}\" >&2\n  exit 7\nfi\nif [ \"$1\" = '--yes' ] && [ \"$2\" = '--decrypt' ] && [ \"$3\" = '--' ]; then\n  cat \"$4\"\n  exit 0\nfi\nprintf 'unexpected args\\n' >&2\nexit 2\n",
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
            "#!/bin/sh\nseen_batch=0\nseen_pinentry=0\nseen_error=0\ninput=\"\"\nfor arg in \"$@\"; do\n  [ \"$arg\" = '--batch' ] && seen_batch=1\n  [ \"$arg\" = '--pinentry-mode' ] && seen_pinentry=1\n  [ \"$arg\" = 'error' ] && seen_error=1\n  input=\"$arg\"\ndone\nif [ \"$seen_batch:$seen_pinentry:$seen_error\" != '1:1:1' ]; then\n  printf 'missing noninteractive flags\\n' >&2\n  exit 8\nfi\ncat \"$input\"\n",
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
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"recipient\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model|--pinentry-mode) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always|error) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  count=$(grep -c '^recipient:' \"$input\" || true)\n  if [ \"$count\" -gt 1 ]; then\n    printf 'gpg: WARNING: multiple plaintexts seen\\n' >&2\n    printf 'gpg: decryption failed: Bad data\\n' >&2\n    exit 2\n  fi\n  sed '1{/^recipient:/d;}' \"$input\"\nelse\n  { printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; } > \"$out\"\nfi\n",
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
        let encrypted = fs::read_to_string(encrypted_path(&path)).unwrap();
        assert_eq!(encrypted.matches("recipient:recipient\n").count(), 1);
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
