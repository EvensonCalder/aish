use super::*;

#[test]
fn private_context_commands_persist_config() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#context off", "context.enabled=false"),
        ("#context confirm off", "context.confirm=false"),
        ("#context 1024", "context.max_bytes=1024"),
        ("#context on", "context.enabled=true"),
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
        assert!(state.draft.is_empty());
    }

    assert!(state.context_config.enabled);
    assert!(!state.context_config.confirm);
    assert_eq!(state.context_config.max_bytes, 1024);
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.context, state.context_config);
}

#[test]
fn private_context_rejects_invalid_usage_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#context 0");
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
    assert!(output.contains("context max bytes must be greater than 0"));
    assert_eq!(state.context_config, ContextConfig::default());
    assert_eq!(
        config::load_config(&config_path).unwrap().context,
        ContextConfig::default()
    );
}

#[test]
fn private_paste_reports_current_config() {
    let mut state = AppState::default();
    state.draft.insert_str("#paste");
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
    assert!(output.contains("paste.multiline=editor"));
    assert!(output.contains("paste.confirm_execute=true"));
    assert!(output.contains("paste.preview=true"));
    assert!(output.contains("paste.preview_lines=3"));
    assert!(output.contains("paste.preview_bytes=240"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn private_paste_commands_persist_config() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#paste multiline execute", "paste.multiline=execute"),
        ("#paste confirm off", "paste.confirm_execute=false"),
        ("#paste preview off", "paste.preview=false"),
        ("#paste preview-lines 5", "paste.preview_lines=5"),
        ("#paste preview-bytes 512", "paste.preview_bytes=512"),
        ("#paste confirm-execute on", "paste.confirm_execute=true"),
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
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.paste_config.multiline, "execute");
    assert!(state.paste_config.confirm_execute);
    assert!(!state.paste_config.preview);
    assert_eq!(state.paste_config.preview_lines, 5);
    assert_eq!(state.paste_config.preview_bytes, 512);
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.paste, state.paste_config);
}

#[test]
fn private_paste_rejects_invalid_usage_without_persisting() {
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
            "#paste multiline raw",
            "usage: #paste multiline editor|execute|discard",
        ),
        ("#paste preview maybe", "usage: #paste preview on|off"),
        (
            "#paste preview-lines 0",
            "paste preview lines must be between 1 and 20",
        ),
        (
            "#paste preview-lines nope",
            "usage: #paste preview-lines <1-20>",
        ),
        (
            "#paste preview-bytes 4097",
            "paste preview bytes must be between 1 and 4096",
        ),
        (
            "#paste preview-bytes nope",
            "usage: #paste preview-bytes <1-4096>",
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
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.paste_config, config::PasteConfig::default());
    assert_eq!(
        config::load_config(&config_path).unwrap().paste,
        config::PasteConfig::default()
    );
}

#[test]
fn ai_prompt_with_context_waits_for_confirmation_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("# explain < printf context");
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
    assert!(output.contains("aish will run this command to collect context"));
    assert!(output.contains("Run context command? [Y/n]"));
    assert!(output.contains("answer Y to run context command or n to skip"));
    assert_eq!(
        state.pending_context,
        Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: false,
        })
    );
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "context command requires confirmation");
}

#[test]
fn ai_prompt_with_context_disabled_does_not_execute_command() {
    let mut state = AppState {
        context_config: ContextConfig {
            enabled: false,
            ..ContextConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("# explain < printf context");
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
    assert!(output.contains("context collection is disabled"));
    assert!(output.contains("context command not executed: printf context"));
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
}

#[test]
fn ai_prompt_with_context_blocks_dangerous_command_even_without_confirmation() {
    let mut state = AppState {
        context_config: ContextConfig {
            confirm: false,
            ..ContextConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("# explain < rm -rf /tmp/aish-test");
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
    assert!(output.contains("dangerous context command requires confirmation"));
    assert_eq!(
        state.pending_context,
        Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "rm -rf /tmp/aish-test".to_string(),
            dangerous: true,
        })
    );
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
}

#[test]
fn answer_context_confirmation_can_skip_pending_command() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        events_path: Some(events_path.clone()),
        pending_context: Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: false,
        }),
        ..AppState::default()
    };
    let mut output = Vec::new();

    answer_context_confirmation(&mut state, false, &mut output, Duration::from_secs(5)).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("context command skipped: printf context"));
    assert_eq!(state.pending_context, None);
    assert!(state.ai_sessions.is_empty());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "context command skipped");
}

#[test]
fn private_log_prints_recent_events() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    append_event(&events_path, 1, EventLevel::Info, "one", DEFAULT_MAX_EVENTS).unwrap();
    append_event(&events_path, 2, EventLevel::Warn, "two", DEFAULT_MAX_EVENTS).unwrap();
    let mut state = AppState {
        events_path: Some(events_path),
        ..AppState::default()
    };
    state.draft.insert_str("#log 1");
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
    assert!(!output.contains("one"));
    assert!(output.contains("2\tWarn\ttwo"));
}

#[test]
fn private_log_reports_usage_or_missing_storage() {
    for (line, expected) in [
        ("#log", "usage: #log <count>"),
        ("#log nope", "usage: #log <count>"),
        ("#log 1", "event log storage is not configured"),
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
    }
}

#[test]
fn ai_config_commands_persist_and_report_values() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#model test-model", "#model=test-model"),
        (
            "#base-url https://example.invalid/v1",
            "#base-url=https://example.invalid/v1/chat/completions",
        ),
        ("#env-key OPENAI_API_KEY", "#env-key=OPENAI_API_KEY"),
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
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.ai_config.model, "test-model");
    assert_eq!(
        state.ai_config.base_url,
        "https://example.invalid/v1/chat/completions"
    );
    assert_eq!(state.ai_config.env_key, "OPENAI_API_KEY");
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.ai, state.ai_config);
}

#[test]
fn ai_env_key_rejects_invalid_shell_name_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut config = config::Config::default();
    config.ai.env_key = "OPENAI_API_KEY".to_string();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ai_config: config.ai.clone(),
        ..AppState::default()
    };
    state.draft.insert_str("#env-key BAD-NAME");
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
    assert!(output.contains("must be a valid shell variable name"));
    assert_eq!(state.ai_config.env_key, "OPENAI_API_KEY");
    assert_eq!(
        config::load_config(&config_path).unwrap().ai.env_key,
        "OPENAI_API_KEY"
    );
}

#[test]
fn ai_config_commands_report_unconfigured_without_config_path() {
    for (line, expected) in [
        ("#model", "#model=unconfigured"),
        ("#base-url", "#base-url=unconfigured"),
        ("#env-key", "#env-key=unconfigured"),
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
        assert!(state.draft.is_empty());
    }
}

#[test]
fn ai_config_write_errors_are_logged() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("bad-config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::write(&config_path, "not = [valid").unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#model test-model");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Error: invalid config")
    );
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 2);
    assert_eq!(events.items[0].level, EventLevel::Error);
    assert_eq!(events.items[0].msg, "config error");
    assert_eq!(events.items[1].level, EventLevel::Error);
    assert_eq!(events.items[1].msg, "private command failed");
    assert!(state.draft.is_empty());
}

#[test]
fn context_config_write_errors_are_logged() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("bad-config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::write(&config_path, "not = [valid").unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#context off");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Error: invalid config")
    );
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 2);
    assert_eq!(events.items[0].level, EventLevel::Error);
    assert_eq!(events.items[0].msg, "config error");
    assert_eq!(events.items[1].level, EventLevel::Error);
    assert_eq!(events.items[1].msg, "private command failed");
    assert!(state.draft.is_empty());
}

#[test]
fn key_commands_report_current_state_without_secret_side_effects() {
    for (line, expected) in [
        ("#key set", "key storage is not configured; no key stored"),
        (
            "#key clear",
            "key storage is not configured; no key removed",
        ),
        ("#key", "usage: #key set | #key clear"),
        ("#key rotate", "usage: #key set | #key clear"),
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
fn key_clear_removes_stored_encrypted_key_and_logs_event() {
    let temp = tempfile::tempdir().unwrap();
    let key_path = temp.path().join("secrets/key.json.gpg");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
    std::fs::write(&key_path, b"encrypted-key-placeholder").unwrap();
    let mut state = AppState {
        secret_key_path: Some(key_path.clone()),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#key clear");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(!key_path.exists());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("stored key cleared")
    );
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 1);
    assert_eq!(events.items[0].level, EventLevel::Info);
    assert_eq!(events.items[0].msg, "stored key cleared");
}

#[test]
fn subsystem_commands_report_current_state() {
    for (line, expected) in [
        ("#completion", "completion.mode=auto"),
        ("#completion", "completion.max_results=5"),
        ("#completion", "completion.enabled=true"),
        ("#completion", "completion.coalesce_ms=50"),
        ("#completion", "completion.display_delay_ms=120"),
        ("#completion", "completion.fuzzy=true"),
        ("#editor", "editor temp directory is not configured"),
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
