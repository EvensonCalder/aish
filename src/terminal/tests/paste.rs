use super::*;

#[test]
fn normalize_paste_newlines_canonicalizes_crlf_and_cr() {
    assert_eq!(
        normalize_paste_newlines("one\r\ntwo\rthree"),
        "one\ntwo\nthree"
    );
    assert_eq!(normalize_paste_newlines("one\r\n"), "one");
}

#[test]
fn single_line_paste_inserts_into_draft() {
    let mut state = AppState::default();
    state.draft.insert_str("git ");

    assert_eq!(
        apply_paste_to_state("status", &mut state),
        PasteAction::Continue
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert!(!state.draft_from_editor);
}

#[test]
fn single_line_paste_copies_history_selection_first() {
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

    assert_eq!(apply_paste_to_state("s", &mut state), PasteAction::Continue);

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git status");
    assert!(!state.draft_from_editor);
}

#[test]
fn multiline_paste_creates_opaque_editor_draft() {
    let mut state = AppState::default();

    assert_eq!(
        apply_paste_to_state("echo one\r\necho two", &mut state),
        PasteAction::Continue
    );

    assert_eq!(state.mode, Mode::Draft);
    assert!(state.draft_from_editor);
    assert!(state.draft_has_paste_preview);
    assert_eq!(state.draft.as_str(), "echo one\necho two");
    assert!(state.render_prompt_line().contains("[draft: 2 lines"));
    assert!(state.render_prompt_line().contains("paste preview:"));
    assert!(state.render_prompt_line().contains("  echo one"));
    assert!(state.render_prompt_line().contains("  echo two"));
}

#[test]
fn multiline_paste_preview_escapes_control_bytes_and_keeps_cursor_on_summary() {
    let mut state = AppState {
        paste_config: crate::config::PasteConfig {
            preview_lines: 2,
            preview_bytes: 100,
            ..crate::config::PasteConfig::default()
        },
        ..AppState::default()
    };

    assert_eq!(
        apply_paste_to_state("printf '\x1b[31m'\tred\nnext", &mut state),
        PasteAction::Continue
    );

    let rendered = state.render_prompt_line();
    assert!(rendered.contains("  printf '\\x1b[31m'\\tred"));
    assert!(!rendered.contains('\x1b'));
    let summary = format!("> {}", state.editor_draft_summary_for_terminal());
    assert_eq!(
        state.terminal_cursor_column(),
        display_width(&summary) as u16
    );
}

#[test]
fn multiline_paste_preview_can_be_disabled() {
    let mut state = AppState {
        paste_config: crate::config::PasteConfig {
            preview: false,
            ..crate::config::PasteConfig::default()
        },
        ..AppState::default()
    };

    assert_eq!(
        apply_paste_to_state("echo one\necho two", &mut state),
        PasteAction::Continue
    );

    assert!(state.draft_has_paste_preview);
    assert!(!state.render_prompt_line().contains("paste preview:"));
}

#[test]
fn pasted_single_line_with_trailing_newline_inserts_without_review() {
    let mut state = AppState::default();

    assert_eq!(
        apply_paste_to_state("echo pasted\n", &mut state),
        PasteAction::Continue
    );

    assert_eq!(state.draft.as_str(), "echo pasted");
    assert!(!state.draft_from_editor);
}

#[test]
fn multiline_paste_discard_config_ignores_content() {
    let mut state = AppState {
        paste_config: crate::config::PasteConfig {
            multiline: "discard".to_string(),
            confirm_execute: true,
            ..crate::config::PasteConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("existing");

    assert_eq!(
        apply_paste_to_state("echo one\necho two", &mut state),
        PasteAction::Continue
    );

    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "existing");
    assert!(!state.draft_from_editor);
}

#[test]
fn multiline_paste_execute_with_confirm_creates_editor_draft() {
    let mut state = AppState {
        paste_config: crate::config::PasteConfig {
            multiline: "execute".to_string(),
            confirm_execute: true,
            ..crate::config::PasteConfig::default()
        },
        ..AppState::default()
    };

    assert_eq!(
        apply_paste_to_state("echo one\necho two", &mut state),
        PasteAction::Continue
    );

    assert!(state.draft_from_editor);
    assert_eq!(state.draft.as_str(), "echo one\necho two");
}

#[test]
fn multiline_paste_execute_without_confirm_requests_submit() {
    let mut state = AppState {
        paste_config: crate::config::PasteConfig {
            multiline: "execute".to_string(),
            confirm_execute: false,
            ..crate::config::PasteConfig::default()
        },
        ..AppState::default()
    };

    assert_eq!(
        apply_paste_to_state("echo one\necho two", &mut state),
        PasteAction::Submit
    );

    assert!(state.draft_from_editor);
    assert_eq!(state.draft.as_str(), "echo one\necho two");
}
