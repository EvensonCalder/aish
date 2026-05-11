use std::time::Duration;

use aish::app::{AppState, execute_draft};
use aish::commands::NoteTag;
use aish::history::{HistoryEntry, HistorySource, NoteEntry, load_jsonl};
use aish::modes::Mode;
use aish::pty::PtyBackend;

fn fixed_clock() -> i64 {
    1234567890
}

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
}

#[test]
fn execute_draft_appends_failed_command_to_regular_history() {
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
fn private_history_command_trims_regular_history_to_newest_entries() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    for command in ["printf 'one\\n'", "printf 'two\\n'", "printf 'three\\n'"] {
        state.draft.insert_str(command);
        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();
    }

    state.draft.insert_str("#history 2");
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].command, "printf 'two\\n'");
    assert_eq!(loaded.items[1].command, "printf 'three\\n'");

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("history trimmed to 2"));
}
