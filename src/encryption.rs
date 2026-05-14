use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn plaintext_git_history_warning() -> &'static str {
    "warning: existing plaintext data may remain in git history; Aish will not rewrite history automatically"
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
        fs::create_dir_all(parent).with_context(|| {
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
    Ok(())
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
    #[test]
    fn plaintext_git_history_warning_is_conservative() {
        let warning = plaintext_git_history_warning();

        assert!(warning.contains("plaintext data may remain in git history"));
        assert!(warning.contains("will not rewrite history automatically"));
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
    fn run_gpg_encrypt_plan_supports_fake_gpg_success() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let input = temp.path().join("plain.txt");
        let output = temp.path().join("plain.txt.gpg");
        fs::write(&input, "secret plaintext").unwrap();
        fs::write(
            &fake_gpg,
            "#!/bin/sh\nout=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then\n    shift\n    out=\"$1\"\n  fi\n  shift\ndone\nprintf 'encrypted-placeholder\\n' > \"$out\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();
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
        fs::write(
            &fake_gpg,
            "#!/bin/sh\nprintf 'secret plaintext should not be surfaced\\n'\nprintf 'no public key\\n' >&2\nexit 2\n",
        )
        .unwrap();
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();
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
        fs::write(
            &fake_gpg,
            "#!/bin/sh\nout=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then\n    shift\n    out=\"$1\"\n  fi\n  shift\ndone\nprintf 'encrypted bytes\\n' > \"$out\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();

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
    }

    #[test]
    fn atomic_gpg_encrypt_bytes_removes_plaintext_tmp_on_failure() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = temp.path().join("fake-gpg");
        let final_path = temp.path().join("secret.json.gpg");
        fs::write(
            &fake_gpg,
            "#!/bin/sh\nprintf 'fake failure\\n' >&2\nexit 2\n",
        )
        .unwrap();
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();

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
}
