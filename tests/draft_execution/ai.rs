use super::*;

#[test]
fn execute_ai_editor_draft_submits_as_ai_prompt_not_shell_text() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();
    let temp = tempfile::tempdir().unwrap();
    let session = prepare_editor_file(temp.path(), "explain this").unwrap();

    state
        .replace_draft_from_ai_prompt_editor_session(&session)
        .unwrap();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("AI request failed"));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert!(!state.draft_from_editor);
    assert!(!state.draft_from_ai_editor);
}

#[test]
fn execute_ai_prompt_switches_to_new_ai_session_and_reports_new_item_count() {
    let _guard = pty_execution_guard();
    let mut state = AppState {
        ai_requester: fake_echo_message_ai_items,
        ai_sessions: vec![AiSession {
            id: "old".to_string(),
            t: 1,
            prompt: "old".to_string(),
            ctx: false,
            model: "test-model".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "echo hello".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        selected_ai_index: Some(0),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("# how to echo something?");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("AI items generated: 1"),
        "output was {output:?}"
    );
    assert_eq!(state.mode, Mode::Ai);
    assert!(state.draft.is_empty());
    assert_eq!(state.ai_sessions.len(), 2);
    assert_eq!(state.ai_command_indices.len(), 2);
    assert_eq!(state.selected_ai_index, Some(1));
    assert_eq!(state.selected_ai_command(), Some("echo {message}"));
}

#[test]
fn executed_draft_can_be_restored_from_draft_history_after_blank_prompt() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("printf 'recoverable draft\\n'");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);

    state.move_draft_selection_older().unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "printf 'recoverable draft\\n'");
    assert_eq!(state.selected_draft_index, Some(0));
}

#[test]
fn execute_ai_selection_success_advances_to_next_command() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = ai_state(
        &["printf 'first ai\\n'", "printf 'second ai\\n'"],
        0,
        history_path.clone(),
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

    assert!(String::from_utf8(output).unwrap().contains("first ai"));
    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_index, Some(1));
    assert_eq!(state.selected_ai_command(), Some("printf 'second ai\\n'"));

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].source, HistorySource::Ai);
    assert_eq!(state.regular_history[0].source, HistorySource::Ai);
}

#[test]
fn execute_ai_selection_failure_stays_on_current_command() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = ai_state(&["false", "printf 'next ai\\n'"], 0, history_path);
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let _output = String::from_utf8(output).unwrap();
    assert_eq!(state.last_status, Some(1));
    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_index, Some(0));
    assert_eq!(state.selected_ai_command(), Some("false"));
}

#[test]
fn execute_ai_selection_last_success_returns_to_draft() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = ai_state(&["printf 'last ai\\n'"], 0, history_path);
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().contains("last ai"));
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.selected_ai_index, None);
    assert!(state.draft.is_empty());
}
