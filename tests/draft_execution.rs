use std::time::Duration;

use aish::app::{AppState, execute_draft};
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
