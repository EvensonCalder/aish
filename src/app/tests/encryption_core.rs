use super::*;

#[test]
fn encryption_and_sync_commands_report_current_state_without_side_effects() {
    for (line, expected) in [
        (
            "#encrypt on",
            "config path is not configured; #encrypt not saved",
        ),
        (
            "#set-remote git@example.invalid:aish.git",
            "config path is not configured; sync config not saved",
        ),
        ("#sync", "no git command run"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}

#[test]
#[cfg(unix)]
fn private_encrypt_ambiguous_key_error_keeps_prompt_recoverable() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = temp.path().join("ambiguous-gpg");
    write_executable_file(
        &fake_gpg,
        "#!/bin/sh\nif [ \"$1\" = \"--batch\" ]; then\n  printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n  printf '%s\\n' 'fpr:::::::::100EB0696C6C86561B72BDE1F707666666666666:'\n  printf '%s\\n' 'uid:u::::1::hash::Test User <test@example.invalid>::::::::::0:'\n  printf '%s\\n' 'pub:u:255:22:2222222222222222:1:::u:::scESC::::::23::0:'\n  printf '%s\\n' 'fpr:::::::::76A4ACC1535D1048A2F58E7F00AA33AA12345678:'\n  printf '%s\\n' 'uid:u::::1::hash::Test User <test@example.invalid>::::::::::0:'\n  exit 0\nfi\nexit 9\n",
    );
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt on test@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Error: GPG key selector is ambiguous"));
    assert!(output.contains("100EB0696C6C86561B72BDE1F707666666666666"));
    assert!(output.contains("76A4ACC1535D1048A2F58E7F00AA33AA12345678"));
    assert!(!state.exit_requested);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert!(!state.encryption_config.enabled);
}

#[test]
fn encrypt_unlock_mode_persists_startup_unlock_policy() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        (
            "#encrypt unlock-mode prompt",
            "encryption.startup_unlock=prompt",
        ),
        (
            "#encrypt unlock-mode lazy",
            "encryption.startup_unlock=lazy",
        ),
    ] {
        state.draft.insert_str(line);
        let mut output = Vec::new();
        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
    }

    assert_eq!(
        config::load_config(&config_path)
            .unwrap()
            .encryption
            .startup_unlock,
        config::EncryptionStartupUnlockMode::Lazy
    );
}

#[test]
#[cfg(unix)]
fn encrypt_on_migrates_plaintext_storage_and_persists_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let ai_path = temp.path().join("history/ai.jsonl");
    let draft_path = temp.path().join("history/draft.jsonl");
    let notes_path = temp.path().join("history/notes.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    append_jsonl(
        &regular_path,
        &HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    append_jsonl(
        &ai_path,
        &AiSession {
            id: "ai-1".to_string(),
            t: 2,
            prompt: "list".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ls".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();
    append_jsonl(
        &draft_path,
        &DraftEntry {
            t: 3,
            text: "draft".to_string(),
        },
    )
    .unwrap();
    append_jsonl(
        &notes_path,
        &NoteEntry {
            tag: crate::commands::NoteTag::Note,
            text: "note".to_string(),
        },
    )
    .unwrap();
    append_template(&template_path, &TemplateEntry::new("echo {message}")).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        ai_history_path: Some(ai_path.clone()),
        draft_history_path: Some(draft_path.clone()),
        notes_path: Some(notes_path.clone()),
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt on test@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Encryption is now enabled for future writes."));
    assert!(output.contains("encryption=on"));
    assert!(state.encryption_config.enabled);
    assert_eq!(
        state.encryption_config.key_fingerprint,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    assert_eq!(state.encryption_config.recipient, "");
    let loaded = config::load_config(&config_path).unwrap();
    assert!(loaded.encryption.enabled);
    assert_eq!(
        loaded.encryption.key_fingerprint,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    assert_eq!(loaded.encryption.recipient, "");
    for path in [
        &regular_path,
        &ai_path,
        &draft_path,
        &notes_path,
        &template_path,
    ] {
        assert!(!path.exists(), "plaintext remained: {}", path.display());
        assert!(
            crate::encryption::encrypted_path(path).exists(),
            "encrypted file missing: {}",
            path.display()
        );
    }
}

#[test]
#[cfg(unix)]
fn encrypt_on_restores_plaintext_storage_when_migration_fails() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_failing_ai_encrypt_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let ai_path = temp.path().join("history/ai.jsonl");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    append_jsonl(
        &regular_path,
        &HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    append_jsonl(
        &ai_path,
        &AiSession {
            id: "ai-1".to_string(),
            t: 2,
            prompt: "list".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ls".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        ai_history_path: Some(ai_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt on test@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Error: GPG encryption failed"));
    assert!(regular_path.exists());
    assert!(ai_path.exists());
    assert!(!crate::encryption::encrypted_path(&regular_path).exists());
    assert!(!crate::encryption::encrypted_path(&ai_path).exists());
    let loaded = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
    assert_eq!(loaded.items[0].command, "pwd");
    let ai_loaded = load_jsonl::<AiSession>(&ai_path).unwrap();
    assert_eq!(ai_loaded.items[0].prompt, "list");
}

#[test]
#[cfg(unix)]
fn encrypt_rotate_reencrypts_existing_storage_and_persists_fingerprint() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let key_path = temp.path().join("secrets/key.json.gpg");
    let mut config = config::Config::default();
    config.encryption.enabled = true;
    config.encryption.key_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
    config::save_config(&config_path, &config).unwrap();
    rewrite_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &regular_path,
        &[HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
    )
    .unwrap();
    atomic_gpg_encrypt_bytes(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &key_path,
        br#"{"env_key":"AISH_TEST_API_KEY","value":"secret-test-key"}"#,
    )
    .unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        secret_key_path: Some(key_path.clone()),
        encryption_config: config.encryption,
        ..AppState::default()
    };
    state
        .draft
        .insert_str("#encrypt rotate second@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("reencrypted_files=1"));
    assert_eq!(
        state.encryption_config.key_fingerprint,
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    );
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(
        loaded.encryption.key_fingerprint,
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    );
    let encrypted = fs::read_to_string(crate::encryption::encrypted_path(&regular_path)).unwrap();
    assert!(encrypted.starts_with("recipient:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB\n"));
    let encrypted_key = fs::read_to_string(&key_path).unwrap();
    assert!(encrypted_key.starts_with("recipient:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB\n"));
    let decrypted_key = gpg_decrypt_file(fake_gpg.display().to_string(), &key_path).unwrap();
    let record: StoredApiKey = serde_json::from_slice(&decrypted_key).unwrap();
    assert_eq!(record.value, "secret-test-key");
    assert!(!regular_path.exists());
}

#[test]
#[cfg(unix)]
fn encrypted_completion_uses_cached_templates_without_gpg_on_keypress() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = temp.path().join("fail-gpg");
    write_executable_file(
        &fake_gpg,
        "#!/bin/sh\nprintf 'unexpected gpg call\\n' >&2\nexit 9\n",
    );
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let mut state = AppState {
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        templates: vec![TemplateEntry::new("git add . && git commit")],
        ..AppState::default()
    };
    state.draft.insert_str("git");

    let candidates = state.completion_candidates().unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert_eq!(candidates[0].display, "git add . && git commit");
}

#[test]
#[cfg(unix)]
fn encrypted_history_append_does_not_block_command_completion() {
    let temp = tempfile::tempdir().unwrap();
    let release_path = temp.path().join("release-gpg");
    let fake_gpg = write_blocking_fake_gpg(&temp, &release_path);
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut cache = HashMap::new();
    cache.insert(regular_path.clone(), Vec::new());
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_writer: Some(EncryptedWriteQueue::start(
            fake_gpg.display().to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            cache,
        )),
        ..AppState::default()
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let completed_quickly = std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = record_completed_command(
                &mut state,
                "echo async-encrypted-history".to_string(),
                "async-encrypted-history\n".to_string(),
                0,
                false,
            )
            .map_err(|error| error.to_string());
            done_tx.send(result).unwrap();
        });
        match done_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(result) => {
                result.unwrap();
                true
            }
            Err(_) => {
                fs::write(&release_path, b"go\n").unwrap();
                let result = done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("record_completed_command stayed blocked");
                result.unwrap();
                false
            }
        }
    });
    assert!(
        completed_quickly,
        "encrypted append blocked command completion"
    );
    assert_eq!(state.regular_history.len(), 1);
    assert!(
        !crate::encryption::encrypted_path(&regular_path).exists(),
        "background GPG finished before it was released"
    );

    fs::write(&release_path, b"go\n").unwrap();
    state.flush_encrypted_writes().unwrap();
    assert!(state.drain_encrypted_write_events());
    let loaded =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "echo async-encrypted-history");
}

#[test]
#[cfg(unix)]
fn encrypted_history_trim_refreshes_writer_cache_before_next_append() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fail_decrypt_marker = temp.path().join("fail-decrypt");
    let fake_gpg = write_decrypt_marker_fake_gpg(&temp, &fail_decrypt_marker);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let regular_path = temp.path().join("history/regular.jsonl");
    let ai_path = temp.path().join("history/ai.jsonl");
    rewrite_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &regular_path,
        &[
            HistoryEntry {
                t: 1,
                command: "old".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "keep".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
    )
    .unwrap();
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        ai_history_path: Some(ai_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_writer: Some(EncryptedWriteQueue::start(
            fake_gpg.display().to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            HashMap::new(),
        )),
        ..AppState::default()
    };

    trim_history_for_state(&state, 1).unwrap();
    fs::write(&fail_decrypt_marker, b"fail future decrypts\n").unwrap();
    record_completed_command(&mut state, "new".to_string(), String::new(), 0, false).unwrap();
    state.flush_encrypted_writes().unwrap();
    fs::remove_file(&fail_decrypt_marker).unwrap();
    let loaded =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].command, "keep");
    assert_eq!(loaded.items[1].command, "new");
}
