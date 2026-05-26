use super::*;

#[test]
fn editor_draft_line_leading_hash_goes_to_backend_shell() {
    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let session = prepare_editor_file(temp.path(), "").unwrap();
    let marker = temp.path().join("editor-raw-ran");
    std::fs::write(
        &session.path,
        format!("#nosuch\ntouch {}", marker.display()),
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

    let output = String::from_utf8(output).unwrap();
    assert!(marker.exists());
    assert!(!output.contains("unknown Aish command"));
    assert_eq!(state.last_status, Some(0));
    assert!(!state.draft_from_editor);
    assert!(state.draft.is_empty());
}

#[test]
#[cfg(unix)]
fn editor_draft_interactive_command_runs_through_backend_passthrough() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = pty_execution_guard();
    let temp = tempfile::tempdir().unwrap();
    let command_path = temp.path().join("less");
    let marker_path = temp.path().join("editor-backend-ran");
    std::fs::write(
        &command_path,
        format!(
            "#!/bin/sh\nif [ -t 1 ]; then printf 'backend pty output\\n'; fi\nprintf ran > {}\n",
            marker_path.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&command_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = AppState::default();
    state.replace_draft_from_editor_text(command_path.display().to_string());
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
    assert!(marker_path.exists());
    assert!(output.contains("backend pty output"));
    assert_eq!(state.last_status, Some(0));
    assert_eq!(state.output_ring.len(), 1);
    assert_eq!(
        state.output_ring[0].command,
        command_path.display().to_string()
    );
    assert!(state.output_ring[0].output.contains("backend pty output"));
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
