use super::*;

#[test]
fn empty_tab_cycles_modes() {
    let mut state = AppState::default();
    state.handle_empty_tab();
    assert_eq!(state.mode, Mode::History);
    state.handle_empty_tab();
    assert_eq!(state.mode, Mode::Ai);
    state.handle_empty_tab();
    assert_eq!(state.mode, Mode::Draft);
}

#[test]
fn empty_tab_to_draft_always_opens_blank_draft() {
    let mut state = AppState {
        mode: Mode::Ai,
        selected_draft_index: Some(0),
        draft_from_editor: true,
        draft_from_ai_editor: true,
        draft_from_template: true,
        ..AppState::default()
    };

    state.handle_empty_tab();

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);
    assert!(!state.draft_from_editor);
    assert!(!state.draft_from_ai_editor);
    assert!(!state.draft_from_template);
}

#[test]
fn non_empty_tab_does_not_switch_modes() {
    let mut state = AppState::default();
    state.draft.insert_str("git");
    state.handle_empty_tab();
    assert_eq!(state.mode, Mode::Draft);
}

#[test]
fn unlock_passthrough_clears_live_completion_and_restores_mode_on_success() {
    let candidate = test_completion_candidate("git status");
    let now = Instant::now();
    let mut state = AppState {
        mode: Mode::Ai,
        ctrl_x_prefix: true,
        completion_panel: vec!["history           git status".to_string()],
        completion_inline: Some(InlineCompletion {
            candidate: candidate.clone(),
            suffix: " status".to_string(),
        }),
        pending_completion: Some(PendingCompletion {
            id: 1,
            line: "git".to_string(),
            cursor: 3,
            candidates: vec![candidate.clone()],
        }),
        pending_completion_update: Some(PendingCompletionUpdate {
            id: 1,
            line: "git".to_string(),
            cursor: 3,
            candidates: vec![candidate],
            first_seen: now,
            final_tier_seen: false,
        }),
        completion_display_not_before: Some(now + Duration::from_millis(120)),
        ..AppState::default()
    };

    let observed = state
        .run_unlock_passthrough(|state| {
            assert_eq!(state.mode, Mode::UnlockPassthrough);
            assert!(!state.ctrl_x_prefix);
            assert!(state.completion_panel.is_empty());
            assert!(state.completion_inline.is_none());
            assert!(state.pending_completion.is_none());
            assert!(state.pending_completion_update.is_none());
            assert!(state.completion_display_not_before.is_none());
            Ok(42)
        })
        .unwrap();

    assert_eq!(observed, 42);
    assert_eq!(state.mode, Mode::Ai);
}

#[test]
fn unlock_passthrough_restores_mode_on_error() {
    let mut state = AppState {
        mode: Mode::History,
        ctrl_x_prefix: true,
        ..AppState::default()
    };

    let result: Result<()> = state.run_unlock_passthrough(|state| {
        assert_eq!(state.mode, Mode::UnlockPassthrough);
        anyhow::bail!("unlock failed")
    });

    assert!(result.unwrap_err().to_string().contains("unlock failed"));
    assert_eq!(state.mode, Mode::History);
    assert!(!state.ctrl_x_prefix);
}

#[test]
fn prompt_line_uses_current_mode_symbol() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");
    assert_eq!(state.render_prompt_line(), "> git status");

    state.mode = Mode::History;
    assert_eq!(state.render_prompt_line(), "$ ");

    state.mode = Mode::Ai;
    assert_eq!(state.render_prompt_line(), "% ");
}

#[test]
fn loaded_draft_history_is_browsable_but_not_selected_by_default() {
    let mut state = AppState {
        draft_history: vec![
            DraftEntry {
                t: 1,
                text: "old".to_string(),
            },
            DraftEntry {
                t: 2,
                text: "new".to_string(),
            },
        ],
        ..AppState::default()
    };

    assert!(state.draft.is_empty());
    assert_eq!(state.selected_draft_index, None);

    assert!(state.move_draft_selection_older().unwrap());
    assert_eq!(state.draft.as_str(), "new");
    assert_eq!(state.selected_draft_index, Some(1));
}

#[test]
fn prompt_line_renders_configured_prompt_variables() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("repo");
    let mut state = AppState {
        current_cwd: Some(cwd.clone()),
        last_status: Some(7),
        prompt_templates: PromptTemplates {
            draft: "[{mode}:{basename}:{last_status}] ".to_string(),
            history: "hist {cwd} {mode} ".to_string(),
            ai: "ai {mode} ".to_string(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("git status");

    assert_eq!(state.render_prompt_line(), "[>:repo:7] git status");

    state.mode = Mode::History;
    assert_eq!(
        state.render_prompt_line(),
        format!("hist {} $ ", cwd.display())
    );

    state.mode = Mode::Ai;
    assert_eq!(state.render_prompt_line(), "ai % ");
}

#[test]
fn prompt_line_abbreviates_home_directory_as_tilde() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let mut state = AppState {
        current_cwd: Some(home.clone()),
        prompt_templates: PromptTemplates {
            draft: "{cwd} > ".to_string(),
            history: "{cwd} $ ".to_string(),
            ai: "{cwd} % ".to_string(),
        },
        ..AppState::default()
    };

    assert_eq!(state.render_prompt_line(), "~ > ");

    state.current_cwd = Some(home.join("repo/project"));
    assert_eq!(state.render_prompt_line(), "~/repo/project > ");
}

#[test]
fn prompt_line_renders_pending_context_confirmation() {
    let state = AppState {
        pending_context: Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: true,
        }),
        ..AppState::default()
    };

    assert_eq!(
        state.render_prompt_line(),
        "> [dangerous context confirmation: Y/n]"
    );
    assert_eq!(
        state.terminal_cursor_column(),
        display_width(&state.render_prompt_line()) as u16
    );
}

#[test]
fn prompt_line_renders_pending_private_output_confirmation() {
    let state = AppState {
        pending_private_output: Some(PendingPrivateOutput {
            label: "history".to_string(),
            output: "git status\n".to_string(),
            sink: PrivateOutputSink::Pipe {
                command: "wc -l".to_string(),
            },
        }),
        ..AppState::default()
    };

    assert_eq!(
        state.render_prompt_line(),
        "> [private output export confirmation: Y/n]"
    );
    assert_eq!(
        state.terminal_cursor_column(),
        display_width(&state.render_prompt_line()) as u16
    );
}

#[test]
fn completion_candidates_use_templates_before_history_for_first_token() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(
        &template_path,
        &TemplateEntry::new("git add . && git commit"),
    )
    .unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        completion_config: CompletionConfig {
            mode: None,
            enabled: true,
            max_results: 2,
            coalesce_ms: 50,
            display_delay_ms: 120,
            backend: false,
            ignore_spaces: true,
            template_first: true,
            inline: true,
            fuzzy: true,
            tab_accept: CompletionTabAccept::Full,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        },
        ..AppState::default()
    };
    state.draft.insert_str("git");

    let candidates = state.completion_candidates_with_max_results(2).unwrap();

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].display, "git add . && git commit");
    assert_eq!(
        candidates[0].source,
        crate::completion::CompletionSource::Template
    );
    assert_eq!(candidates[1].display, "git status");
    assert_eq!(
        candidates[1].source,
        crate::completion::CompletionSource::History
    );
}

#[test]
fn completion_candidates_use_path_completion_for_path_like_token() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    state.draft.insert_str("cat src/m");

    let candidates = state.completion_candidates().unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].display, "src/main.rs");
    assert_eq!(
        candidates[0].source,
        crate::completion::CompletionSource::Path
    );
}

#[test]
fn completion_candidates_offer_private_commands_after_hash_prefix() {
    let mut state = AppState::default();
    state.draft.insert_str("#sta");

    let candidates = state.completion_candidates().unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].display, "#status");
    assert_eq!(
        candidates[0].source,
        crate::completion::CompletionSource::PrivateCommand
    );
}

#[test]
fn completion_candidates_stay_quiet_for_hash_space_ai_prompts() {
    let mut state = AppState {
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("# ");

    assert!(state.completion_candidates().unwrap().is_empty());

    state.draft.insert_str("git");
    assert!(state.completion_candidates().unwrap().is_empty());
}

#[test]
fn completion_candidates_use_structural_history_after_trailing_space() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("local-file"), "").unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    let candidates = state.completion_candidates().unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].display, "status --short");
    assert_eq!(
        candidates[0].source,
        crate::completion::CompletionSource::History
    );
}

#[test]
fn completion_candidates_split_discovery_from_panel_row_limit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("alpha-one.txt"), "").unwrap();
    std::fs::write(temp.path().join("alpha-two.txt"), "").unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config: CompletionConfig {
            max_results: 1,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("cat alpha-");

    let all_candidates = state.completion_candidates().unwrap();
    let panel_candidates = state.completion_panel_candidates().unwrap();

    assert_eq!(all_candidates.len(), 2);
    assert_eq!(panel_candidates.len(), 1);
}

#[test]
fn completion_candidates_skip_editor_drafts_and_read_only_modes() {
    let mut state = AppState::default();
    state.draft.insert_str("git");
    state.draft_from_editor = true;
    assert!(state.completion_candidates().unwrap().is_empty());

    state.draft_from_editor = false;
    state.mode = Mode::History;
    assert!(state.completion_candidates().unwrap().is_empty());
}

#[test]
fn completion_candidates_respect_global_enabled_switch() {
    let mut state = AppState {
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        completion_config: CompletionConfig {
            enabled: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("git");

    assert!(state.completion_candidates().unwrap().is_empty());
    assert!(
        state
            .start_live_completion_request(usize::MAX)
            .unwrap()
            .is_empty()
    );
    assert!(state.pending_completion.is_none());
}

#[test]
fn pending_completion_update_waits_for_coalesce_window_without_final_tier() {
    let candidate = CompletionCandidate {
        display: "status --short".to_string(),
        replacement: "status --short".to_string(),
        is_dir: false,
        source: crate::completion::CompletionSource::History,
    };
    let first_seen = Instant::now();
    let mut state = AppState {
        completion_config: CompletionConfig {
            coalesce_ms: 50,
            ..CompletionConfig::default()
        },
        pending_completion: Some(PendingCompletion {
            id: 7,
            line: "git ".to_string(),
            cursor: 4,
            candidates: vec![candidate.clone()],
        }),
        pending_completion_update: Some(PendingCompletionUpdate {
            id: 7,
            line: "git ".to_string(),
            cursor: 4,
            candidates: vec![candidate.clone()],
            first_seen,
            final_tier_seen: false,
        }),
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    assert!(
        state
            .ready_completion_update(first_seen + Duration::from_millis(49))
            .is_none()
    );
    assert_eq!(
        state.ready_completion_update(first_seen + Duration::from_millis(50)),
        Some(vec![candidate])
    );
}

#[test]
fn pending_completion_update_flushes_immediately_on_final_tier() {
    let candidate = CompletionCandidate {
        display: "status --short".to_string(),
        replacement: "status --short".to_string(),
        is_dir: false,
        source: crate::completion::CompletionSource::History,
    };
    let first_seen = Instant::now();
    let mut state = AppState {
        completion_config: CompletionConfig {
            coalesce_ms: 1_000,
            ..CompletionConfig::default()
        },
        pending_completion: Some(PendingCompletion {
            id: 8,
            line: "git ".to_string(),
            cursor: 4,
            candidates: vec![candidate.clone()],
        }),
        pending_completion_update: Some(PendingCompletionUpdate {
            id: 8,
            line: "git ".to_string(),
            cursor: 4,
            candidates: vec![candidate.clone()],
            first_seen,
            final_tier_seen: true,
        }),
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    assert_eq!(
        state.ready_completion_update(first_seen),
        Some(vec![candidate])
    );
}

#[test]
fn completion_display_delay_hides_ui_without_blocking_candidate_cache() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config: CompletionConfig {
            display_delay_ms: 120,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("cat si");
    let now = Instant::now();
    state.defer_completion_display(now);
    let deadline = state.completion_display_not_before.unwrap();

    let visible_candidates = state.start_live_completion_request(usize::MAX).unwrap();

    assert!(visible_candidates.is_empty());
    let pending = state.pending_completion.as_ref().unwrap();
    assert!(
        pending
            .candidates
            .iter()
            .any(|candidate| candidate.display == "single.txt")
    );
    assert!(
        state
            .ready_completion_update(deadline - Duration::from_millis(1))
            .is_none()
    );
    assert!(
        state
            .ready_completion_update(deadline)
            .unwrap()
            .iter()
            .any(|candidate| candidate.display == "single.txt")
    );
}

#[test]
fn completion_display_delay_resets_to_latest_input_time() {
    let mut state = AppState {
        completion_config: CompletionConfig {
            display_delay_ms: 120,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    let first = Instant::now();
    state.defer_completion_display(first);
    let first_deadline = state.completion_display_not_before.unwrap();

    state.defer_completion_display(first + Duration::from_millis(80));

    assert_eq!(
        state.completion_display_not_before,
        Some(first_deadline + Duration::from_millis(80))
    );
}

#[test]
#[cfg(unix)]
fn first_token_executable_live_candidate_arrives_from_background_worker() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("aishco-exec");
    std::fs::write(&executable, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old_path = std::env::var_os("PATH");
    unsafe {
        std::env::set_var("PATH", &bin);
    }

    let mut state = AppState {
        completion_config: CompletionConfig {
            fuzzy: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("aishco");

    let visible_candidates = state.start_live_completion_request(usize::MAX);

    unsafe {
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
    let visible_candidates = visible_candidates.unwrap();
    assert!(visible_candidates.is_empty());
    assert!(state.pending_completion.is_some());
    assert!(state.pending_completion_update.is_none());

    let mut candidates = None;
    for _ in 0..50 {
        candidates = state.drain_live_completion_events();
        if candidates.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let candidates = candidates.expect("missing executable completion worker event");
    assert!(candidates.iter().any(|candidate| {
        candidate.source == CompletionSource::Executable && candidate.display == "aishco-exec"
    }));
}

#[test]
fn apply_picker_selection_replaces_current_token_with_quoted_value() {
    let mut state = AppState::default();
    state.draft.insert_str("cat old.txt");
    state.draft.move_left();
    state.draft.move_left();
    state.draft.move_left();

    assert!(state.apply_picker_selection(
        "my file.txt",
        crate::picker::PickerAction::ReplaceCurrentToken
    ));

    assert_eq!(state.draft.as_str(), "cat 'my file.txt'");
    assert_eq!(state.draft.cursor(), "cat 'my file.txt'".len());
}

#[test]
fn apply_picker_selection_skips_editor_and_read_only_modes() {
    let mut state = AppState::default();
    state.draft.insert_str("cat ");
    state.draft_from_editor = true;
    assert!(!state.apply_picker_selection("file", crate::picker::PickerAction::InsertAtCursor));
    assert_eq!(state.draft.as_str(), "cat ");

    state.draft_from_editor = false;
    state.mode = Mode::History;
    assert!(!state.apply_picker_selection("file", crate::picker::PickerAction::InsertAtCursor));
}

#[test]
fn apply_raw_picker_selection_replaces_without_shell_quoting() {
    let mut state = AppState::default();
    state.draft.insert_str("echo OLD");
    state.draft.move_left();
    state.draft.move_left();

    assert!(
        state.apply_raw_picker_selection("$HOME", crate::picker::PickerAction::ReplaceCurrentToken)
    );

    assert_eq!(state.draft.as_str(), "echo $HOME");
    assert_eq!(state.draft.cursor(), "echo $HOME".len());
}

#[test]
fn history_picker_candidates_follow_current_mode_scope() {
    let regular_history = vec![
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
    ];
    let ai_sessions = vec![AiSession {
        id: "s1".to_string(),
        t: 3,
        prompt: "prompt".to_string(),
        ctx: false,
        model: "test".to_string(),
        items: vec![AiItem {
            kind: AiItemKind::Command,
            text: "ai command".to_string(),
            name: None,
        }],
    }];
    let mut state = AppState {
        regular_history,
        ai_sessions,
        ..AppState::default()
    };

    assert_eq!(
        state.history_picker_candidates(),
        vec!["two", "one", "ai command"]
    );
    state.mode = Mode::History;
    assert_eq!(state.history_picker_candidates(), vec!["two", "one"]);
    state.mode = Mode::Ai;
    assert_eq!(state.history_picker_candidates(), vec!["ai command"]);
}

#[test]
fn replace_draft_from_history_picker_copies_raw_command_to_draft() {
    let mut state = AppState {
        mode: Mode::History,
        draft_from_editor: true,
        draft_from_template: true,
        selected_draft_index: Some(0),
        ..AppState::default()
    };

    state.replace_draft_from_history_picker("git commit -m 'hello world'");

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git commit -m 'hello world'");
    assert_eq!(state.selected_draft_index, None);
    assert!(!state.draft_from_editor);
    assert!(!state.draft_from_template);
}

#[test]
fn template_picker_candidates_return_newest_unique_ids() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates.jsonl");
    for body in ["old", "tail", "old"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };

    assert_eq!(
        state.template_picker_candidates().unwrap(),
        vec![
            format!("{}\told", template_id("old")),
            format!("{}\ttail", template_id("tail"))
        ]
    );
}

#[test]
fn replace_draft_from_template_picker_uses_selected_template_id() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates.jsonl");
    for body in ["old", "rsync {from} {to}"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path),
        draft_from_editor: true,
        selected_draft_index: Some(0),
        ..AppState::default()
    };

    assert!(
        state
            .replace_draft_from_template_picker(&template_id("rsync {from} {to}"))
            .unwrap()
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "rsync {from} {to}");
    assert_eq!(state.selected_draft_index, None);
    assert!(state.draft_from_template);
    assert!(!state.draft_from_editor);
}

#[test]
fn store_ai_session_from_items_persists_and_selects_first_command() {
    let temp = tempfile::tempdir().unwrap();
    let ai_path = temp.path().join("history/ai.jsonl");
    let mut state = AppState {
        ai_history_path: Some(ai_path.clone()),
        ai_sessions: vec![AiSession {
            id: "old".to_string(),
            t: 1,
            prompt: "old prompt".to_string(),
            ctx: false,
            model: "old-model".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "old command".to_string(),
                name: None,
            }],
        }],
        clock: || 42,
        ..AppState::default()
    };

    assert!(
        state
            .store_ai_session_from_items(
                "new prompt",
                "gpt-test",
                vec![
                    AiItem {
                        kind: AiItemKind::Template,
                        text: "template body".to_string(),
                        name: Some("tpl".to_string()),
                    },
                    AiItem {
                        kind: AiItemKind::Command,
                        text: "new command".to_string(),
                        name: None,
                    },
                ],
            )
            .unwrap()
    );

    assert_eq!(state.mode, Mode::Ai);
    assert_eq!(state.selected_ai_index, Some(1));
    assert_eq!(state.selected_ai_command(), Some("new command"));
    assert_eq!(state.ai_sessions.len(), 2);
    assert_eq!(state.ai_sessions[1].prompt, "new prompt");
    assert_eq!(state.ai_sessions[1].model, "gpt-test");
    let loaded = load_jsonl::<AiSession>(&ai_path).unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].prompt, "new prompt");
}
