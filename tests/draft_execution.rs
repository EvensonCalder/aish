use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use aish::app::{AppState, execute_draft};
use aish::commands::NoteTag;
use aish::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, HistoryEntry, HistorySource, NoteEntry,
    append_jsonl, load_jsonl,
};
use aish::modes::Mode;
use aish::pty::PtyBackend;

fn fixed_clock() -> i64 {
    1234567890
}

static PTY_EXECUTION_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn pty_execution_guard() -> MutexGuard<'static, ()> {
    PTY_EXECUTION_TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn execute_draft_sends_command_to_backend_and_resets_state() {
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

    assert!(String::from_utf8(output).unwrap().contains("one\ntwo"));
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
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
    assert!(output.contains("Aish command not implemented yet"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn execute_draft_does_not_run_context_pseudo_pipe_command() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("context-ran");
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state
        .draft
        .insert_str(&format!("# explain this < touch {}", marker.display()));

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("AI prompts with context are not implemented yet"));
    assert!(output.contains("context command not executed"));
    assert!(output.contains("prompt: explain this"));
    assert!(!marker.exists());
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

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
    assert_eq!(state.last_status, Some(0));

    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "printf 'from history\\n'");
    assert_eq!(state.regular_history.len(), 2);
}

fn ai_state(
    commands: &[&str],
    selected_ai_index: usize,
    history_path: std::path::PathBuf,
) -> AppState {
    AppState {
        mode: Mode::Ai,
        regular_history_path: Some(history_path),
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: commands
                .iter()
                .map(|command| AiItem {
                    kind: AiItemKind::Command,
                    text: (*command).to_string(),
                    name: None,
                })
                .collect(),
        }],
        ai_command_indices: commands
            .iter()
            .enumerate()
            .map(|(item_index, _)| AiCommandIndex {
                session_index: 0,
                item_index,
            })
            .collect(),
        selected_ai_index: Some(selected_ai_index),
        clock: fixed_clock,
        ..AppState::default()
    }
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
