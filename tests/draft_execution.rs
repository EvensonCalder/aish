use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use aish::app::{AppState, execute_draft};
use aish::commands::NoteTag;
use aish::config::AiConfig;
use aish::editor::prepare_editor_file;
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

    assert!(String::from_utf8(output).unwrap().contains("one\ntwo"));
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
    assert!(output.contains("Aish command not implemented yet"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
}

#[test]
fn editor_draft_can_send_line_leading_hash_to_shell() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let session = prepare_editor_file(temp.path(), "").unwrap();
    let marker = temp.path().join("editor-raw-ran");
    std::fs::write(
        &session.path,
        format!("# shell comment\ntouch {}", marker.display()),
    )
    .unwrap();
    let mut state = AppState::default();
    state.replace_draft_from_editor_session(&session).unwrap();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(marker.exists());
    assert_eq!(state.last_status, Some(0));
    assert!(!state.draft_from_editor);
    assert!(state.draft.is_empty());
}

#[test]
#[allow(unused_variables)]
fn editor_draft_sends_multiline_backslash_continuation_to_shell() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let _history_path = temp.path().join("history/regular.jsonl");
    let command = "printf '%s\\n' \\\n+editor-continuation";
    let session = prepare_editor_file(temp.path(), "").unwrap();
    std::fs::write(&session.path, "printf '%s\\n' \\\n+editor-continuation").unwrap();
    let mut state = AppState::default();
    state.replace_draft_from_editor_session(&session).unwrap();
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
    assert!(output.contains("editor-continuation"));
    assert_eq!(state.last_status, Some(0));
    assert!(state.draft.is_empty());
}

#[test]
fn editor_draft_executes_each_pasted_line() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    state.replace_draft_from_editor_text("echo paste-one\necho paste-two");

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("paste-one"), "output was {output:?}");
    assert!(output.contains("paste-two"), "output was {output:?}");
}

#[test]
fn editor_draft_preserves_multiline_command_after_prompt_render() {
    let _guard = pty_execution_guard();
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut redraw_output = Vec::new();
    let mut command_output = Vec::new();

    state.replace_draft_from_editor_text("echo paste-one\necho paste-two");
    let rendered = state.rendered_text();
    std::io::Write::write_all(&mut redraw_output, rendered.as_bytes()).unwrap();

    execute_draft(
        &mut state,
        &mut backend,
        &mut command_output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(command_output).unwrap();
    assert!(output.contains("paste-one"), "output was {output:?}");
    assert!(output.contains("paste-two"), "output was {output:?}");
}

#[test]
fn editor_draft_preserves_backslash_continuation_in_history() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let command = "printf '%s\\n' \\\n+editor-history-continuation";
    let session = prepare_editor_file(temp.path(), "").unwrap();
    std::fs::write(&session.path, command).unwrap();
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    state.replace_draft_from_editor_session(&session).unwrap();
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
    assert!(output.contains("editor-history-continuation"));
    let loaded = load_jsonl::<HistoryEntry>(&history_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, command);
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
    assert!(output.contains("aish will run this command to collect context"));
    assert!(output.contains("Run context command? [Y/n]"));
    assert!(!marker.exists());
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.pending_context.is_some());
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

fn fake_echo_message_ai_items(_: &AiConfig, prompt: &str) -> anyhow::Result<Vec<AiItem>> {
    assert_eq!(prompt, "how to echo something?");
    Ok(vec![AiItem {
        kind: AiItemKind::Command,
        text: "echo {message}".to_string(),
        name: None,
    }])
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
