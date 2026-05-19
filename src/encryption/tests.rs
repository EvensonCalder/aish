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
        resolve_gpg_key_fingerprint(fake_gpg.display().to_string(), "--looks-like-option").unwrap();

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

    let err = resolve_gpg_key_fingerprint(fake_gpg.display().to_string(), "test@example.invalid")
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
        load_encrypted_jsonl::<serde_json::Value>(fake_gpg.display().to_string(), &path).unwrap();

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
