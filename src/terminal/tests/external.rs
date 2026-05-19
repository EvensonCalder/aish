use super::*;

#[test]
fn apply_file_picker_result_replaces_current_token() {
    let mut state = AppState::default();
    state.draft.insert_str("cat old.txt");
    state.draft.move_left();
    state.draft.move_left();
    state.draft.move_left();
    let mut output = Vec::new();

    apply_file_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some("new file.txt".to_string()),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "cat 'new file.txt'");
    assert!(String::from_utf8(output).unwrap().is_empty());
}

#[test]
fn apply_file_picker_result_reports_cancel_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("cat old.txt");
    let mut output = Vec::new();

    apply_file_picker_result(
        &mut state,
        PickerRunResult {
            selected: None,
            exit_code: Some(130),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "cat old.txt");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\nfile picker cancelled\n"
    );
}

#[test]
fn apply_history_picker_result_replaces_draft_without_shell_quoting() {
    let mut state = AppState::default();
    state.draft.insert_str("partial");
    let mut output = Vec::new();

    apply_history_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some("git commit -m 'hello world'".to_string()),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git commit -m 'hello world'");
    assert!(String::from_utf8(output).unwrap().is_empty());
}

#[test]
fn apply_history_picker_result_reports_cancel_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("partial");
    let mut output = Vec::new();

    apply_history_picker_result(
        &mut state,
        PickerRunResult {
            selected: None,
            exit_code: Some(130),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "partial");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\nhistory search cancelled\n"
    );
}

#[test]
fn apply_template_picker_result_copies_template_to_protected_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates.jsonl");
    crate::templates::append_template(
        &template_path,
        &crate::templates::TemplateEntry::new("rsync {from} {to}"),
    )
    .unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    let mut output = Vec::new();

    apply_template_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some(crate::templates::template_id("rsync {from} {to}")),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "rsync {from} {to}");
    assert!(state.draft_from_template);
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!(
            "template copied to draft: {}\n",
            crate::templates::template_id("rsync {from} {to}")
        )
    );
}

#[test]
fn apply_template_picker_result_reports_cancel_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("partial");
    let mut output = Vec::new();

    apply_template_picker_result(
        &mut state,
        PickerRunResult {
            selected: None,
            exit_code: Some(130),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "partial");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\ntemplate picker cancelled\n"
    );
}

#[test]
fn apply_git_branch_picker_result_replaces_current_token() {
    let mut state = AppState::default();
    state.draft.insert_str("git checkout old");
    state.draft.move_left();
    state.draft.move_left();
    let mut output = Vec::new();

    apply_git_branch_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some("feature/new branch".to_string()),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "git checkout 'feature/new branch'");
    assert!(String::from_utf8(output).unwrap().is_empty());
}

#[test]
fn apply_git_branch_picker_result_reports_cancel_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("git checkout old");
    let mut output = Vec::new();

    apply_git_branch_picker_result(
        &mut state,
        PickerRunResult {
            selected: None,
            exit_code: Some(130),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "git checkout old");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\ngit branch picker cancelled\n"
    );
}

#[test]
fn apply_env_var_picker_result_replaces_current_token_with_reference() {
    let mut state = AppState::default();
    state.draft.insert_str("echo OLD");
    state.draft.move_left();
    state.draft.move_left();
    let mut output = Vec::new();

    apply_env_var_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some("AISH_TEST_VAR".to_string()),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "echo $AISH_TEST_VAR");
    assert!(String::from_utf8(output).unwrap().is_empty());
}

#[test]
fn apply_env_var_picker_result_rejects_invalid_names_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("echo OLD");
    let mut output = Vec::new();

    apply_env_var_picker_result(
        &mut state,
        PickerRunResult {
            selected: Some("BAD-NAME".to_string()),
            exit_code: Some(0),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "echo OLD");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "environment variable picker rejected invalid name: BAD-NAME\n"
    );
}

#[test]
fn apply_env_var_picker_result_reports_cancel_without_editing() {
    let mut state = AppState::default();
    state.draft.insert_str("echo OLD");
    let mut output = Vec::new();

    apply_env_var_picker_result(
        &mut state,
        PickerRunResult {
            selected: None,
            exit_code: Some(130),
        },
        &mut output,
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "echo OLD");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\nenvironment variable picker cancelled\n"
    );
}

#[test]
fn run_external_editor_replaces_draft_after_success() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf 'echo edited' > \"$1\"\n").unwrap();
    make_executable(&script);
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec![script.display().to_string()],
            execute_after_save: false,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    state.draft.insert_str("old draft");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    run_external_editor(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "echo edited");
    assert_eq!(state.draft.cursor(), "echo edited".len());
    assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
}

#[test]
fn run_external_editor_keeps_draft_after_editor_failure() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf changed > \"$1\"\nexit 4\n").unwrap();
    make_executable(&script);
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec![script.display().to_string()],
            execute_after_save: false,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    state.draft.insert_str("old draft");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    run_external_editor(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "old draft");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "editor exited without saving draft: status=4\n"
    );
}

#[test]
fn run_external_editor_reports_missing_editor() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec!["/definitely/missing/aish-editor".to_string()],
            execute_after_save: false,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    let error = run_external_editor(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap_err();

    assert!(error.to_string().contains("failed to run editor command"));
    assert!(state.draft.is_empty());
}

#[test]
fn run_external_editor_executes_after_save_when_configured() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    let marker = temp.path().join("auto-ran");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf 'touch {}' > \"$1\"\n", marker.display()),
    )
    .unwrap();
    make_executable(&script);
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec![script.display().to_string()],
            execute_after_save: true,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    run_external_editor(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(marker.exists());
    assert_eq!(state.last_status, Some(0));
    assert!(state.draft.is_empty());
    assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
}

#[test]
fn run_external_editor_on_ai_prompt_creates_sendable_ai_editor_draft() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    let captured = temp.path().join("captured.txt");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat \"$1\" > '{}'\nprintf 'line one\\nline two' > \"$1\"\n",
            captured.display()
        ),
    )
    .unwrap();
    make_executable(&script);
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec![script.display().to_string()],
            execute_after_save: true,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    state.draft.insert_str("# explain this");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    run_external_editor(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(captured).unwrap(), "explain this");
    assert_eq!(state.draft.as_str(), "line one\nline two");
    assert!(state.draft_from_editor);
    assert!(state.draft_from_ai_editor);
    assert_eq!(
        state.render_prompt_line(),
        "> [ai prompt: 2 lines, 17 bytes; Enter send, Ctrl-X Ctrl-E edit]"
    );
    assert_eq!(state.last_status, None);
    assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
}

#[test]
fn enter_on_empty_hash_space_opens_ai_prompt_editor() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-editor.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf 'multi\\nAI' > \"$1\"\n").unwrap();
    make_executable(&script);
    let mut state = AppState {
        editor_config: EditorConfig {
            command: vec![script.display().to_string()],
            execute_after_save: false,
        },
        editor_temp_root: Some(temp.path().join("editor")),
        ..AppState::default()
    };
    state.draft.insert_str("# ");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    handle_key(
        key(KeyCode::Enter),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "multi\nAI");
    assert!(state.draft_from_editor);
    assert!(state.draft_from_ai_editor);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("editor saved draft\r\n"));
    assert!(output.contains("[ai prompt: 2 lines, 8 bytes; Enter send"));
}
