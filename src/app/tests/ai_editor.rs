use super::*;

#[test]
fn store_ai_session_from_items_without_commands_stays_in_draft() {
    let mut state = AppState::default();

    assert!(
        !state
            .store_ai_session_from_items(
                "prompt",
                "gpt-test",
                vec![AiItem {
                    kind: AiItemKind::Template,
                    text: "template body".to_string(),
                    name: Some("tpl".to_string()),
                }],
            )
            .unwrap()
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.selected_ai_index, None);
    assert!(state.ai_command_indices.is_empty());
    assert_eq!(state.ai_sessions.len(), 1);
}

#[test]
fn ai_prompt_reports_config_error_without_crashing() {
    let mut state = AppState::default();
    state.draft.insert_str("# how do I list files?");
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
    assert!(output.contains("AI request failed: AI model is not configured"));
    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
}

#[test]
fn command_output_preserves_clear_home_sequence_verbatim() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[H\x1b[2J\x1b[3J\x1b[H").unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\x1b[H\x1b[2J\x1b[3J\x1b[H"
    );
}

#[test]
fn command_output_preserves_common_clear_sequence_verbatim() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[H\x1b[2J").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "\x1b[H\x1b[2J");
}

#[test]
fn command_output_preserves_terminfo_clear_sequence_verbatim() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[3J\x1b[H\x1b[2J").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "\x1b[3J\x1b[H\x1b[2J");
}

#[test]
fn command_output_does_not_home_after_partial_clear_to_screen_end() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[J").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "\x1b[J");
}

#[test]
fn command_output_does_not_home_after_scrollback_only_clear() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[3J").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "\x1b[3J");
}

#[test]
fn command_output_preserves_plain_output_without_newline() {
    let mut output = Vec::new();

    write_command_output(&mut output, "plain output").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "plain output");
}

#[test]
fn terminal_cursor_column_tracks_draft_cursor() {
    let mut state = AppState::default();
    state.draft.insert_str("abc");
    assert_eq!(state.terminal_cursor_column(), 5);

    state.draft.move_left();
    assert_eq!(state.terminal_cursor_column(), 4);

    state.draft.move_start();
    assert_eq!(state.terminal_cursor_column(), 2);
}

#[test]
fn terminal_cursor_column_counts_cjk_as_full_width() {
    let mut state = AppState::default();
    state.draft.insert_str("a中b");

    assert_eq!(state.terminal_cursor_column(), 6);

    state.draft.move_left();
    assert_eq!(state.terminal_cursor_column(), 5);

    state.draft.move_left();
    assert_eq!(state.terminal_cursor_column(), 3);
}

#[test]
fn history_mode_selects_and_renders_regular_history_newest_first() {
    let mut state = AppState {
        regular_history: vec![
            HistoryEntry {
                t: 1,
                command: "one".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "two".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
        ..AppState::default()
    };

    state.handle_empty_tab();

    assert_eq!(state.mode, Mode::History);
    assert_eq!(state.selected_history_index, Some(0));
    assert_eq!(state.selected_history_command(), Some("two"));
    assert_eq!(state.render_prompt_line(), "$ two");
    assert_eq!(state.terminal_cursor_column(), 5);

    assert!(state.move_history_selection_older());
    assert_eq!(state.selected_history_command(), Some("one"));
    assert!(!state.move_history_selection_older());
    assert!(state.move_history_selection_newer());
    assert_eq!(state.selected_history_command(), Some("two"));
}

#[test]
fn selected_history_copies_to_draft_for_editing() {
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        selected_history_index: Some(0),
        selected_draft_index: Some(0),
        ..AppState::default()
    };

    assert!(state.copy_selected_history_to_draft());

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft.cursor(), "git status".len());
}

#[test]
fn ai_mode_selects_and_renders_command_items_in_order() {
    let mut state = AppState {
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "make commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "one".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Command,
                    text: "two".to_string(),
                    name: None,
                },
            ],
        }],
        ai_command_indices: vec![
            AiCommandIndex {
                session_index: 0,
                item_index: 0,
            },
            AiCommandIndex {
                session_index: 0,
                item_index: 1,
            },
        ],
        ..AppState::default()
    };

    state.handle_empty_tab();
    state.handle_empty_tab();

    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_index, Some(0));
    assert_eq!(state.selected_ai_command(), Some("one"));
    assert_eq!(state.render_prompt_line(), "% one");

    assert!(state.move_ai_selection_next());
    assert_eq!(state.selected_ai_command(), Some("two"));
    assert!(!state.move_ai_selection_next());
    assert!(state.move_ai_selection_previous());
    assert_eq!(state.selected_ai_command(), Some("one"));
}

#[test]
fn empty_tab_to_ai_preserves_existing_ai_selection() {
    let mut state = AppState {
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "one".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Command,
                    text: "two".to_string(),
                    name: None,
                },
            ],
        }],
        ai_command_indices: vec![
            AiCommandIndex {
                session_index: 0,
                item_index: 0,
            },
            AiCommandIndex {
                session_index: 0,
                item_index: 1,
            },
        ],
        selected_ai_index: Some(1),
        ..AppState::default()
    };

    state.handle_empty_tab();
    assert_eq!(state.mode, Mode::History);
    state.handle_empty_tab();

    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_index, Some(1));
    assert_eq!(state.selected_ai_command(), Some("two"));
}

#[test]
fn selected_ai_copies_to_draft_for_editing() {
    let mut state = AppState {
        mode: Mode::Ai,
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "make commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "git status".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        selected_ai_index: Some(0),
        selected_draft_index: Some(0),
        ..AppState::default()
    };

    assert!(state.copy_selected_ai_to_draft());

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft.cursor(), "git status".len());
}

#[test]
fn prepare_editor_session_writes_draft_text() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    let session = state.prepare_editor_session(temp.path()).unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(
        std::fs::read_to_string(&session.path).unwrap(),
        "git status"
    );
}

#[test]
fn prepare_editor_session_copies_history_selection_to_draft_and_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        selected_history_index: Some(0),
        ..AppState::default()
    };

    let session = state.prepare_editor_session(temp.path()).unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(
        std::fs::read_to_string(&session.path).unwrap(),
        "git status"
    );
}

#[test]
fn prepare_editor_session_copies_ai_selection_to_draft_and_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        mode: Mode::Ai,
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "status".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "git status".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        selected_ai_index: Some(0),
        ..AppState::default()
    };

    let session = state.prepare_editor_session(temp.path()).unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(
        std::fs::read_to_string(&session.path).unwrap(),
        "git status"
    );
}

#[test]
fn replace_draft_from_editor_session_preserves_editor_content() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::default();
    state.draft.insert_str("old draft");
    let session = state.prepare_editor_session(temp.path()).unwrap();
    std::fs::write(&session.path, "echo edited\n# filtered\n echo kept").unwrap();

    state.replace_draft_from_editor_session(&session).unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo edited\n# filtered\n echo kept");
    assert_eq!(state.draft.cursor(), state.draft.as_str().len());
    assert!(state.draft_from_editor);
    assert_eq!(state.last_status, None);
    assert!(state.regular_history.is_empty());
}

#[test]
fn editor_draft_renders_as_opaque_summary() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::default();
    let session = state.prepare_editor_session(temp.path()).unwrap();
    std::fs::write(&session.path, "echo one\necho two").unwrap();

    state.replace_draft_from_editor_session(&session).unwrap();

    assert_eq!(
        state.render_prompt_line(),
        "> [draft: 2 lines, 17 bytes; Enter run, Ctrl-X Ctrl-E edit]"
    );
    assert_eq!(
        state.terminal_cursor_column(),
        display_width(&state.render_prompt_line()) as u16
    );
}

#[test]
fn replace_draft_from_editor_text_creates_opaque_editor_draft() {
    let mut state = AppState::default();

    state.replace_draft_from_editor_text("echo one\necho two");

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft_from_editor);
    assert_eq!(state.draft.as_str(), "echo one\necho two");
    assert!(state.render_prompt_line().contains("[draft: 2 lines"));
}

#[test]
fn ai_prompt_editor_session_uses_prompt_body_and_renders_send_summary() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::default();
    state.draft.insert_str("# explain this");

    let session = state.prepare_ai_prompt_editor_session(temp.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&session.path).unwrap(),
        "explain this"
    );

    std::fs::write(&session.path, "line one\nline two\n").unwrap();
    state
        .replace_draft_from_ai_prompt_editor_session(&session)
        .unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "line one\nline two");
    assert!(state.draft_from_editor);
    assert!(state.draft_from_ai_editor);
    assert_eq!(
        state.render_prompt_line(),
        "> [ai prompt: 2 lines, 17 bytes; Enter send, Ctrl-X Ctrl-E edit]"
    );
}

#[test]
fn run_editor_roundtrip_replaces_draft_after_success() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf 'echo edited' > \"$1\"\n").unwrap();
    make_executable(&script);
    let command = EditorCommand {
        argv: vec![script.display().to_string()],
    };
    let mut state = AppState::default();
    state.draft.insert_str("old draft");

    let result = state.run_editor_roundtrip(temp.path(), &command).unwrap();

    assert_eq!(result.exit_code, Some(0));
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo edited");
    assert_eq!(state.draft.cursor(), "echo edited".len());
    assert!(state.regular_history.is_empty());
}

#[test]
fn run_editor_roundtrip_keeps_original_draft_after_editor_failure() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'should not replace' > \"$1\"\nexit 9\n",
    )
    .unwrap();
    make_executable(&script);
    let command = EditorCommand {
        argv: vec![script.display().to_string()],
    };
    let mut state = AppState::default();
    state.draft.insert_str("old draft");

    let result = state.run_editor_roundtrip(temp.path(), &command).unwrap();

    assert_eq!(result.exit_code, Some(9));
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "old draft");
    assert!(state.regular_history.is_empty());
}

#[test]
fn output_ring_keeps_latest_entries_up_to_capacity() {
    let mut state = AppState::default();

    for index in 0..(OUTPUT_RING_CAPACITY + 1) {
        state.push_output_entry(OutputEntry {
            command: format!("cmd {index}"),
            output: format!("out {index}"),
            exit_code: index as i32,
        });
    }

    assert_eq!(state.output_ring.len(), OUTPUT_RING_CAPACITY);
    assert_eq!(state.output_ring.front().unwrap().command, "cmd 1");
    assert_eq!(
        state.output_ring.back().unwrap().command,
        format!("cmd {OUTPUT_RING_CAPACITY}")
    );
}
