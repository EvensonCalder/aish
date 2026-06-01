use super::*;

#[test]
fn tab_with_inline_enabled_shows_single_completion_before_accepting() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat si");
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat single.txt");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn tab_accepts_cached_completion_hidden_by_display_delay() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");
    state.defer_completion_display(Instant::now());
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat single.txt");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn typed_input_shows_live_inline_completion_without_tab() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat s");

    apply_key_to_state(key(KeyCode::Char('i')), &mut state);
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert_eq!(state.draft.as_str(), "cat si");
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
    assert!(state.completion_panel.is_empty());
}

#[test]
fn async_history_completion_updates_live_ui_after_request() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("local-file"), "").unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    let mut output = Vec::new();
    for _ in 0..50 {
        refresh_after_background_events(&mut state, &mut output).unwrap();
        if state
            .completion_inline
            .as_ref()
            .is_some_and(|inline| inline.suffix == "status --short")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        state.completion_inline.as_ref().unwrap().suffix,
        "status --short"
    );
    assert!(state.completion_panel.is_empty());
}

#[test]
fn backend_shell_completion_updates_live_ui_before_duplicate_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        backend_shell: Some("aish-test-backend:status,stash".to_string()),
        current_cwd: Some(temp.path().to_path_buf()),
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("git st");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    wait_for_inline_source_with_attempts(
        &mut state,
        crate::completion::CompletionSource::BackendShell,
        200,
    );

    let inline = state.completion_inline.as_ref().unwrap();
    assert_eq!(
        inline.candidate.source,
        crate::completion::CompletionSource::BackendShell
    );
    assert_eq!(inline.candidate.replacement, "status");
    assert!(
        state
            .completion_panel
            .iter()
            .any(|row| row.starts_with("shell git stash"))
    );
    assert_eq!(
        state
            .cached_live_completion_candidates_with_max_results(10)
            .unwrap()
            .iter()
            .filter(|candidate| candidate.replacement == "status")
            .count(),
        1
    );
}

#[test]
fn tab_waits_for_backend_shell_before_accepting_history_completion() {
    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:80:assistant".to_string()),
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "nativecmd assemble".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("nativecmd ass");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "nativecmd ass");
    wait_for_inline_source_with_attempts(
        &mut state,
        crate::completion::CompletionSource::BackendShell,
        200,
    );
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "istant");
}

#[test]
fn history_completion_appears_after_backend_shell_finishes_empty() {
    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:40:".to_string()),
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "nativecmd assemble".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("nativecmd ass");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    wait_for_inline_suffix_with_attempts(&mut state, "emble", 200);
    assert_eq!(
        state.completion_inline.as_ref().unwrap().candidate.source,
        crate::completion::CompletionSource::History
    );
}

#[test]
fn visible_history_completion_is_not_accepted_while_backend_shell_is_pending() {
    let candidate = CompletionCandidate {
        display: "assemble".to_string(),
        replacement: "assemble".to_string(),
        is_dir: false,
        source: crate::completion::CompletionSource::History,
    };
    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:200:assistant".to_string()),
        completion_inline: Some(InlineCompletion {
            candidate: candidate.clone(),
            suffix: "emble".to_string(),
        }),
        pending_completion: Some(crate::app::PendingCompletion {
            id: 42,
            line: "nativecmd ass".to_string(),
            cursor: "nativecmd ass".len(),
            candidates: vec![candidate],
            backend_expected: true,
            backend_complete: false,
            backend_priority_deadline: Some(Instant::now() + Duration::from_secs(1)),
        }),
        ..AppState::default()
    };
    state.draft.insert_str("nativecmd ass");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "nativecmd ass");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn visible_history_completion_is_accepted_after_backend_priority_wait_expires() {
    let candidate = CompletionCandidate {
        display: "assemble".to_string(),
        replacement: "assemble".to_string(),
        is_dir: false,
        source: crate::completion::CompletionSource::History,
    };
    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:200:assistant".to_string()),
        completion_inline: Some(InlineCompletion {
            candidate: candidate.clone(),
            suffix: "emble".to_string(),
        }),
        pending_completion: Some(crate::app::PendingCompletion {
            id: 43,
            line: "nativecmd ass".to_string(),
            cursor: "nativecmd ass".len(),
            candidates: vec![candidate],
            backend_expected: true,
            backend_complete: false,
            backend_priority_deadline: Some(Instant::now() - Duration::from_millis(1)),
        }),
        ..AppState::default()
    };
    state.draft.insert_str("nativecmd ass");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "nativecmd assemble");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn backend_shell_completion_does_not_defer_local_path_hint() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:150:shellfile".to_string()),
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    state.draft.insert_str("cat s");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert_eq!(
        state.completion_inline.as_ref().unwrap().suffix,
        "ingle.txt"
    );
    assert_eq!(
        state.completion_inline.as_ref().unwrap().candidate.source,
        crate::completion::CompletionSource::Path
    );
    assert!(state.completion_panel.is_empty());
    wait_for_inline_suffix_with_attempts(&mut state, "hellfile", 200);
    assert_eq!(
        state.completion_inline.as_ref().unwrap().candidate.source,
        crate::completion::CompletionSource::BackendShell
    );
}

#[test]
fn tab_restarts_empty_live_request_as_explicit_backend_shell_completion() {
    let mut state = AppState {
        backend_shell: Some("aish-test-backend:remote".to_string()),
        ..AppState::default()
    };
    state.draft.insert_str("native r");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    let live_generation = state.completion_generation;

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
    assert_eq!(
        state
            .pending_completion
            .as_ref()
            .map(|pending| pending.candidates.as_slice()),
        Some([].as_slice())
    );

    complete_or_show_candidates(&mut state).unwrap();

    assert!(
        state.completion_generation > live_generation,
        "explicit Tab should replace an empty background request"
    );
    wait_for_inline_suffix_with_attempts(&mut state, "emote", 50);
    assert_eq!(
        state.completion_inline.as_ref().unwrap().candidate.source,
        crate::completion::CompletionSource::BackendShell
    );
}

#[test]
fn tab_mode_backend_shell_completion_appears_after_explicit_tab_without_aish_match() {
    let mut completion_config = CompletionConfig::default();
    completion_config.set_mode(CompletionMode::Tab);
    let mut state = AppState {
        backend_shell: Some("aish-test-backend:native-one,native-two".to_string()),
        completion_config,
        ..AppState::default()
    };
    state.draft.insert_str("nativecmd native");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "nativecmd native");
    wait_for_inline_suffix_with_attempts(&mut state, "-one", 50);
    assert!(
        state
            .completion_panel
            .iter()
            .any(|row| row.starts_with("shell nativecmd native-two")),
        "{:?}",
        state.completion_panel
    );
}

#[test]
#[cfg(unix)]
fn tab_accepts_first_token_executable_while_slow_backend_shell_is_pending() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("aishrclone");
    std::fs::write(&executable, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old_path = std::env::var_os("PATH");
    unsafe {
        std::env::set_var("PATH", &bin);
    }

    let mut state = AppState {
        backend_shell: Some("aish-test-backend-delay-ms:250:aishremote".to_string()),
        ..AppState::default()
    };
    state.draft.insert_str("aishrcl");
    state.defer_completion_display(Instant::now());
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
    let tab_result = complete_or_show_candidates(&mut state);
    let draft_after_tab = state.draft.as_str().to_string();
    let inline_after_tab = state.completion_inline.clone();
    let panel_after_tab = state.completion_panel.clone();

    unsafe {
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }

    tab_result.unwrap();
    assert_eq!(draft_after_tab, "aishrclone");
    assert!(inline_after_tab.is_none());
    assert!(panel_after_tab.is_empty());
}

#[test]
fn tab_mode_history_completion_shows_after_explicit_tab() {
    let mut completion_config = CompletionConfig::default();
    completion_config.set_mode(CompletionMode::Tab);
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config,
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    complete_or_show_candidates(&mut state).unwrap();
    assert_eq!(
        state.completion_inline.as_ref().unwrap().suffix,
        "status --short"
    );
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "git status");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

fn wait_for_inline_suffix_with_attempts(state: &mut AppState, suffix: &str, attempts: usize) {
    let mut output = Vec::new();
    for _ in 0..attempts {
        refresh_after_background_events(state, &mut output).unwrap();
        if state
            .completion_inline
            .as_ref()
            .is_some_and(|inline| inline.suffix == suffix)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing inline suffix {suffix:?}; inline was {:?}, panel was {:?}",
        state.completion_inline, state.completion_panel
    );
}

fn wait_for_inline_source_with_attempts(
    state: &mut AppState,
    source: crate::completion::CompletionSource,
    attempts: usize,
) {
    let mut output = Vec::new();
    for _ in 0..attempts {
        refresh_after_background_events(state, &mut output).unwrap();
        if state
            .completion_inline
            .as_ref()
            .is_some_and(|inline| inline.candidate.source == source)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing inline source {source:?}; inline was {:?}, panel was {:?}",
        state.completion_inline, state.completion_panel
    );
}

#[test]
fn tab_mode_typo_completion_corrects_previous_word_before_accepting_suffix() {
    let mut completion_config = CompletionConfig::default();
    completion_config.set_mode(CompletionMode::Tab);
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config,
        ..AppState::default()
    };
    state.draft.insert_str("git statuz --");

    complete_or_show_candidates(&mut state).unwrap();
    wait_for_visible_completion_panel_contains(&mut state, "git status --short");
    assert!(state.completion_inline.is_none());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "git status --short");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn stale_async_completion_events_are_ignored_after_input_changes() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("git ");
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    state.draft.clear();
    state.draft.insert_str("# ");
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    let mut output = Vec::new();
    for _ in 0..20 {
        refresh_after_background_events(&mut state, &mut output).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn no_match_completion_leaves_completion_ui_empty() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config: CompletionConfig {
            inline: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("zzzzzz-no-match");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "zzzzzz-no-match");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn auto_live_completion_shows_remaining_candidates_as_panel_hints() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("one.txt"), "").unwrap();
    std::fs::write(temp.path().join("only.log"), "").unwrap();
    state.draft.insert_str("cat o");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ne.txt");
    assert_eq!(
        state.completion_panel,
        vec!["file cat only.log".to_string()]
    );
}

#[test]
fn auto_live_completion_prefers_matching_directory_over_async_history() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "cat src-file-from-history".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("cat sr");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "c/");

    let mut output = Vec::new();
    for _ in 0..50 {
        refresh_after_background_events(&mut state, &mut output).unwrap();
        if state
            .completion_panel
            .iter()
            .any(|row| row.contains("src-file-from-history"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "c/");
    assert!(
        state
            .completion_panel
            .iter()
            .any(|row| row.contains("src-file-from-history"))
    );

    assert!(accept_first_completion(&mut state).unwrap());
    assert_eq!(state.draft.as_str(), "cat src/");
}

#[test]
fn tab_mode_directory_typo_completion_accepts_corrected_directory() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    let mut completion_config = CompletionConfig::default();
    completion_config.set_mode(CompletionMode::Tab);
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config,
        ..AppState::default()
    };
    state.draft.insert_str("cat srd");

    complete_or_show_candidates(&mut state).unwrap();

    assert!(state.completion_inline.is_none());
    assert!(
        state
            .completion_panel
            .iter()
            .any(|row| row.contains("cat src/"))
    );

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat src/");
}

#[test]
fn inline_completion_uses_first_prefix_candidate_when_earlier_candidate_is_panel_only() {
    let mut state = AppState::default();
    state.draft.insert_str("cat al");
    let candidates = vec![
        CompletionCandidate {
            display: "beta-alpha".to_string(),
            replacement: "beta-alpha".to_string(),
            is_dir: false,
            source: crate::completion::CompletionSource::History,
        },
        CompletionCandidate {
            display: "alpha.txt".to_string(),
            replacement: "alpha.txt".to_string(),
            is_dir: false,
            source: crate::completion::CompletionSource::Path,
        },
    ];

    set_completion_ui_from_candidates(&mut state, candidates, 80);

    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "pha.txt");
    assert_eq!(
        state.completion_panel,
        vec!["history cat beta-alpha".to_string()]
    );
}

#[test]
fn first_tab_accepts_live_inline_completion() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat single.txt");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn live_inline_completion_respects_inline_disabled_config() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config: CompletionConfig {
            inline: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn completion_enabled_false_disables_live_and_stale_inline_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert!(state.completion_inline.is_some());

    state.completion_config.enabled = false;
    assert!(!accept_first_completion(&mut state).unwrap());
    assert_eq!(state.draft.as_str(), "cat si");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    complete_or_show_candidates(&mut state).unwrap();
    assert_eq!(state.draft.as_str(), "cat si");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn completion_fuzzy_false_keeps_structural_history_completion() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config: CompletionConfig {
            fuzzy: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("git ");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    wait_for_inline_suffix(&mut state, "status --short");

    assert_eq!(
        state.completion_inline.as_ref().unwrap().suffix,
        "status --short"
    );
}

#[test]
fn tab_mode_first_tab_shows_candidates_second_tab_accepts_first_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config: CompletionConfig {
            inline: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    std::fs::write(temp.path().join("single.txt"), "").unwrap();
    state.draft.insert_str("cat si");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat si");
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
    assert!(state.completion_panel.is_empty());

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "cat single.txt");
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn tab_shows_multiple_completion_candidates_below_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        ..AppState::default()
    };
    std::fs::write(temp.path().join("one.txt"), "").unwrap();
    std::fs::write(temp.path().join("only.log"), "").unwrap();
    state.draft.insert_str("cat o");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(
        state.completion_panel,
        vec!["file cat only.log".to_string()]
    );
    assert_eq!(state.draft.as_str(), "cat o");
}

#[test]
fn tab_display_respects_completion_max_results() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState {
        current_cwd: Some(temp.path().to_path_buf()),
        completion_config: CompletionConfig {
            max_results: 1,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    std::fs::write(temp.path().join("one.txt"), "").unwrap();
    std::fs::write(temp.path().join("only.log"), "").unwrap();
    state.draft.insert_str("cat o");

    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.completion_panel.len(), 1);
    assert!(state.completion_panel[0].starts_with("file "));
}

#[test]
fn editing_after_completion_panel_clears_panel() {
    let mut state = AppState {
        completion_panel: vec!["exec\tgit".to_string()],
        completion_inline: Some(InlineCompletion {
            candidate: crate::completion::CompletionCandidate {
                display: "git status".to_string(),
                replacement: "git status".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::History,
            },
            suffix: " status".to_string(),
        }),
        ..AppState::default()
    };
    state.draft.insert_str("git");

    apply_key_to_state(key(KeyCode::Char('x')), &mut state);

    assert!(state.completion_panel.is_empty());
    assert!(state.completion_inline.is_none());
}

#[test]
fn write_completion_candidates_prints_labeled_rows() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    crate::templates::append_template(
        &template_path,
        &crate::templates::TemplateEntry::new("git add . && git commit"),
    )
    .unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("git");
    let mut output = Vec::new();

    write_completion_candidates(&state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("template git add . && git commit"));
}

#[test]
fn right_at_end_requests_visible_completion_accept_without_editing_immediately() {
    let mut state = AppState::default();
    state.draft.insert_str("git");
    state.completion_inline = Some(InlineCompletion {
        candidate: CompletionCandidate {
            display: "git status".to_string(),
            replacement: "git status".to_string(),
            is_dir: false,
            source: crate::completion::CompletionSource::History,
        },
        suffix: " status".to_string(),
    });

    assert_eq!(
        apply_key_to_state(key(KeyCode::Right), &mut state),
        KeyAction::AcceptCompletion
    );

    assert_eq!(state.draft.as_str(), "git");
}

#[test]
fn right_at_end_without_visible_completion_keeps_cursor_behavior() {
    let mut state = AppState::default();
    state.draft.insert_str("git");

    assert_eq!(
        apply_key_to_state(key(KeyCode::Right), &mut state),
        KeyAction::Continue
    );

    assert_eq!(state.draft.as_str(), "git");
}

#[test]
fn right_inside_line_keeps_cursor_movement_behavior() {
    let mut state = AppState::default();
    state.draft.insert_str("git status");
    state.draft.move_start();

    assert_eq!(
        apply_key_to_state(key(KeyCode::Right), &mut state),
        KeyAction::Continue
    );

    assert_eq!(state.draft.cursor(), 1);
}

#[test]
fn accept_first_completion_replaces_current_token() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config: CompletionConfig {
            inline: false,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("git sta");

    assert!(accept_first_completion(&mut state).unwrap());

    assert_eq!(state.draft.as_str(), "git status");
    assert_eq!(state.draft.cursor(), "git status".len());
}

#[test]
fn tab_accept_word_mode_accepts_only_next_word_from_inline_suggestion() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "kubectl apply -f deployment.yaml".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config: CompletionConfig {
            tab_accept: CompletionTabAccept::Word,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("kub");

    complete_or_show_candidates(&mut state).unwrap();
    wait_for_inline_suffix(&mut state, "ectl apply -f deployment.yaml");
    complete_or_show_candidates(&mut state).unwrap();

    assert_eq!(state.draft.as_str(), "kubectl");
    assert_eq!(state.draft.cursor(), "kubectl".len());
}

#[test]
fn right_accepts_inline_completion_when_available() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        ..AppState::default()
    };
    state.draft.insert_str("git");
    complete_or_show_candidates(&mut state).unwrap();
    let mut output = Vec::new();
    for _ in 0..50 {
        refresh_after_background_events(&mut state, &mut output).unwrap();
        if state
            .completion_inline
            .as_ref()
            .is_some_and(|inline| inline.suffix == " status")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(accept_first_completion(&mut state).unwrap());

    assert_eq!(state.draft.as_str(), "git status");
    assert!(state.completion_inline.is_none());
}

#[test]
fn tab_template_placeholder_name_completion_accepts_raw_placeholder() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates.jsonl");
    crate::templates::append_template(
        &template_path,
        &crate::templates::TemplateEntry::new("echo {something}"),
    )
    .unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("echo something");

    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
    wait_for_visible_completion_panel_contains(&mut state, "{something}");

    assert!(state.completion_inline.is_none());
    assert!(
        state
            .completion_panel
            .iter()
            .any(|row| row.contains("{something}"))
    );

    complete_or_show_candidates_for_width(&mut state, 80).unwrap();

    assert_eq!(state.draft.as_str(), "echo {something}");
    assert!(state.draft_from_template);
    assert!(state.completion_inline.is_none());
    assert!(state.completion_panel.is_empty());
}

#[test]
fn right_accepts_structural_template_completion_as_protected_template_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates.jsonl");
    crate::templates::append_template(
        &template_path,
        &crate::templates::TemplateEntry::new("echo {something}"),
    )
    .unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("echo something");

    assert!(accept_first_completion(&mut state).unwrap());

    assert_eq!(state.draft.as_str(), "echo {something}");
    assert!(state.draft_from_template);
}
