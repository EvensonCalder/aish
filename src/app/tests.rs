use super::encryption_commands::{StoredApiKey, write_history_rewrite_script};
use super::startup_unlock::{EncryptedStartupPaths, UnlockMode, load_encrypted_startup_data};
use super::state::OUTPUT_RING_CAPACITY;
use super::sync_commands::{run_startup_sync_check, write_last_sync_attempt};
use super::*;
use crate::completion::{CompletionCandidate, CompletionSource};
use crate::config::{
    self, AiConfig, CompletionConfig, CompletionMode, CompletionTabAccept, ContextConfig,
    EditorConfig, EncryptionConfig, PromptConfig, SyncConfig,
};
use crate::display_width::display_width;
use crate::editor::EditorCommand;
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{
    append_encrypted_jsonl, atomic_gpg_encrypt_bytes, gpg_decrypt_file, load_encrypted_jsonl,
    rewrite_encrypted_jsonl,
};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, HistoryEntry, HistorySource, NoteEntry,
    ai_command_indices, append_jsonl, load_jsonl,
};
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event, load_events};
use crate::modes::Mode;
use crate::pty::PtyBackend;
use crate::templates::{TemplateEntry, append_template, load_templates, template_id};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod ai_editor;
mod config_commands;
mod diagnostics;
mod drafts;
mod encryption_core;
mod encryption_runtime;
mod execution_private;
mod prompt_completion_config;
mod state_completion;
mod sync;
mod templates;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn write_executable_file(path: &Path, contents: impl AsRef<[u8]>) {
    use std::os::unix::fs::PermissionsExt;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).unwrap();
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&tmp, path).unwrap();
    if let Some(parent) = path.parent() {
        let _ = fs::File::open(parent).and_then(|dir| dir.sync_all());
    }
}

#[cfg(unix)]
fn write_fake_gpg(temp: &tempfile::TempDir) -> PathBuf {
    let fake_gpg = temp.path().join("fake-gpg");
    write_executable_file(
        &fake_gpg,
        "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\nlast=\"\"\nfor arg in \"$@\"; do\n  last=\"$arg\"\n  if [ \"$arg\" = \"--version\" ]; then printf 'fake gpg\\n'; exit 0; fi\ndone\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--list-keys\" ]; then\n    fpr='AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'\n    uid='Test User <test@example.invalid>'\n    case \"$last\" in\n      *BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB*|second@example.invalid) fpr='BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB'; uid='Second User <second@example.invalid>' ;;\n    esac\n    printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n    printf 'fpr:::::::::%s:\\n' \"$fpr\"\n    printf 'uid:u::::1::hash::%s:::::::::0:\\n' \"$uid\"\n    exit 0\n  fi\ndone\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model|--pinentry-mode) shift ;;\n    --batch|--yes|--no-tty|--encrypt|--with-colons|--fingerprint|error) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  count=$(grep -c '^recipient:' \"$input\" || true)\n  if [ \"$count\" -gt 1 ]; then\n    printf 'gpg: WARNING: multiple plaintexts seen\\n' >&2\n    printf 'gpg: decryption failed: Bad data\\n' >&2\n    exit 2\n  fi\n  sed '1{/^recipient:/d;}' \"$input\"\nelse\n  { printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; } > \"$out\"\nfi\n",
    );
    fake_gpg
}

#[cfg(unix)]
fn write_decrypt_marker_fake_gpg(temp: &tempfile::TempDir, fail_decrypt_marker: &Path) -> PathBuf {
    let fake_gpg = temp.path().join("marker-gpg");
    write_executable_file(
        &fake_gpg,
        format!(
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model|--pinentry-mode) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always|error) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  if [ -f '{}' ]; then\n    printf 'decrypt disabled\\n' >&2\n    exit 9\n  fi\n  sed '1{{/^recipient:/d;}}' \"$input\"\nelse\n  {{ printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; }} > \"$out\"\nfi\n",
            fail_decrypt_marker.display()
        ),
    );
    fake_gpg
}

#[cfg(unix)]
fn write_failing_ai_encrypt_gpg(temp: &tempfile::TempDir) -> PathBuf {
    let fake_gpg = temp.path().join("fail-ai-encrypt-gpg");
    write_executable_file(
        &fake_gpg,
        "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\nlast=\"\"\nfor arg in \"$@\"; do\n  last=\"$arg\"\n  if [ \"$arg\" = \"--version\" ]; then printf 'fake gpg\\n'; exit 0; fi\ndone\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--list-keys\" ]; then\n    printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n    printf '%s\\n' 'fpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:'\n    printf '%s\\n' 'uid:u::::1::hash::Test User <test@example.invalid>::::::::::0:'\n    exit 0\n  fi\ndone\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model|--pinentry-mode) shift ;;\n    --batch|--yes|--no-tty|--encrypt|--with-colons|--fingerprint|always|error) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  sed '1{/^recipient:/d;}' \"$input\"\nelse\n  case \"$out\" in\n    *history/ai.jsonl.gpg.tmp) printf 'planned encrypt failure\\n' >&2; exit 9 ;;\n  esac\n  { printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; } > \"$out\"\nfi\n",
    );
    fake_gpg
}

#[cfg(unix)]
fn write_blocking_fake_gpg(temp: &tempfile::TempDir, release_path: &Path) -> PathBuf {
    let fake_gpg = temp.path().join("blocking-gpg");
    write_executable_file(
        &fake_gpg,
        format!(
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  while [ ! -f '{}' ]; do sleep 0.02; done\n  cp \"$input\" \"$out\"\nfi\n",
            release_path.display()
        ),
    );
    fake_gpg
}

fn ai_requester_requires_stored_key(config: &AiConfig, _prompt: &str) -> Result<Vec<AiItem>> {
    assert_eq!(config.api_key_override.as_deref(), Some("secret-test-key"));
    assert_eq!(config.model, "test-model");
    Ok(vec![AiItem {
        kind: AiItemKind::Command,
        text: "pwd".to_string(),
        name: None,
    }])
}

fn test_completion_candidate(display: &str) -> CompletionCandidate {
    CompletionCandidate {
        display: display.to_string(),
        replacement: display.to_string(),
        is_dir: false,
        source: CompletionSource::History,
    }
}

fn run_test_git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_local_remote(remote: &Path, seed: &Path, root: &Path) {
    run_test_git(
        remote.parent().unwrap(),
        ["init", "--bare", remote.to_str().unwrap()],
    );
    fs::create_dir_all(seed).unwrap();
    run_test_git(seed, ["init"]);
    run_test_git(seed, ["config", "user.name", "Aish Test"]);
    run_test_git(seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(seed, ["config", "commit.gpgsign", "false"]);
    fs::write(seed.join("README.md"), "seed\n").unwrap();
    run_test_git(seed, ["add", "README.md"]);
    run_test_git(seed, ["commit", "-m", "seed"]);
    run_test_git(seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(seed, ["push", "-u", "origin", "HEAD"]);
    run_test_git(
        remote.parent().unwrap(),
        ["clone", remote.to_str().unwrap(), root.to_str().unwrap()],
    );
    run_test_git(root, ["config", "user.name", "Aish Test"]);
    run_test_git(root, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(root, ["config", "commit.gpgsign", "false"]);
}

fn fixed_clock() -> i64 {
    42
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}
