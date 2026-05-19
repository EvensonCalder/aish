use super::*;

#[test]
fn append_only_typing_renders_incrementally_without_full_redraw() {
    let mut completion_config = CompletionConfig::default();
    completion_config.set_mode(CompletionMode::Off);
    let mut state = AppState {
        completion_config,
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();
    redraw(&mut state, &mut output).unwrap();
    output.clear();

    handle_key(
        key(KeyCode::Char('x')),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(state.draft.as_str(), "x");
    assert_eq!(String::from_utf8(output).unwrap(), "x");
    assert!(state.render_anchor_saved);
}

#[test]
fn redraw_renders_completion_panel_below_prompt_and_restores_cursor() {
    let mut state = AppState::default();
    state.draft.insert_str("git");
    state.completion_panel = vec!["exec\tgit".to_string(), "exec\tgit-shell".to_string()];
    let mut output = Vec::new();

    redraw(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("> git\r\nexec\tgit\r\nexec\tgit-shell"));
    assert!(output.contains("\u{1b}7"), "output was {output:?}");
    assert!(output.contains("\u{1b}8"), "output was {output:?}");
    assert!(output.ends_with("\u{1b}[6G"), "output was {output:?}");
}

#[test]
fn redraw_reserves_space_before_drawing_panel_at_screen_bottom() {
    let mut state = AppState::default();
    state.draft.insert_str("sudo");
    state.completion_panel = vec![
        "exec sudo_logsrvd".to_string(),
        "exec sudo_sendlog".to_string(),
        "exec sudoedit".to_string(),
        "exec sudoreplay".to_string(),
    ];
    let mut output = b"\r\n\r\n\r\n\r\n".to_vec();

    redraw_for_size(&mut state, &mut output, 80, 5).unwrap();
    redraw_for_size(&mut state, &mut output, 80, 5).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    let screen = TestScreen::from_output_with_height(&rendered, 5);
    assert_eq!(screen.line(0), "> sudo");
    assert_eq!(screen.line(1), "exec sudo_logsrvd");
    assert_eq!(screen.line(4), "exec sudoreplay");
    assert!(
        screen
            .scrollback_lines()
            .iter()
            .all(|line| !line.contains("> sudo") && !line.contains("exec sudo")),
        "scrollback was {:?}",
        screen.scrollback_lines()
    );
}

#[test]
fn redraw_positions_cursor_from_anchor_at_wrap_boundary() {
    let mut state = AppState::default();
    state.draft.insert_str("ab");
    let mut output = Vec::new();

    redraw_for_width(&mut state, &mut output, 4).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("> ab"), "output was {output:?}");
    assert!(
        output.ends_with("\u{1b}8\u{1b}[1B\u{1b}[1G"),
        "output was {output:?}"
    );
}

#[test]
fn redraw_positions_cursor_from_anchor_at_cjk_wrap_boundary() {
    let mut state = AppState::default();
    state.draft.insert_str("a中b");
    let mut output = Vec::new();

    redraw_for_width(&mut state, &mut output, 6).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("> a中b"), "output was {output:?}");
    assert!(
        output.ends_with("\u{1b}8\u{1b}[1B\u{1b}[1G"),
        "output was {output:?}"
    );
}

#[test]
fn redraw_renders_inline_completion_suffix_without_moving_cursor() {
    let mut state = AppState::default();
    state.draft.insert_str("cat Car");
    state.completion_inline = Some(InlineCompletion {
        candidate: crate::completion::CompletionCandidate {
            display: "Cargo.toml".to_string(),
            replacement: "Cargo.toml".to_string(),
            is_dir: false,
            source: crate::completion::CompletionSource::Path,
        },
        suffix: "go.toml".to_string(),
    });
    let mut output = Vec::new();

    redraw(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("> cat Car"), "output was {output:?}");
    assert!(output.contains("go.toml"), "output was {output:?}");
    assert!(output.ends_with("\u{1b}[10G"), "output was {output:?}");
}

#[test]
fn inline_completion_suffix_elides_to_terminal_width() {
    let mut state = AppState::default();
    state.draft.insert_str("cat very");
    state.completion_inline = Some(InlineCompletion {
        candidate: crate::completion::CompletionCandidate {
            display: "very-long-target.txt".to_string(),
            replacement: "very-long-target.txt".to_string(),
            is_dir: false,
            source: crate::completion::CompletionSource::Path,
        },
        suffix: "-long-target.txt".to_string(),
    });

    assert_eq!(
        render_inline_completion_suffix(&state, "> cat very-l...".len()),
        Some("-l...".to_string())
    );
}

#[test]
fn redraw_positions_cursor_on_multiline_draft_last_line() {
    let mut state = AppState::default();
    state.draft.insert_str("echo \"\n123");
    let mut output = Vec::new();

    redraw(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("> echo \"\r\n.. 123"),
        "output was {output:?}"
    );
    assert!(output.ends_with("\u{1b}[7G"), "output was {output:?}");
}

#[test]
fn empty_ctrl_d_prints_exit_on_own_line_and_final_newline() {
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    assert!(
        handle_key(
            ctrl('d'),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap()
    );

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("\r\nexit\r\n"),
        "exit should start on its own line: {output:?}"
    );
    assert!(output.ends_with("exit\r\n"), "output was {output:?}");
}

#[test]
fn submit_moves_cursor_to_prompt_line_end_before_newline() {
    let mut state = AppState::default();
    state.draft.insert_str("echo hello");
    state.draft.move_start();
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

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("\u{1b}[13G\r\nhello"),
        "output was {output:?}"
    );
}

#[test]
fn submit_redraws_without_inline_ghost_suffix() {
    let mut state = AppState {
        regular_history: vec![crate::history::HistoryEntry {
            t: 1,
            command: "echo inline-history seeded".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        }],
        completion_config: CompletionConfig {
            tab_accept: CompletionTabAccept::Word,
            ..CompletionConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("echo in");
    refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
    wait_for_inline_suffix(&mut state, "line-history seeded");
    complete_or_show_candidates(&mut state).unwrap();
    wait_for_inline_suffix(&mut state, " seeded");
    assert_eq!(state.draft.as_str(), "echo inline-history");
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, " seeded");
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

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("> echo inline-history"),
        "output was {output:?}"
    );
    assert!(
        output.contains("\r\ninline-history"),
        "output was {output:?}"
    );
    assert!(
        !output.contains("echo inline-history seeded\r\ninline-history"),
        "output was {output:?}"
    );
}

#[test]
fn submit_normalizes_multiline_shell_output_for_raw_terminal() {
    let mut state = AppState::default();
    state.draft.insert_str("printf 'one\\ntwo\\n'");
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

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("one\r\ntwo"), "output was {output:?}");
    assert!(!output.contains("one\ntwo\n"), "output was {output:?}");
    assert!(
        !output.contains("one\r\ntwo\r\n\r\n"),
        "output was {output:?}"
    );
}

#[test]
fn submit_output_stays_visible_above_redrawn_prompt() {
    let mut state = AppState::default();
    state.draft.insert_str("echo hello");
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

    let rendered = String::from_utf8(output).unwrap();
    let screen = TestScreen::from_output(&rendered);
    let prompt_row = screen
        .rows
        .iter()
        .position(|row| row.iter().collect::<String>() == "> ")
        .expect("redrawn prompt row");
    assert!(prompt_row > 0, "screen was {:?}", screen.lines());
    assert_eq!(screen.line(prompt_row - 1), "hello");
}

#[test]
fn submit_after_completion_panel_keeps_output_adjacent_to_command_line() {
    let mut state = AppState::default();
    state.draft.insert_str("echo hello");
    state.completion_panel = vec![
        "exec\techo".to_string(),
        "exec\techoctl".to_string(),
        "exec\techoed".to_string(),
    ];
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    redraw(&mut state, &mut output).unwrap();
    handle_key(
        key(KeyCode::Enter),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let rendered = String::from_utf8(output).unwrap();
    let screen = TestScreen::from_output(&rendered);
    assert_eq!(screen.line(0), "> echo hello");
    assert_eq!(screen.line(1), "hello");
    assert_eq!(screen.line(2), "> ");
}

#[test]
fn submit_cancels_hidden_completion_request_before_command_output() {
    let candidate = CompletionCandidate {
        display: "echo hidden-history".to_string(),
        replacement: "echo hidden-history".to_string(),
        is_dir: false,
        source: crate::completion::CompletionSource::History,
    };
    let mut state = AppState::default();
    state.draft.insert_str("echo hello");
    state.pending_completion = Some(crate::app::PendingCompletion {
        id: 9,
        line: "echo hello".to_string(),
        cursor: "echo hello".len(),
        candidates: vec![candidate.clone()],
    });
    state.pending_completion_update = Some(crate::app::PendingCompletionUpdate {
        id: 9,
        line: "echo hello".to_string(),
        cursor: "echo hello".len(),
        candidates: vec![candidate],
        first_seen: Instant::now(),
        final_tier_seen: true,
    });
    state.completion_display_not_before = Some(Instant::now() + Duration::from_secs(1));
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

    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.contains("hello"), "output was {rendered:?}");
    assert!(
        !rendered.contains("hidden-history"),
        "hidden completion leaked into output: {rendered:?}"
    );
    assert!(state.pending_completion.is_none());
    assert!(state.pending_completion_update.is_none());
    assert!(state.completion_display_not_before.is_none());
}

#[test]
fn clear_screen_moves_to_top_left_before_redraw() {
    let mut state = AppState {
        last_rendered_lines: 3,
        ..AppState::default()
    };
    let mut output = Vec::new();

    clear_screen_for_redraw(&mut state, &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert!(!rendered.starts_with("\r\n"));
    assert!(!rendered.starts_with('\n'));
    assert!(rendered.contains("\x1b[2J"));
    assert!(rendered.contains("\x1b[3J"));
    assert_eq!(state.last_rendered_lines, 0);
}

#[test]
fn ctrl_l_redraw_does_not_emit_leading_blank_line() {
    let mut state = AppState::default();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    handle_key(
        ctrl('l'),
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let rendered = String::from_utf8(output).unwrap();
    let screen = TestScreen::from_output(&rendered);
    assert_eq!(screen.line(0), "> ");
    assert_eq!(screen.first_non_empty_line(), Some(0));
}

#[test]
fn clear_like_command_output_redraws_prompt_on_first_screen_line() {
    let mut state = AppState::default();
    state.draft.insert_str("printf '\\033[H\\033[2J'");
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

    let rendered = String::from_utf8(output).unwrap();
    let screen = TestScreen::from_output(&rendered);
    assert_eq!(screen.line(0), "> ");
    assert_eq!(screen.first_non_empty_line(), Some(0));
}
