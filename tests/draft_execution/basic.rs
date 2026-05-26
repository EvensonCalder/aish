use super::*;

#[test]
fn execute_draft_sends_command_to_backend_and_opens_blank_draft() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("printf 'hello draft\\n'");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().contains("hello draft"));
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft_history.len(), 1);
    assert_eq!(state.draft_history[0].text, "printf 'hello draft\\n'");
    assert_eq!(state.output_ring.len(), 1);
    assert_eq!(state.output_ring[0].command, "printf 'hello draft\\n'");
    assert!(state.output_ring[0].output.contains("hello draft"));
    assert_eq!(state.output_ring[0].exit_code, 0);
}

#[test]
fn execute_draft_user_command_waits_for_backend_ready_without_command_timeout() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state
        .draft
        .insert_str("sleep 0.2; printf 'after-backend-wait\\n'");

    let started = Instant::now();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_millis(50),
    )
    .unwrap();

    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("after-backend-wait")
    );
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
}

#[test]
fn execute_draft_records_failed_status_and_returns_to_draft() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("false");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let _output = String::from_utf8(output).unwrap();
    assert_eq!(state.last_status, Some(1));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft_history.len(), 1);
    assert_eq!(state.draft_history[0].text, "false");
}

#[test]
fn execute_draft_updates_current_cwd_from_backend_shell() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state
        .draft
        .insert_str(&format!("cd {}", temp.path().display()));

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.current_cwd.as_deref(), Some(temp.path()));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
}

#[test]
fn execute_draft_keeps_unfinished_quote_as_continuation_draft() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("echo \"");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().is_empty());
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo \"\n");
    assert_eq!(state.continuation_prompt.as_deref(), Some("dquote> "));

    let after = backend
        .run_command("printf 'backend-ok\\n'", Duration::from_secs(5))
        .unwrap();
    assert_eq!(after.output.trim(), "backend-ok");
}

#[test]
fn execute_draft_keeps_unfinished_single_quote_as_continuation_draft() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("echo '");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().is_empty());
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo '\n");
    assert_eq!(state.continuation_prompt.as_deref(), Some("quote> "));
}

#[test]
fn execute_draft_runs_completed_multiline_quote_after_continuation() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("echo \"\n123\n\"");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().contains("123"));
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert!(state.continuation_prompt.is_none());
}

#[test]
fn execute_draft_sends_multiline_buffer_exactly_to_backend() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("printf 'one\\n'\nprintf 'two\\n'");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().contains("one\r\ntwo"));
    assert_eq!(state.output_ring.back().unwrap().output, "one\ntwo\n");
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
}

#[test]
fn execute_draft_preserves_backslash_continuation_and_history() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let command = "printf '%s\\n' \\\n+draft-continuation";
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str(command);

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("draft-continuation"));
    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, command);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft_history.len(), 1);
    assert_eq!(state.draft_history[0].text, command);
}

#[test]
fn execute_draft_does_not_send_line_leading_hash_to_backend_shell() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("#definitely-not-a-shell-comment");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("unknown Aish command"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}
