use super::*;

#[test]
fn private_exit_requests_app_exit() {
    let mut state = AppState::default();
    state.draft.insert_str("#exit");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.exit_requested);
    assert!(state.draft.is_empty());
    assert!(output.is_empty());
}

#[test]
fn ordinary_shell_exit_requests_clean_app_exit() {
    let mut state = AppState::default();
    state.draft.insert_str("exit");
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
    assert!(state.exit_requested);
    assert!(state.draft.is_empty());
    assert!(!output.contains("backend shell PTY closed"));
}

#[test]
fn editor_draft_private_command_uses_aish_parser() {
    let mut state = AppState::default();
    state.draft.insert_str("#status");
    state.draft_from_editor = true;
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
    assert!(output.contains("Aish status"));
    assert!(state.draft.is_empty());
    assert!(!state.draft_from_editor);
}

#[test]
fn editor_draft_shell_command_is_saved_to_regular_history() {
    let temp = tempfile::tempdir().unwrap();
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("printf 'editor-history\\n'");
    state.draft_from_editor = true;
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "printf 'editor-history\\n'");
    assert_eq!(loaded.items[0].exit_code, Some(0));
}

#[test]
fn editor_draft_still_checks_shell_continuation() {
    let mut state = AppState::default();
    state.draft.insert_str("printf 'unterminated");
    state.draft_from_editor = true;
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.as_str().ends_with('\n'));
    assert!(state.continuation_prompt.is_some());
    assert!(String::from_utf8(output).unwrap().is_empty());
}

#[test]
fn private_help_prints_available_commands() {
    let mut state = AppState::default();
    state.draft.insert_str("#help");
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
    assert!(output.contains("Aish help"));
    assert!(output.contains("Usage:"));
    assert!(output.contains("#help [topic]"));
    assert!(output.contains("Topics:"));
    assert!(
        output
            .contains("commands, keys, ai, paste, completion, templates, sync, encryption, config")
    );
    assert!(output.contains("Private commands:"));
    assert!(output.contains("#help"));
    assert!(output.contains("#status"));
    assert!(output.contains("#config"));
    assert!(output.contains("#doctor"));
    assert!(output.contains("#prompt"));
    assert!(output.contains("#model"));
    assert!(output.contains("#base-url"));
    assert!(output.contains("#env-key"));
    assert!(output.contains("#key"));
    assert!(output.contains("#context"));
    assert!(output.contains("#paste"));
    assert!(output.contains("#completion"));
    assert!(output.contains("#log"));
    assert!(output.contains("#editor"));
    assert!(output.contains("#mt"));
    assert!(output.contains("#template"));
    assert!(output.contains("#encrypt"));
    assert!(output.contains("#set-remote"));
    assert!(output.contains("#sync"));
    assert!(!output.contains("#push"));
    assert!(output.contains("#exit"));
    assert!(output.contains("#quit"));
    assert!(output.contains("#history"));
    assert!(output.contains("Keybindings:"));
    assert!(
        output.contains(
            "Tab - empty draft cycles modes; non-empty draft shows or accepts completion"
        )
    );
    assert!(output.contains("Ctrl-X Ctrl-E - open the configured external editor"));
    assert!(output.contains("AI and notes:"));
    assert!(output.contains("# <prompt> - send an AI prompt"));
    assert!(output.contains("# TODO: <text> - store a note"));
    assert!(state.draft.is_empty());
}

#[test]
fn removed_duplicate_private_commands_do_not_dispatch() {
    for (line, expected) in [
        ("#push", "Aish command not implemented yet: #push"),
        ("#sync union", "usage: #sync resolve-union"),
        ("#template pending shared", "usage: #template"),
        (
            "#template publish shared --plain",
            "usage: #template publish <name> [--encrypt <key>]",
        ),
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
fn private_help_prints_topic_specific_usage() {
    let mut state = AppState::default();
    state.draft.insert_str("#help completion");
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
    assert!(output.contains("Completion help"));
    assert!(output.contains("#completion mode auto|tab|off"));
    assert!(output.contains("#completion display-delay-ms <0-1000>"));
    assert!(output.contains("#completion tab-accept full|word"));
    assert!(output.contains("auto shows live hints while typing"));
    assert!(!output.contains("Sync help"));
}

#[test]
fn private_help_rejects_unknown_topic_without_running_shell() {
    let mut state = AppState::default();
    state.draft.insert_str("#help unknown-topic");
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
    assert!(output.contains("unknown help topic: unknown-topic"));
    assert!(output.contains(
        "usage: #help [commands|keys|ai|paste|completion|templates|sync|encryption|config]"
    ));
    assert!(state.draft.is_empty());
}

#[test]
fn private_context_reports_current_config() {
    let mut state = AppState::default();
    state.draft.insert_str("#context");
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
    assert!(output.contains("context.enabled=true"));
    assert!(output.contains("context.confirm=true"));
    assert!(output.contains("context.max_bytes=65536"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}
