use super::*;

#[test]
fn printable_keys_edit_draft_at_cursor() {
    let mut state = AppState::default();
    apply_key_to_state(key(KeyCode::Char('a')), &mut state);
    apply_key_to_state(key(KeyCode::Char('c')), &mut state);
    apply_key_to_state(key(KeyCode::Left), &mut state);
    apply_key_to_state(key(KeyCode::Char('b')), &mut state);

    assert_eq!(state.draft.as_str(), "abc");
    assert_eq!(state.draft.cursor(), 2);
}

#[test]
fn control_navigation_and_deletion_update_draft() {
    let mut state = AppState::default();
    state.draft.insert_str("cargo test --all");

    assert_eq!(
        apply_key_to_state(ctrl('a'), &mut state),
        KeyAction::Continue
    );
    assert_eq!(state.draft.cursor(), 0);
    apply_key_to_state(ctrl('e'), &mut state);
    assert_eq!(state.draft.cursor(), state.draft.as_str().len());
    apply_key_to_state(ctrl('w'), &mut state);
    assert_eq!(state.draft.as_str(), "cargo test ");
    apply_key_to_state(ctrl('u'), &mut state);
    assert_eq!(state.draft.as_str(), "");
}

#[test]
fn alt_word_navigation_moves_by_tokens() {
    let mut state = AppState::default();
    state.draft.insert_str("git commit message");

    apply_key_to_state(ctrl('a'), &mut state);
    apply_key_to_state(alt('f'), &mut state);
    assert_eq!(state.draft.cursor(), 4);
    apply_key_to_state(alt('f'), &mut state);
    assert_eq!(state.draft.cursor(), 11);
    apply_key_to_state(alt('b'), &mut state);
    assert_eq!(state.draft.cursor(), 4);
}

#[test]
fn alt_word_deletion_removes_words_around_cursor() {
    let mut state = AppState::default();
    state.draft.insert_str("cargo test --all");

    apply_key_to_state(ctrl('a'), &mut state);
    apply_key_to_state(alt_key(KeyCode::Delete), &mut state);
    assert_eq!(state.draft.as_str(), " test --all");
    assert_eq!(state.draft.cursor(), 0);

    apply_key_to_state(alt('d'), &mut state);
    assert_eq!(state.draft.as_str(), " --all");
    assert_eq!(state.draft.cursor(), 0);

    apply_key_to_state(ctrl('e'), &mut state);
    apply_key_to_state(alt_key(KeyCode::Backspace), &mut state);
    assert_eq!(state.draft.as_str(), " ");
}

#[test]
fn tab_switches_mode_only_for_empty_draft() {
    let mut state = AppState::default();
    apply_key_to_state(key(KeyCode::Tab), &mut state);
    assert_eq!(state.mode, Mode::History);
}

#[test]
fn non_empty_tab_requests_completion_display_without_editing_draft() {
    let mut state = AppState::default();
    state.draft.insert_str("git");

    assert_eq!(
        apply_key_to_state(key(KeyCode::Tab), &mut state),
        KeyAction::CompleteOrShow
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git");
}

#[test]
fn down_on_non_empty_new_draft_saves_and_opens_blank_draft() {
    let temp = tempfile::tempdir().unwrap();
    let draft_path = temp.path().join("draft.jsonl");
    let mut state = AppState {
        draft_history_path: Some(draft_path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    state.draft.insert_str("echo saved-draft");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    assert!(
        !handle_key(
            key(KeyCode::Down),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap()
    );

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert!(!state.draft_from_editor);
    assert!(!state.draft_from_template);
    assert_eq!(state.selected_draft_index, None);
    let loaded = crate::history::load_jsonl::<crate::history::DraftEntry>(&draft_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].t, fixed_clock());
    assert_eq!(loaded.items[0].text, "echo saved-draft");
    assert_eq!(state.draft_history, loaded.items);
}

#[test]
fn up_on_blank_draft_restores_newest_saved_draft() {
    let mut state = AppState {
        draft_history: vec![
            crate::history::DraftEntry {
                t: 1,
                text: "echo older-draft".to_string(),
            },
            crate::history::DraftEntry {
                t: 2,
                text: "echo newest-draft".to_string(),
            },
        ],
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    handle_key(
        key(KeyCode::Up),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo newest-draft");
    assert_eq!(state.selected_draft_index, Some(1));
    assert!(!state.draft_from_editor);
    assert!(!state.draft_from_template);
}

#[test]
fn up_and_down_browse_multiple_saved_drafts() {
    let mut state = AppState {
        draft_history: vec![
            crate::history::DraftEntry {
                t: 1,
                text: "echo first-draft".to_string(),
            },
            crate::history::DraftEntry {
                t: 2,
                text: "echo second-draft".to_string(),
            },
            crate::history::DraftEntry {
                t: 3,
                text: "echo third-draft".to_string(),
            },
        ],
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    for (key_code, expected_draft, expected_index) in [
        (KeyCode::Up, "echo third-draft", Some(2)),
        (KeyCode::Up, "echo second-draft", Some(1)),
        (KeyCode::Up, "echo first-draft", Some(0)),
        (KeyCode::Up, "echo first-draft", Some(0)),
        (KeyCode::Down, "echo second-draft", Some(1)),
        (KeyCode::Down, "echo third-draft", Some(2)),
    ] {
        handle_key(
            key(key_code),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(state.draft.as_str(), expected_draft);
        assert_eq!(state.selected_draft_index, expected_index);
    }

    handle_key(
        key(KeyCode::Down),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
}

#[test]
fn draft_navigation_is_disabled_when_draft_persistence_is_disabled() {
    let mut state = AppState {
        draft_persist: false,
        draft_history: vec![crate::history::DraftEntry {
            t: 1,
            text: "echo saved-draft".to_string(),
        }],
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    handle_key(
        key(KeyCode::Up),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
}

#[test]
fn editing_saved_draft_then_navigating_newer_saves_as_new_draft() {
    let temp = tempfile::tempdir().unwrap();
    let draft_path = temp.path().join("draft.jsonl");
    let mut state = AppState {
        draft_history_path: Some(draft_path.clone()),
        draft_history: vec![
            crate::history::DraftEntry {
                t: 1,
                text: "echo older-draft".to_string(),
            },
            crate::history::DraftEntry {
                t: 2,
                text: "echo newer-draft".to_string(),
            },
        ],
        selected_draft_index: Some(0),
        clock: fixed_clock,
        ..AppState::default()
    };
    state.draft.insert_str("echo older-draft edited");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    handle_key(
        key(KeyCode::Down),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.draft_history.len(), 3);
    assert_eq!(state.draft_history[2].t, fixed_clock());
    assert_eq!(state.draft_history[2].text, "echo older-draft edited");
    let loaded = crate::history::load_jsonl::<crate::history::DraftEntry>(&draft_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].text, "echo older-draft edited");
}

#[test]
fn ctrl_c_clears_multiline_draft_and_returns_to_empty_prompt() {
    let mut state = AppState {
        selected_draft_index: Some(0),
        ..AppState::default()
    };
    state.draft.insert_str("echo \"\n123");

    apply_key_to_state(ctrl('c'), &mut state);

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.mode, crate::modes::Mode::Draft);
    assert_eq!(state.render_prompt_line(), "> ");
}

#[test]
fn ctrl_c_from_history_mode_returns_to_draft_prompt() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "echo older".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        mode: crate::modes::Mode::History,
        selected_history_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(ctrl('c'), &mut state);

    assert_eq!(state.mode, crate::modes::Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.render_prompt_line(), "> ");
}

#[test]
fn enter_and_empty_ctrl_d_return_actions() {
    let mut state = AppState::default();
    assert_eq!(
        apply_key_to_state(key(KeyCode::Enter), &mut state),
        KeyAction::Submit
    );
    assert_eq!(apply_key_to_state(ctrl('d'), &mut state), KeyAction::Exit);

    state.draft.insert_str("x");
    assert_eq!(
        apply_key_to_state(ctrl('d'), &mut state),
        KeyAction::Continue
    );
}

#[test]
fn pending_context_confirmation_keys_return_confirmation_actions() {
    let mut state = AppState {
        pending_context: Some(crate::app::PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: false,
        }),
        ..AppState::default()
    };

    assert_eq!(
        apply_key_to_state(key(KeyCode::Char('y')), &mut state),
        KeyAction::ConfirmContext(true)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Enter), &mut state),
        KeyAction::ConfirmContext(true)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Char('n')), &mut state),
        KeyAction::ConfirmContext(false)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Esc), &mut state),
        KeyAction::ConfirmContext(false)
    );
    assert_eq!(
        apply_key_to_state(ctrl('c'), &mut state),
        KeyAction::ConfirmContext(false)
    );
}

#[test]
fn pending_private_output_confirmation_keys_return_confirmation_actions() {
    let mut state = AppState {
        pending_private_output: Some(crate::app::PendingPrivateOutput {
            label: "history".to_string(),
            output: "git status\n".to_string(),
            sink: crate::app::PrivateOutputSink::Pipe {
                command: "wc -l".to_string(),
            },
        }),
        ..AppState::default()
    };

    assert_eq!(
        apply_key_to_state(key(KeyCode::Char('y')), &mut state),
        KeyAction::ConfirmPrivateOutput(true)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Enter), &mut state),
        KeyAction::ConfirmPrivateOutput(true)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Char('n')), &mut state),
        KeyAction::ConfirmPrivateOutput(false)
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Esc), &mut state),
        KeyAction::ConfirmPrivateOutput(false)
    );
}

#[test]
fn esc_clears_draft_and_returns_to_draft_mode() {
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        selected_history_index: Some(0),
        selected_draft_index: Some(0),
        ..AppState::default()
    };
    state.draft.insert_str("partial");

    assert_eq!(
        apply_key_to_state(key(KeyCode::Esc), &mut state),
        KeyAction::Continue
    );

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert_eq!(state.selected_history_index, Some(0));
}

#[test]
fn ctrl_r_returns_history_search_action_without_editing_draft() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    assert_eq!(
        apply_key_to_state(ctrl('r'), &mut state),
        KeyAction::HistorySearch
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn configured_single_key_binding_replaces_default_history_search() {
    let mut state = AppState {
        keybinding_config: KeybindingConfig {
            history_search: vec![KeySequenceConfig::new("Ctrl-P").unwrap()],
            ..KeybindingConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("git status");

    assert_eq!(
        apply_key_to_state(ctrl('p'), &mut state),
        KeyAction::HistorySearch
    );
    assert_eq!(
        apply_key_to_state(ctrl('r'), &mut state),
        KeyAction::Continue
    );
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn configured_two_key_binding_uses_custom_prefix() {
    let mut state = AppState {
        keybinding_config: KeybindingConfig {
            file_picker: vec![KeySequenceConfig::new("Ctrl-G Ctrl-F").unwrap()],
            ..KeybindingConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("cat old.txt");

    assert_eq!(
        apply_key_to_state(ctrl('g'), &mut state),
        KeyAction::Continue
    );
    assert!(!state.ctrl_x_prefix);
    assert!(state.has_pending_key_prefix());
    assert_eq!(
        apply_key_to_state(ctrl('f'), &mut state),
        KeyAction::FilePicker
    );
    assert!(!state.has_pending_key_prefix());
    assert_eq!(state.draft.as_str(), "cat old.txt");
}

#[test]
fn ctrl_x_prefix_resolves_editor_chord_to_launch_action() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    assert_eq!(
        apply_key_to_state(ctrl('x'), &mut state),
        KeyAction::Continue
    );
    assert!(state.ctrl_x_prefix);
    assert_eq!(
        apply_key_to_state(ctrl('e'), &mut state),
        KeyAction::ExternalEditor
    );

    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn ctrl_x_prefix_resolves_file_picker_chord_to_launch_action() {
    let mut state = AppState::default();
    state.draft.insert_str("cat old.txt");

    apply_key_to_state(ctrl('x'), &mut state);

    assert_eq!(
        apply_key_to_state(ctrl('f'), &mut state),
        KeyAction::FilePicker
    );
    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.draft.as_str(), "cat old.txt");
}

#[test]
fn ctrl_x_prefix_resolves_template_picker_chord_to_launch_action() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    apply_key_to_state(ctrl('x'), &mut state);

    assert_eq!(
        apply_key_to_state(ctrl('t'), &mut state),
        KeyAction::TemplatePicker
    );
    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn ctrl_x_prefix_resolves_git_branch_picker_chord_to_launch_action() {
    let mut state = AppState::default();
    state.draft.insert_str("git checkout main");

    apply_key_to_state(ctrl('x'), &mut state);

    assert_eq!(
        apply_key_to_state(ctrl('b'), &mut state),
        KeyAction::GitBranchPicker
    );
    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.draft.as_str(), "git checkout main");
}

#[test]
fn ctrl_x_prefix_resolves_env_var_picker_chord_to_launch_action() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    apply_key_to_state(ctrl('x'), &mut state);

    assert_eq!(
        apply_key_to_state(ctrl('v'), &mut state),
        KeyAction::EnvVarPicker
    );
    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn ctrl_x_prefix_cancels_on_unknown_chord_without_editing_draft() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");

    apply_key_to_state(ctrl('x'), &mut state);
    assert_eq!(
        apply_key_to_state(ctrl('q'), &mut state),
        KeyAction::Continue
    );

    assert!(!state.ctrl_x_prefix);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn passthrough_mode_forwards_keys_without_interpreting_app_actions() {
    let mut state = AppState {
        mode: Mode::Passthrough,
        ..AppState::default()
    };
    state.draft.insert_str("keep");

    assert_eq!(
        apply_key_to_state(ctrl('c'), &mut state),
        KeyAction::ForwardToBackend("\x03".to_string())
    );
    assert_eq!(state.mode, Mode::Passthrough);
    assert_eq!(state.draft.as_str(), "keep");

    assert_eq!(
        apply_key_to_state(key(KeyCode::Char('x')), &mut state),
        KeyAction::ForwardToBackend("x".to_string())
    );
    assert_eq!(state.draft.as_str(), "keep");
}

#[test]
fn passthrough_mode_forwards_navigation_escape_sequences() {
    let mut state = AppState {
        mode: Mode::UnlockPassthrough,
        ..AppState::default()
    };

    assert_eq!(
        apply_key_to_state(key(KeyCode::Up), &mut state),
        KeyAction::ForwardToBackend("\x1b[A".to_string())
    );
    assert_eq!(
        apply_key_to_state(key(KeyCode::Delete), &mut state),
        KeyAction::ForwardToBackend("\x1b[3~".to_string())
    );
}

#[test]
fn editor_draft_ignores_inline_editing_keys() {
    let mut state = AppState::default();
    state.draft.insert_str("echo one\necho two");
    state.draft_from_editor = true;

    apply_key_to_state(key(KeyCode::Char('x')), &mut state);
    apply_key_to_state(key(KeyCode::Backspace), &mut state);
    apply_key_to_state(ctrl('u'), &mut state);
    apply_key_to_state(key(KeyCode::Left), &mut state);

    assert_eq!(state.draft.as_str(), "echo one\necho two");
    assert!(state.draft_from_editor);
}

#[test]
fn template_draft_backspace_deletes_placeholder_from_outside() {
    let mut state = AppState {
        draft_from_template: true,
        ..AppState::default()
    };
    state.draft.insert_str("echo {name} now");
    state.draft.move_left();
    state.draft.move_left();
    state.draft.move_left();
    state.draft.move_left();

    apply_key_to_state(key(KeyCode::Backspace), &mut state);

    assert_eq!(state.draft.as_str(), "echo  now");
    assert_eq!(state.draft.cursor(), 5);
    assert!(state.draft_from_template);
}

#[test]
fn template_draft_delete_deletes_placeholder_from_outside() {
    let mut state = AppState {
        draft_from_template: true,
        ..AppState::default()
    };
    state.draft.insert_str("echo {name} now");
    state.draft.move_start();
    for _ in 0..5 {
        state.draft.move_right();
    }

    apply_key_to_state(key(KeyCode::Delete), &mut state);

    assert_eq!(state.draft.as_str(), "echo  now");
    assert_eq!(state.draft.cursor(), 5);
    assert!(state.draft_from_template);
}

#[test]
fn template_draft_edit_inside_placeholder_expands_to_plain_draft() {
    let mut state = AppState {
        draft_from_template: true,
        ..AppState::default()
    };
    state.draft.insert_str("echo {name}");
    state.draft.move_start();
    for _ in 0..7 {
        state.draft.move_right();
    }

    apply_key_to_state(key(KeyCode::Char('X')), &mut state);

    assert_eq!(state.draft.as_str(), "echo {nXame}");
    assert!(!state.draft_from_template);
}

#[test]
fn history_mode_up_down_browses_without_editing_draft() {
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![
            crate::history::HistoryEntry {
                t: 1,
                command: "one".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
            crate::history::HistoryEntry {
                t: 2,
                command: "two".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            },
        ],
        selected_history_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Up), &mut state);
    assert_eq!(state.mode, Mode::History);
    assert_eq!(state.selected_history_command(), Some("one"));
    assert!(state.draft.is_empty());

    apply_key_to_state(key(KeyCode::Down), &mut state);
    assert_eq!(state.selected_history_command(), Some("two"));
    assert!(state.draft.is_empty());
}

#[test]
fn history_mode_typing_copies_selection_to_draft_then_edits() {
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git statu".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        selected_history_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Char('s')), &mut state);

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn history_mode_cursor_movement_does_not_copy_to_draft() {
    let mut state = AppState {
        mode: Mode::History,
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        selected_history_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Left), &mut state);
    apply_key_to_state(key(KeyCode::Right), &mut state);
    apply_key_to_state(ctrl('a'), &mut state);
    apply_key_to_state(ctrl('e'), &mut state);

    assert_eq!(state.mode, Mode::History);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_history_command(), Some("git status"));
}

#[test]
fn ai_mode_up_down_browses_without_editing_draft() {
    let mut state = AppState {
        mode: Mode::Ai,
        ai_sessions: vec![crate::history::AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                crate::history::AiItem {
                    kind: crate::history::AiItemKind::Command,
                    text: "one".to_string(),
                    name: None,
                },
                crate::history::AiItem {
                    kind: crate::history::AiItemKind::Command,
                    text: "two".to_string(),
                    name: None,
                },
            ],
        }],
        ai_command_indices: vec![
            crate::history::AiCommandIndex {
                session_index: 0,
                item_index: 0,
            },
            crate::history::AiCommandIndex {
                session_index: 0,
                item_index: 1,
            },
        ],
        selected_ai_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Down), &mut state);
    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_command(), Some("two"));
    assert!(state.draft.is_empty());

    apply_key_to_state(key(KeyCode::Up), &mut state);
    assert_eq!(state.selected_ai_command(), Some("one"));
    assert!(state.draft.is_empty());
}

#[test]
fn ai_mode_typing_copies_selection_to_draft_then_edits() {
    let mut state = AppState {
        mode: Mode::Ai,
        ai_sessions: vec![crate::history::AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![crate::history::AiItem {
                kind: crate::history::AiItemKind::Command,
                text: "git statu".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![crate::history::AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        selected_ai_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Char('s')), &mut state);

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
}

#[test]
fn ai_mode_cursor_movement_does_not_copy_to_draft() {
    let mut state = AppState {
        mode: Mode::Ai,
        ai_sessions: vec![crate::history::AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![crate::history::AiItem {
                kind: crate::history::AiItemKind::Command,
                text: "git status".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![crate::history::AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        selected_ai_index: Some(0),
        ..AppState::default()
    };

    apply_key_to_state(key(KeyCode::Left), &mut state);
    apply_key_to_state(key(KeyCode::Right), &mut state);
    apply_key_to_state(ctrl('a'), &mut state);
    apply_key_to_state(ctrl('e'), &mut state);

    assert_eq!(state.mode, Mode::Ai);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_ai_command(), Some("git status"));
}
