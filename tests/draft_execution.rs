use std::time::Duration;

use aish::app::{AppState, execute_draft};
use aish::history::{HistoryEntry, HistorySource, load_jsonl};
use aish::modes::Mode;
use aish::pty::PtyBackend;

#[test]
fn execute_draft_sends_command_to_backend_and_resets_state() {
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

    assert_eq!(String::from_utf8(output).unwrap().trim(), "hello draft");
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn execute_draft_records_failed_status_and_returns_to_draft() {
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

    assert!(String::from_utf8(output).unwrap().trim().is_empty());
    assert_eq!(state.last_status, Some(1));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn execute_draft_sends_multiline_buffer_exactly_to_backend() {
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

    assert_eq!(String::from_utf8(output).unwrap().trim(), "one\ntwo");
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn execute_draft_does_not_send_line_leading_hash_to_backend_shell() {
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
    assert!(output.contains("Aish command not implemented yet"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn execute_draft_appends_successful_command_to_regular_history() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
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
    assert_eq!(loaded.items[0].exit_code, Some(0));
    assert_eq!(loaded.items[0].source, HistorySource::User);
}

#[test]
fn execute_draft_appends_failed_command_to_regular_history() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
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
    assert_eq!(loaded.items[0].exit_code, Some(1));
    assert_eq!(loaded.items[0].source, HistorySource::User);
}
