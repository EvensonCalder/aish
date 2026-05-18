use std::process::Command;

use anyhow::{Context, Result, bail};

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
            "--",
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

pub(super) fn parse_gpg_public_keys(output: &str) -> Vec<GpgPublicKey> {
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
