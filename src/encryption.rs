use std::path::Path;
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
}
