use super::*;

#[test]
fn execute_draft_appends_successful_command_to_regular_history() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("printf 'stored\\n'");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "printf 'stored\\n'");
    assert_eq!(loaded.items[0].t, 1234567890);
    assert_eq!(loaded.items[0].exit_code, Some(0));
    assert_eq!(loaded.items[0].source, HistorySource::User);
    assert_eq!(state.regular_history, loaded.items);
}

#[test]
fn execute_draft_appends_failed_command_to_regular_history() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
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

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "false");
    assert_eq!(loaded.items[0].t, 1234567890);
    assert_eq!(loaded.items[0].exit_code, Some(1));
    assert_eq!(loaded.items[0].source, HistorySource::User);
}

#[test]
fn execute_draft_stores_notes_without_sending_them_to_shell() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let notes_path = temp.path().join("history/notes.jsonl");
    let mut state = AppState {
        notes_path: Some(notes_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.draft.insert_str("# TODO: deploy later");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("note stored"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());

    let loaded = load_jsonl::<NoteEntry>(&notes_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].tag, NoteTag::Todo);
    assert_eq!(loaded.items[0].text, "deploy later");
}

#[test]
fn private_history_command_trims_regular_and_ai_history_to_combined_limit() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let ai_history_path = temp.path().join("history/ai.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        ai_history_path: Some(ai_history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    for command in ["printf 'one\\n'", "printf 'two\\n'", "printf 'three\\n'"] {
        state.draft.clear();
        state.draft.insert_str(command);
        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();
    }

    append_jsonl(
        &ai_history_path,
        &AiSession {
            id: "a_1".to_string(),
            t: 4,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "printf 'ai\\n'".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();
    state.ai_sessions = load_jsonl::<AiSession>(&ai_history_path).unwrap().items;
    state.ai_command_indices = vec![AiCommandIndex {
        session_index: 0,
        item_index: 0,
    }];

    state.draft.clear();
    state.draft.insert_str("#history 2");
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    let loaded_ai = load_jsonl::<AiSession>(&ai_history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].command, "printf 'two\\n'");
    assert_eq!(loaded.items[1].command, "printf 'three\\n'");
    assert!(loaded_ai.items.is_empty());

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("history trimmed to 2"));
    assert_eq!(state.regular_history.len(), 2);
    assert!(state.ai_sessions.is_empty());
    assert!(state.ai_command_indices.is_empty());
    assert_eq!(state.regular_history[0].command, "printf 'two\\n'");
    assert_eq!(state.regular_history[1].command, "printf 'three\\n'");
}

#[test]
fn execute_history_selection_runs_selected_command() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        mode: Mode::History,
        regular_history_path: Some(history_path.clone()),
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "printf 'from history\\n'".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        selected_history_index: Some(0),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(String::from_utf8(output).unwrap().contains("from history"));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.last_status, Some(0));

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "printf 'from history\\n'");
    assert_eq!(state.regular_history.len(), 2);
}
