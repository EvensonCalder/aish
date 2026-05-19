use super::*;

#[test]
fn encrypt_rewrite_history_plan_reports_manual_confirmed_flow() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut state = AppState {
        config_path: Some(config_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt rewrite-history plan");
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
    assert!(output.contains("history rewrite plan"));
    assert!(output.contains("risk=rewrites commit ids"));
    assert!(
        output.contains(
            "next=#encrypt rewrite-history run <key-fingerprint> --confirm-rewrite-history"
        )
    );
}

#[test]
fn encrypt_rewrite_history_script_keeps_decrypted_temp_outside_rewrite_tree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let state = AppState {
        regular_history_path: Some(root.join("history/regular.jsonl")),
        ..AppState::default()
    };

    let script_path = write_history_rewrite_script(root, &state).unwrap();
    let script = fs::read_to_string(&script_path).unwrap();

    assert!(script.contains("mktemp -d"));
    assert!(script.contains("tmp=\"$tmp_dir/plain\""));
    assert!(script.contains("trap cleanup EXIT HUP INT TERM"));
    assert!(!script.contains("$enc.plain"));
    assert!(!script.contains(".plain.$$"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&script_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn encrypt_rewrite_history_run_requires_clean_git_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    run_test_git(root, ["init"]);
    fs::write(root.join("dirty.txt"), "uncommitted").unwrap();
    let config_path = root.join("config.toml");
    let mut state = AppState {
        config_path: Some(config_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str(
            "#encrypt rewrite-history run BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB --confirm-rewrite-history",
        );
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
    assert!(output.contains("history rewrite requires a clean git worktree"));
}

#[test]
#[cfg(unix)]
fn encrypted_writes_use_gpg_files_without_plaintext_jsonl() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let regular_path = temp.path().join("history/regular.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        template_store_path: Some(template_path.clone()),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    state.draft.insert_str("echo encrypted-history");
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();
    state.draft.insert_str("#mt echo encrypted-template");
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded_history =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();
    let loaded_templates =
        load_encrypted_jsonl::<TemplateEntry>(fake_gpg.display().to_string(), &template_path)
            .unwrap();
    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert!(!regular_path.exists());
    assert!(!template_path.exists());
    assert!(crate::encryption::encrypted_path(&regular_path).exists());
    assert!(crate::encryption::encrypted_path(&template_path).exists());
    assert_eq!(loaded_history.items.len(), 1);
    assert_eq!(loaded_history.items[0].command, "echo encrypted-history");
    assert_eq!(loaded_templates.items.len(), 1);
    assert_eq!(loaded_templates.items[0].body, "echo encrypted-template");
}

#[test]
#[cfg(unix)]
fn startup_unlock_noninteractive_loads_cached_agent_data() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let paths = EncryptedStartupPaths {
        regular_history: temp.path().join("history/regular.jsonl"),
        draft_history: temp.path().join("history/draft.jsonl"),
        ai_history: temp.path().join("history/ai.jsonl"),
        notes: temp.path().join("history/notes.jsonl"),
        template_store: temp.path().join("templates/templates.jsonl"),
    };
    append_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &paths.regular_history,
        &HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    append_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &paths.template_store,
        &TemplateEntry::new("echo cached"),
    )
    .unwrap();

    let data = load_encrypted_startup_data(&paths, UnlockMode::Noninteractive).unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert_eq!(data.store.regular.len(), 1);
    assert_eq!(data.store.regular[0].command, "pwd");
    assert_eq!(data.templates.items.len(), 1);
    assert_eq!(data.templates.items[0].body, "echo cached");
    assert!(data.encrypted_cache.contains_key(&paths.regular_history));
    assert!(data.encrypted_cache.contains_key(&paths.template_store));
}

#[test]
#[cfg(unix)]
fn locked_encrypted_storage_buffers_history_until_unlock() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let regular_path = temp.path().join("history/regular.jsonl");
    let draft_path = temp.path().join("history/draft.jsonl");
    let ai_path = temp.path().join("history/ai.jsonl");
    let notes_path = temp.path().join("history/notes.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    append_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &regular_path,
        &HistoryEntry {
            t: 1,
            command: "old".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        draft_history_path: Some(draft_path),
        ai_history_path: Some(ai_path),
        notes_path: Some(notes_path),
        template_store_path: Some(template_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_storage_unlocked: false,
        ..AppState::default()
    };

    record_completed_command(&mut state, "new".to_string(), String::new(), 0, false).unwrap();

    assert!(state.encrypted_storage_is_locked());
    assert_eq!(state.pending_locked_regular_history.len(), 1);
    assert!(state.encrypted_writer.is_none());

    assert!(state.unlock_encrypted_storage_interactively().unwrap());
    state.flush_encrypted_writes().unwrap();
    let loaded =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert!(state.encrypted_storage_unlocked);
    assert!(state.pending_locked_regular_history.is_empty());
    assert_eq!(state.regular_history.len(), 2);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].command, "old");
    assert_eq!(loaded.items[1].command, "new");
}

#[test]
fn locked_history_mode_renders_unlock_message() {
    let state = AppState {
        mode: Mode::History,
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_storage_unlocked: false,
        ..AppState::default()
    };

    assert_eq!(
        state.render_prompt_line(),
        "$ history is still unlocking..."
    );
    assert_eq!(
        state.terminal_cursor_column(),
        display_width("$ history is still unlocking...") as u16
    );
}

#[test]
#[cfg(unix)]
fn key_set_encrypts_env_api_key_without_printing_secret() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
        std::env::set_var("AISH_TEST_API_KEY", "secret-test-key");
    }
    let key_path = temp.path().join("secrets/key.json.gpg");
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        secret_key_path: Some(key_path.clone()),
        events_path: Some(events_path.clone()),
        ai_config: AiConfig {
            env_key: "AISH_TEST_API_KEY".to_string(),
            ..AiConfig::default()
        },
        encryption_config: EncryptionConfig {
            enabled: false,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#key set");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let decrypted = gpg_decrypt_file(fake_gpg.display().to_string(), &key_path).unwrap();
    let record: StoredApiKey = serde_json::from_slice(&decrypted).unwrap();
    unsafe {
        std::env::remove_var("AISH_GPG");
        std::env::remove_var("AISH_TEST_API_KEY");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("stored key encrypted"));
    assert!(!output.contains("secret-test-key"));
    assert_eq!(record.env_key, "AISH_TEST_API_KEY");
    assert_eq!(record.value, "secret-test-key");
    assert!(key_path.exists());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "stored key encrypted");
}

#[test]
#[cfg(unix)]
fn ai_prompt_uses_gpg_stored_key_when_env_key_is_missing() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
        std::env::set_var("AISH_TEST_API_KEY", "secret-test-key");
    }
    let key_path = temp.path().join("secrets/key.json.gpg");
    let mut state = AppState {
        secret_key_path: Some(key_path),
        ai_config: AiConfig {
            model: "test-model".to_string(),
            base_url: "https://example.invalid/v1/chat/completions".to_string(),
            env_key: "AISH_TEST_API_KEY".to_string(),
            ..AiConfig::default()
        },
        ai_requester: ai_requester_requires_stored_key,
        encryption_config: EncryptionConfig {
            enabled: false,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#key set");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();
    unsafe {
        std::env::remove_var("AISH_TEST_API_KEY");
    }
    state.draft.insert_str("# list files");
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
    assert!(output.contains("AI items generated: 1"));
    assert_eq!(state.ai_sessions.len(), 1);
    assert_eq!(state.ai_sessions[0].items[0].text, "pwd");
}

#[test]
#[cfg(unix)]
fn encrypt_off_decrypts_storage_and_persists_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut config = config::Config::default();
    config.encryption.enabled = true;
    config.encryption.key_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
    config::save_config(&config_path, &config).unwrap();
    rewrite_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "test@example.invalid",
        &regular_path,
        &[HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
    )
    .unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        encryption_config: config.encryption,
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt off");
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
    assert!(output.contains("encryption=off"));
    assert!(!state.encryption_config.enabled);
    assert!(
        !config::load_config(&config_path)
            .unwrap()
            .encryption
            .enabled
    );
    assert!(regular_path.exists());
    assert!(!crate::encryption::encrypted_path(&regular_path).exists());
    let loaded = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
    assert_eq!(loaded.items[0].command, "pwd");
}
