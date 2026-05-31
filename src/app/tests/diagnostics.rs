use super::*;

#[test]
fn private_config_prints_read_only_runtime_config() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let notes_path = temp.path().join("history/notes.jsonl");
    let draft_path = temp.path().join("history/draft.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    let config_path = temp.path().join("config.toml");
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(history_path.clone()),
        notes_path: Some(notes_path.clone()),
        draft_history_path: Some(draft_path.clone()),
        template_store_path: Some(template_path.clone()),
        backend_shell: Some("/bin/bash".to_string()),
        draft_persist: false,
        editor_config: EditorConfig {
            command: vec!["nvim".to_string(), "--clean".to_string()],
            execute_after_save: false,
        },
        completion_config: CompletionConfig {
            mode: None,
            enabled: true,
            max_results: 8,
            coalesce_ms: 50,
            display_delay_ms: 120,
            backend: true,
            ignore_spaces: false,
            template_first: true,
            inline: false,
            fuzzy: true,
            tab_accept: CompletionTabAccept::Word,
            match_threshold_percent: 75,
            typo_threshold_percent: 80,
        },
        ai_config: AiConfig {
            model: "gpt-test".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            ..AiConfig::default()
        },
        context_config: ContextConfig {
            enabled: false,
            confirm: false,
            max_bytes: 1024,
        },
        ..AppState::default()
    };
    state.draft.insert_str("#config");
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
    assert!(output.contains("Aish config"));
    assert!(output.contains("config_path="));
    assert!(output.contains(&config_path.display().to_string()));
    assert!(output.contains("shell.backend=/bin/bash"));
    assert!(output.contains("draft.persist=false"));
    assert!(output.contains("editor.execute_after_save=false"));
    assert!(output.contains("editor.command=nvim --clean"));
    assert!(output.contains("paste.multiline=editor"));
    assert!(output.contains("paste.confirm_execute=true"));
    assert!(output.contains("paste.preview=true"));
    assert!(output.contains("paste.preview_lines=3"));
    assert!(output.contains("paste.preview_bytes=240"));
    assert!(output.contains("completion.enabled=true"));
    assert!(output.contains("completion.mode=tab"));
    assert!(output.contains("completion.max_results=8"));
    assert!(output.contains("completion.coalesce_ms=50"));
    assert!(output.contains("completion.display_delay_ms=120"));
    assert!(output.contains("completion.ignore_spaces=false"));
    assert!(output.contains("completion.template_first=true"));
    assert!(output.contains("completion.inline=false"));
    assert!(output.contains("completion.fuzzy=true"));
    assert!(output.contains("completion.tab_accept=word"));
    assert!(output.contains("completion.match_threshold_percent=75"));
    assert!(output.contains("completion.typo_threshold_percent=80"));
    assert!(output.contains("ai.model=gpt-test"));
    assert!(output.contains("ai.base_url=https://example.invalid/v1"));
    assert!(output.contains("ai.env_key=OPENAI_API_KEY"));
    assert!(output.contains("context.enabled=false"));
    assert!(output.contains("context.confirm=false"));
    assert!(output.contains("context.max_bytes=1024"));
    assert!(output.contains("encryption=off"));
    assert!(output.contains("sync.enabled=false"));
    assert!(output.contains("editor.resolved=nvim --clean"));
    assert!(output.contains("history.regular="));
    assert!(output.contains(&history_path.display().to_string()));
    assert!(output.contains("history.notes="));
    assert!(output.contains(&notes_path.display().to_string()));
    assert!(output.contains("history.draft="));
    assert!(output.contains(&draft_path.display().to_string()));
    assert!(output.contains("templates.store="));
    assert!(output.contains(&template_path.display().to_string()));
    assert!(!history_path.exists());
    assert!(!notes_path.exists());
    assert!(!draft_path.exists());
    assert!(!template_path.exists());
    assert!(state.draft.is_empty());
}

#[test]
fn private_doctor_prints_read_only_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let history_path = temp.path().join("history/regular.jsonl");
    let notes_path = temp.path().join("history/notes.jsonl");
    let draft_path = temp.path().join("history/draft.jsonl");
    let mut state = AppState {
        last_status: Some(7),
        current_cwd: Some(temp.path().to_path_buf()),
        backend_shell: Some("/bin/bash".to_string()),
        editor_config: EditorConfig {
            command: vec!["vim".to_string()],
            execute_after_save: false,
        },
        ai_config: AiConfig {
            model: "test".to_string(),
            base_url: "https://example.invalid/v1/chat/completions".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            ..AiConfig::default()
        },
        regular_history_path: Some(history_path.clone()),
        notes_path: Some(notes_path.clone()),
        draft_history_path: Some(draft_path.clone()),
        regular_history: vec![HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ls".to_string(),
                name: None,
            }],
        }],
        ai_command_indices: vec![AiCommandIndex {
            session_index: 0,
            item_index: 0,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("#doctor");
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
    assert!(output.contains("Aish doctor"));
    assert!(output.contains("mode=>"));
    assert!(output.contains("last_status=7"));
    assert!(output.contains(&format!("cwd={}", temp.path().display())));
    assert!(output.contains("draft_persist=true"));
    assert!(output.contains("regular_history_entries=1"));
    assert!(output.contains("ai_sessions=1"));
    assert!(output.contains("ai_commands=1"));
    assert!(output.contains("output_ring_entries=0"));
    assert!(output.contains("backend_shell=/bin/bash"));
    assert!(output.contains("pty=ok"));
    assert!(output.contains("gpg=not_configured"));
    assert!(output.contains("git=not_configured"));
    assert!(output.contains("fzf=external"));
    assert!(output.contains("ai.final_url="));
    assert!(output.contains("ai.key_source=unconfigured"));
    assert!(output.contains("encryption=off"));
    assert!(output.contains("sync.enabled=false"));
    assert!(output.contains("editor.resolved=vim"));
    assert!(output.contains("regular_history_path="));
    assert!(output.contains("exists=false"));
    assert!(!history_path.exists());
    assert!(!notes_path.exists());
    assert!(!draft_path.exists());
    assert!(state.draft.is_empty());
}

#[test]
fn private_editor_reports_resolution_without_launching_editor() {
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec!["code".to_string(), "--wait".to_string()],
            execute_after_save: false,
        },
        editor_temp_root: Some(std::env::temp_dir().join("aish-editor-test")),
        ..AppState::default()
    };
    state.draft.insert_str("#editor");
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
    assert!(output.contains("Aish editor"));
    assert!(output.contains("configured=code --wait"));
    assert!(output.contains("editor.resolved=code --wait"));
    assert!(output.contains("external editor launch is wired to Ctrl-X Ctrl-E"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn unknown_private_command_prints_suggestion() {
    let mut state = AppState::default();
    state.draft.insert_str("#statsu");
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
    assert!(output.contains("unknown Aish command: #statsu"));
    assert!(output.contains("Did you mean #status?"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn private_status_prints_mode_and_last_status() {
    let mut state = AppState {
        last_status: Some(7),
        current_cwd: Some(std::env::temp_dir()),
        backend_shell: Some("/bin/bash".to_string()),
        ai_config: AiConfig {
            model: "gpt-test".to_string(),
            base_url: "https://example.invalid/v1/chat/completions".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            ..AiConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("#status");
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
    assert!(output.contains("Aish status"));
    assert!(output.contains("mode=>"));
    assert!(output.contains("last_status=7"));
    assert!(output.contains(&format!("cwd={}", std::env::temp_dir().display())));
    assert!(output.contains("shell=/bin/bash"));
    assert!(output.contains("ai.final_url="));
    assert!(output.contains("ai.key_source=unconfigured"));
    assert!(output.contains("encryption=off"));
    assert!(output.contains("sync.enabled=false"));
    assert!(output.contains("context.enabled=true"));
    assert!(output.contains("completion.enabled=true"));
    assert!(output.contains("completion.mode=auto"));
    assert!(output.contains("completion.max_results=5"));
    assert!(output.contains("completion.coalesce_ms=50"));
    assert!(output.contains("completion.display_delay_ms=120"));
    assert!(output.contains("completion.fuzzy=true"));
    assert!(output.contains("completion.match_threshold_percent=50"));
    assert!(output.contains("completion.typo_threshold_percent=80"));
    assert!(output.contains("keybindings=26"));
    assert!(state.draft.is_empty());
}

#[test]
fn private_history_without_count_prints_usage() {
    let mut state = AppState::default();
    state.draft.insert_str("#history nope");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("usage: #history search <query>")
    );
    assert!(state.draft.is_empty());
}

#[test]
fn private_list_commands_print_newest_commands_one_per_line() {
    let temp = tempfile::tempdir().unwrap();
    let sessions = vec![AiSession {
        id: "ai-1".to_string(),
        t: 1,
        prompt: "prompt".to_string(),
        ctx: false,
        model: "model".to_string(),
        items: vec![
            AiItem {
                kind: AiItemKind::Command,
                text: "ls -la".to_string(),
                name: None,
            },
            AiItem {
                kind: AiItemKind::Template,
                text: "ignored {template}".to_string(),
                name: None,
            },
            AiItem {
                kind: AiItemKind::Command,
                text: "cargo test".to_string(),
                name: None,
            },
        ],
    }];
    let mut state = AppState {
        regular_history: vec![
            HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
        draft_history: vec![
            DraftEntry {
                t: 1,
                text: "echo one\ncontinued".to_string(),
            },
            DraftEntry {
                t: 2,
                text: "echo two".to_string(),
            },
        ],
        ai_command_indices: ai_command_indices(&sessions),
        ai_sessions: sessions,
        template_store_path: Some(temp.path().join("templates.jsonl")),
        templates: vec![
            TemplateEntry::new("rsync {from} {to}"),
            TemplateEntry::new("git commit -m {message}"),
        ],
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    state.draft.insert_str("#history list");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "git status\npwd\n");

    state.draft.insert_str("#ai list");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "cargo test\nls -la\n");

    state.draft.insert_str("#draft list");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "echo two\necho one\\ncontinued\n"
    );

    state.draft.insert_str("#template list");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "git commit -m {message}\nrsync {from} {to}\n"
    );
}

#[test]
fn private_search_commands_print_matching_commands_one_per_line() {
    let mut state = AppState {
        regular_history: vec![
            HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
        draft_history: vec![DraftEntry {
            t: 1,
            text: "git diff".to_string(),
        }],
        templates: vec![TemplateEntry::new("git commit -m {message}")],
        template_store_path: Some(PathBuf::from("/tmp/aish-test-templates.jsonl")),
        ..AppState::default()
    };
    let sessions = vec![AiSession {
        id: "ai-1".to_string(),
        t: 1,
        prompt: "prompt".to_string(),
        ctx: false,
        model: "model".to_string(),
        items: vec![AiItem {
            kind: AiItemKind::Command,
            text: "git log --oneline".to_string(),
            name: None,
        }],
    }];
    state.ai_command_indices = ai_command_indices(&sessions);
    state.ai_sessions = sessions;
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#history search git", "git status\n"),
        ("#ai search log", "git log --oneline\n"),
        ("#draft search diff", "git diff\n"),
        ("#template search commit", "git commit -m {message}\n"),
    ] {
        state.draft.insert_str(line);
        let mut output = Vec::new();
        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected);
    }
}

#[test]
fn private_list_redirection_requires_confirmation_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let export_path = temp.path().join("history.txt");
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        regular_history: vec![
            HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
        ..AppState::default()
    };
    state.draft.insert_str("#history list > history.txt");
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
    assert!(output.contains("aish will export 2 history line(s)"));
    assert!(output.contains("Export list output? [Y/n]"));
    assert!(!export_path.exists());
    assert!(state.pending_private_output.is_some());

    let mut output = Vec::new();
    answer_private_output_confirmation(&mut state, true, &mut output, Duration::from_secs(5))
        .unwrap();

    assert_eq!(
        fs::read_to_string(export_path).unwrap(),
        "git status\npwd\n"
    );
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("exported 2 history line(s)")
    );
    assert!(state.pending_private_output.is_none());
}

#[test]
fn private_list_pipe_feeds_confirmed_output_to_shell_stdin() {
    let mut state = AppState {
        regular_history: vec![
            HistoryEntry {
                t: 1,
                command: "pwd".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
            HistoryEntry {
                t: 2,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        ],
        ..AppState::default()
    };
    state.draft.insert_str("#history list | wc -l");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.pending_private_output.is_some());
    let mut output = Vec::new();
    answer_private_output_confirmation(&mut state, true, &mut output, Duration::from_secs(5))
        .unwrap();

    assert!(
        String::from_utf8(output).unwrap().contains('2'),
        "wc output should include the exported line count"
    );
}
