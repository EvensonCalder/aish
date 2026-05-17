use super::encryption_commands::{StoredApiKey, write_history_rewrite_script};
use super::state::OUTPUT_RING_CAPACITY;
use super::sync_commands::{run_startup_sync_check, write_last_sync_attempt};
use super::*;
use crate::completion::{CompletionCandidate, CompletionSource};
use crate::config::{
    self, AiConfig, CompletionConfig, CompletionMode, CompletionTabAccept, ContextConfig,
    EditorConfig, EncryptionConfig, PromptConfig, SyncConfig,
};
use crate::display_width::display_width;
use crate::editor::EditorCommand;
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::encryption::{gpg_decrypt_file, load_encrypted_jsonl, rewrite_encrypted_jsonl};
use crate::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, HistoryEntry, HistorySource, NoteEntry,
    append_jsonl, load_jsonl,
};
use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event, load_events};
use crate::modes::Mode;
use crate::pty::PtyBackend;
use crate::templates::{TemplateEntry, append_template, load_templates, template_id};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn write_fake_gpg(temp: &tempfile::TempDir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_gpg = temp.path().join("fake-gpg");
    fs::write(
            &fake_gpg,
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nrecipient=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\nlast=\"\"\nfor arg in \"$@\"; do\n  last=\"$arg\"\n  if [ \"$arg\" = \"--version\" ]; then printf 'fake gpg\\n'; exit 0; fi\ndone\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--list-keys\" ]; then\n    fpr='AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'\n    uid='Test User <test@example.invalid>'\n    case \"$last\" in\n      *BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB*|second@example.invalid) fpr='BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB'; uid='Second User <second@example.invalid>' ;;\n    esac\n    printf '%s\\n' 'pub:u:255:22:1111111111111111:1:::u:::scESC::::::23::0:'\n    printf 'fpr:::::::::%s:\\n' \"$fpr\"\n    printf 'uid:u::::1::hash::%s:::::::::0:\\n' \"$uid\"\n    exit 0\n  fi\ndone\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient) shift; recipient=\"$1\" ;;\n    --trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|--with-colons|--fingerprint) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  sed '1{/^recipient:/d;}' \"$input\"\nelse\n  { printf 'recipient:%s\\n' \"$recipient\"; cat \"$input\"; } > \"$out\"\nfi\n",
        )
        .unwrap();
    fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();
    fake_gpg
}

#[cfg(unix)]
fn write_blocking_fake_gpg(temp: &tempfile::TempDir, release_path: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_gpg = temp.path().join("blocking-gpg");
    fs::write(
            &fake_gpg,
            format!(
                "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  while [ ! -f '{}' ]; do sleep 0.02; done\n  cp \"$input\" \"$out\"\nfi\n",
                release_path.display()
            ),
        )
        .unwrap();
    fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();
    fake_gpg
}

fn ai_requester_requires_stored_key(config: &AiConfig, _prompt: &str) -> Result<Vec<AiItem>> {
    assert_eq!(config.api_key_override.as_deref(), Some("secret-test-key"));
    assert_eq!(config.model, "test-model");
    Ok(vec![AiItem {
        kind: AiItemKind::Command,
        text: "pwd".to_string(),
        name: None,
    }])
}

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
fn command_output_does_not_add_newline_after_clear_home_sequence() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[H\x1b[2J\x1b[3J\x1b[H").unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\x1b[H\x1b[2J\x1b[3J\x1b[H\x1b[H"
    );
}

#[test]
fn command_output_does_not_add_newline_after_common_clear_sequence() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[H\x1b[2J").unwrap();

    assert_eq!(String::from_utf8(output).unwrap(), "\x1b[H\x1b[2J\x1b[H");
}

#[test]
fn command_output_homes_cursor_after_terminfo_clear_sequence() {
    let mut output = Vec::new();

    write_command_output(&mut output, "\x1b[3J\x1b[H\x1b[2J").unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\x1b[3J\x1b[H\x1b[2J\x1b[H"
    );
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
    assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
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
    assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
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
    assert_eq!(std::fs::read_to_string(session.path).unwrap(), "git status");
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

#[test]
fn private_exit_requests_app_exit() {
    let mut state = AppState::default();
    state.draft.insert_str("#exit");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(state.exit_requested);
    assert!(state.draft.is_empty());
    assert!(output.is_empty());
}

#[test]
fn private_help_prints_available_commands() {
    let mut state = AppState::default();
    state.draft.insert_str("#help");
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
    assert!(output.contains("Aish help"));
    assert!(output.contains("Usage:"));
    assert!(output.contains("#help [topic]"));
    assert!(output.contains("Topics:"));
    assert!(output.contains("commands, keys, ai, completion, templates, sync, encryption, config"));
    assert!(output.contains("Private commands:"));
    assert!(output.contains("#help"));
    assert!(output.contains("#status"));
    assert!(output.contains("#config"));
    assert!(output.contains("#doctor"));
    assert!(output.contains("#prompt"));
    assert!(output.contains("#model"));
    assert!(output.contains("#base-url"));
    assert!(output.contains("#env-key"));
    assert!(output.contains("#key"));
    assert!(output.contains("#context"));
    assert!(output.contains("#completion"));
    assert!(output.contains("#log"));
    assert!(output.contains("#editor"));
    assert!(output.contains("#mt"));
    assert!(output.contains("#template"));
    assert!(output.contains("#encrypt"));
    assert!(output.contains("#set-remote"));
    assert!(output.contains("#push"));
    assert!(output.contains("#sync"));
    assert!(output.contains("#exit"));
    assert!(output.contains("#quit"));
    assert!(output.contains("#history"));
    assert!(output.contains("Keybindings:"));
    assert!(
        output.contains(
            "Tab - empty draft cycles modes; non-empty draft shows or accepts completion"
        )
    );
    assert!(output.contains("Ctrl-X Ctrl-E - open the configured external editor"));
    assert!(output.contains("AI and notes:"));
    assert!(output.contains("# <prompt> - send an AI prompt"));
    assert!(output.contains("# TODO: <text> - store a note"));
    assert!(state.draft.is_empty());
}

#[test]
fn private_help_prints_topic_specific_usage() {
    let mut state = AppState::default();
    state.draft.insert_str("#help completion");
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
    assert!(output.contains("Completion help"));
    assert!(output.contains("#completion mode auto|tab|off"));
    assert!(output.contains("#completion display-delay-ms <0-1000>"));
    assert!(output.contains("#completion tab-accept full|word"));
    assert!(output.contains("auto shows live hints while typing"));
    assert!(!output.contains("Sync help"));
}

#[test]
fn private_help_rejects_unknown_topic_without_running_shell() {
    let mut state = AppState::default();
    state.draft.insert_str("#help unknown-topic");
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
    assert!(output.contains("unknown help topic: unknown-topic"));
    assert!(
        output.contains(
            "usage: #help [commands|keys|ai|completion|templates|sync|encryption|config]"
        )
    );
    assert!(state.draft.is_empty());
}

#[test]
fn private_context_reports_current_config() {
    let mut state = AppState::default();
    state.draft.insert_str("#context");
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
    assert!(output.contains("context.enabled=true"));
    assert!(output.contains("context.confirm=true"));
    assert!(output.contains("context.max_bytes=65536"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn private_context_commands_persist_config() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#context off", "context.enabled=false"),
        ("#context confirm off", "context.confirm=false"),
        ("#context 1024", "context.max_bytes=1024"),
        ("#context on", "context.enabled=true"),
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

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }

    assert!(state.context_config.enabled);
    assert!(!state.context_config.confirm);
    assert_eq!(state.context_config.max_bytes, 1024);
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.context, state.context_config);
}

#[test]
fn private_context_rejects_invalid_usage_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#context 0");
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
    assert!(output.contains("context max bytes must be greater than 0"));
    assert_eq!(state.context_config, ContextConfig::default());
    assert_eq!(
        config::load_config(&config_path).unwrap().context,
        ContextConfig::default()
    );
}

#[test]
fn ai_prompt_with_context_waits_for_confirmation_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("# explain < printf context");
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
    assert!(output.contains("aish will run this command to collect context"));
    assert!(output.contains("Run context command? [Y/n]"));
    assert!(output.contains("answer Y to run context command or n to skip"));
    assert_eq!(
        state.pending_context,
        Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: false,
        })
    );
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "context command requires confirmation");
}

#[test]
fn ai_prompt_with_context_disabled_does_not_execute_command() {
    let mut state = AppState {
        context_config: ContextConfig {
            enabled: false,
            ..ContextConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("# explain < printf context");
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
    assert!(output.contains("context collection is disabled"));
    assert!(output.contains("context command not executed: printf context"));
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
}

#[test]
fn ai_prompt_with_context_blocks_dangerous_command_even_without_confirmation() {
    let mut state = AppState {
        context_config: ContextConfig {
            confirm: false,
            ..ContextConfig::default()
        },
        ..AppState::default()
    };
    state.draft.insert_str("# explain < rm -rf /tmp/aish-test");
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
    assert!(output.contains("dangerous context command requires confirmation"));
    assert_eq!(
        state.pending_context,
        Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "rm -rf /tmp/aish-test".to_string(),
            dangerous: true,
        })
    );
    assert!(state.draft.is_empty());
    assert!(state.ai_sessions.is_empty());
}

#[test]
fn answer_context_confirmation_can_skip_pending_command() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        events_path: Some(events_path.clone()),
        pending_context: Some(PendingContextPrompt {
            prompt: "explain".to_string(),
            command: "printf context".to_string(),
            dangerous: false,
        }),
        ..AppState::default()
    };
    let mut output = Vec::new();

    answer_context_confirmation(&mut state, false, &mut output, Duration::from_secs(5)).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("context command skipped: printf context"));
    assert_eq!(state.pending_context, None);
    assert!(state.ai_sessions.is_empty());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "context command skipped");
}

#[test]
fn private_log_prints_recent_events() {
    let temp = tempfile::tempdir().unwrap();
    let events_path = temp.path().join("logs/events.jsonl");
    append_event(&events_path, 1, EventLevel::Info, "one", DEFAULT_MAX_EVENTS).unwrap();
    append_event(&events_path, 2, EventLevel::Warn, "two", DEFAULT_MAX_EVENTS).unwrap();
    let mut state = AppState {
        events_path: Some(events_path),
        ..AppState::default()
    };
    state.draft.insert_str("#log 1");
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
    assert!(!output.contains("one"));
    assert!(output.contains("2\tWarn\ttwo"));
}

#[test]
fn private_log_reports_usage_or_missing_storage() {
    for (line, expected) in [
        ("#log", "usage: #log <count>"),
        ("#log nope", "usage: #log <count>"),
        ("#log 1", "event log storage is not configured"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
    }
}

#[test]
fn ai_config_commands_persist_and_report_values() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#model test-model", "#model=test-model"),
        (
            "#base-url https://example.invalid/v1",
            "#base-url=https://example.invalid/v1/chat/completions",
        ),
        ("#env-key OPENAI_API_KEY", "#env-key=OPENAI_API_KEY"),
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

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.ai_config.model, "test-model");
    assert_eq!(
        state.ai_config.base_url,
        "https://example.invalid/v1/chat/completions"
    );
    assert_eq!(state.ai_config.env_key, "OPENAI_API_KEY");
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.ai, state.ai_config);
}

#[test]
fn ai_config_commands_report_unconfigured_without_config_path() {
    for (line, expected) in [
        ("#model", "#model=unconfigured"),
        ("#base-url", "#base-url=unconfigured"),
        ("#env-key", "#env-key=unconfigured"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }
}

#[test]
fn ai_config_write_errors_are_logged() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("bad-config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::write(&config_path, "not = [valid").unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#model test-model");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    let err = execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap_err();

    assert!(err.to_string().contains("invalid config"));
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 1);
    assert_eq!(events.items[0].level, EventLevel::Error);
    assert_eq!(events.items[0].msg, "config error");
}

#[test]
fn context_config_write_errors_are_logged() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("bad-config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::write(&config_path, "not = [valid").unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#context off");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    let err = execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap_err();

    assert!(err.to_string().contains("invalid config"));
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 1);
    assert_eq!(events.items[0].level, EventLevel::Error);
    assert_eq!(events.items[0].msg, "config error");
}

#[test]
fn key_commands_report_current_state_without_secret_side_effects() {
    for (line, expected) in [
        ("#key set", "key storage is not configured; no key stored"),
        (
            "#key clear",
            "key storage is not configured; no key removed",
        ),
        ("#key", "usage: #key set | #key clear"),
        ("#key rotate", "usage: #key set | #key clear"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}

#[test]
fn key_clear_removes_stored_encrypted_key_and_logs_event() {
    let temp = tempfile::tempdir().unwrap();
    let key_path = temp.path().join("secrets/key.json.gpg");
    let events_path = temp.path().join("logs/events.jsonl");
    std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
    std::fs::write(&key_path, b"encrypted-key-placeholder").unwrap();
    let mut state = AppState {
        secret_key_path: Some(key_path.clone()),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#key clear");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(!key_path.exists());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("stored key cleared")
    );
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 1);
    assert_eq!(events.items[0].level, EventLevel::Info);
    assert_eq!(events.items[0].msg, "stored key cleared");
}

#[test]
fn subsystem_commands_report_current_state() {
    for (line, expected) in [
        ("#completion", "completion.mode=auto"),
        ("#completion", "completion.max_results=5"),
        ("#completion", "completion.enabled=true"),
        ("#completion", "completion.coalesce_ms=50"),
        ("#completion", "completion.display_delay_ms=120"),
        ("#completion", "completion.fuzzy=true"),
        ("#editor", "editor temp directory is not configured"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}

#[test]
fn prompt_config_commands_persist_apply_and_reset() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        current_cwd: Some(repo),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#prompt", "prompt.draft=\"{mode} \""),
        (
            "#prompt draft \"[{basename}] > \"",
            "prompt.draft=\"[{basename}] > \"",
        ),
        (
            "#prompt history \"hist {mode} \"",
            "prompt.history=\"hist {mode} \"",
        ),
        ("#prompt ai 'ai {mode} '", "prompt.ai=\"ai {mode} \""),
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

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.render_prompt_line(), "[repo] > ");
    state.mode = Mode::History;
    assert_eq!(state.render_prompt_line(), "hist $ ");
    state.mode = Mode::Ai;
    assert_eq!(state.render_prompt_line(), "ai % ");
    state.mode = Mode::Draft;

    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.prompt.draft, "[{basename}] > ");
    assert_eq!(loaded.prompt.history, "hist {mode} ");
    assert_eq!(loaded.prompt.ai, "ai {mode} ");

    state.draft.insert_str("#prompt reset");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("prompt.draft=\"{user}@{host} {cwd} > \""));
    assert_eq!(state.prompt_templates, PromptConfig::default().into());
    assert_eq!(
        config::load_config(&config_path).unwrap().prompt,
        PromptConfig::default()
    );
}

#[test]
fn prompt_config_commands_report_usage_and_missing_config() {
    for (line, expected) in [
        (
            "#prompt draft",
            "usage: #prompt [draft|history|ai <template>|reset]",
        ),
        ("#prompt draft \"\"", "prompt template must not be empty"),
        (
            "#prompt nope value",
            "usage: #prompt [draft|history|ai <template>|reset]",
        ),
        (
            "#prompt draft \"x> \"",
            "config path is not configured; #prompt not saved",
        ),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }
}

#[test]
fn completion_config_commands_persist_and_reject_invalid_values() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#completion off", "completion.mode=off"),
        ("#completion on", "completion.mode=auto"),
        ("#completion mode tab", "completion.mode=tab"),
        ("#completion mode auto", "completion.mode=auto"),
        ("#completion max 2", "completion.max_results=2"),
        ("#completion coalesce-ms 75", "completion.coalesce_ms=75"),
        ("#completion coalesce 50", "completion.coalesce_ms=50"),
        (
            "#completion display-delay-ms 180",
            "completion.display_delay_ms=180",
        ),
        (
            "#completion display-delay 120",
            "completion.display_delay_ms=120",
        ),
        ("#completion inline off", "completion.mode=tab"),
        ("#completion tab-accept word", "completion.tab_accept=word"),
        ("#completion fuzzy off", "completion.fuzzy=false"),
        ("#completion fuzzy on", "completion.fuzzy=true"),
        (
            "#completion match-threshold 80",
            "completion.match_threshold_percent=80",
        ),
        (
            "#completion typo-threshold 85",
            "completion.typo_threshold_percent=85",
        ),
        (
            "#completion max 0",
            "completion max results must be greater than 0",
        ),
        ("#completion max nope", "usage: #completion max <count>"),
        (
            "#completion coalesce-ms 1001",
            "completion coalesce ms must be between 0 and 1000",
        ),
        (
            "#completion coalesce-ms nope",
            "usage: #completion coalesce-ms <0-1000>",
        ),
        (
            "#completion display-delay-ms 1001",
            "completion display delay ms must be between 0 and 1000",
        ),
        (
            "#completion display-delay-ms nope",
            "usage: #completion display-delay-ms <0-1000>",
        ),
        (
            "#completion inline maybe",
            "usage: #completion inline on|off",
        ),
        (
            "#completion mode manual",
            "usage: #completion mode auto|tab|off",
        ),
        ("#completion fuzzy maybe", "usage: #completion fuzzy on|off"),
        (
            "#completion tab-accept line",
            "usage: #completion tab-accept full|word",
        ),
        (
            "#completion match-threshold 101",
            "completion match threshold must be between 0 and 100",
        ),
        (
            "#completion match-threshold nope",
            "usage: #completion match-threshold <0-100>",
        ),
        (
            "#completion typo-threshold 101",
            "completion typo threshold must be between 0 and 100",
        ),
        (
            "#completion typo-threshold nope",
            "usage: #completion typo-threshold <0-100>",
        ),
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

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(state.draft.is_empty());
    }

    assert!(state.completion_config.enabled);
    assert_eq!(state.completion_config.mode(), CompletionMode::Tab);
    assert_eq!(state.completion_config.max_results, 2);
    assert_eq!(state.completion_config.coalesce_ms, 50);
    assert_eq!(state.completion_config.display_delay_ms, 120);
    assert!(!state.completion_config.inline);
    assert!(state.completion_config.fuzzy);
    assert_eq!(
        state.completion_config.tab_accept,
        CompletionTabAccept::Word
    );
    assert_eq!(state.completion_config.match_threshold_percent, 80);
    assert_eq!(state.completion_config.typo_threshold_percent, 85);
    let loaded = config::load_config(&config_path).unwrap().completion;
    assert!(loaded.enabled);
    assert_eq!(loaded.mode(), CompletionMode::Tab);
    assert_eq!(loaded.max_results, 2);
    assert_eq!(loaded.coalesce_ms, 50);
    assert_eq!(loaded.display_delay_ms, 120);
    assert!(!loaded.inline);
    assert!(loaded.fuzzy);
    assert_eq!(loaded.tab_accept, CompletionTabAccept::Word);
    assert_eq!(loaded.match_threshold_percent, 80);
    assert_eq!(loaded.typo_threshold_percent, 85);
}

#[test]
fn mt_command_persists_template_entry() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#mt rsync {from} {to}");
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
    let id = template_id("rsync {from} {to}");
    assert!(output.contains(&format!("template stored: {id}")));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].body, "rsync {from} {to}");
}

#[test]
fn template_list_is_intentionally_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template list");
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
    assert!(output.contains("template listing is intentionally not supported"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_find_prints_matching_hash_ids() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(&template_path, &TemplateEntry::new("rsync {from} {to}")).unwrap();
    append_template(&template_path, &TemplateEntry::new("tail -f {file}")).unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template find rsync");
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
    assert!(output.contains(&format!(
        "template {}\trsync {{from}} {{to}}",
        template_id("rsync {from} {to}")
    )));
    assert!(!output.contains("tail -f"));
}

#[test]
fn template_rm_removes_matching_templates() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["rsync {from} {to}", "tail -f {file}", "rsync {from} {to}"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    let id = template_id("rsync {from} {to}");
    state.draft.insert_str(&format!("#template rm {id}"));
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
    assert!(output.contains(&format!("template removed: {id} (2)")));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].body, "tail -f {file}");
}

#[test]
fn template_replace_rewrites_matching_templates() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["old deploy", "tail -f {file}", "old deploy"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    let old_id = template_id("old deploy");
    let new_id = template_id("new deploy body");
    state
        .draft
        .insert_str(&format!("#template replace {old_id} new deploy body"));
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
    assert!(output.contains(&format!(
        "template replaced: {old_id} -> {new_id} (removed 2)"
    )));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].body, "tail -f {file}");
    assert_eq!(loaded.items[1].body, "new deploy body");
}

#[test]
fn template_use_copies_newest_matching_body_to_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in [
        "old deploy",
        "tail -f {file}",
        "rsync {from} {user}@{host}:{to} {from}",
    ] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    let id = template_id("rsync {from} {user}@{host}:{to} {from}");
    state.draft.insert_str(&format!(
        "#template use {id} from=src host=prod to=/srv/app zextra=ignored aextra=unused"
    ));
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
    assert!(output.contains(&format!("template copied to draft: {id}")));
    assert!(output.contains("template placeholders: from, user, host, to"));
    assert!(output.contains("unresolved template placeholders: user"));
    assert!(output.contains("unused template values: aextra, zextra"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "rsync src {user}@prod:/srv/app src");
    assert_eq!(state.draft.cursor(), state.draft.as_str().len());
}

#[test]
fn template_use_reports_missing_template_without_changing_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template use missing");
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
    assert!(output.contains("template not found: missing"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_use_supports_quoted_values_with_spaces() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(
        &template_path,
        &TemplateEntry::new("echo {message} && cd {path}"),
    )
    .unwrap();
    let id = template_id("echo {message} && cd {path}");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!(
        "#template use {id} message=\"hello world\" path='/tmp/my dir'"
    ));
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
    assert!(output.contains(&format!("template copied to draft: {id}")));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo hello world && cd /tmp/my dir");
}

#[test]
fn template_use_supports_described_and_variadic_placeholders() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(
        &template_path,
        &TemplateEntry::new("git commit -m {message:commit message} -- {paths...}"),
    )
    .unwrap();
    let id = template_id("git commit -m {message:commit message} -- {paths...}");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!(
        "#template use {id} message='ship it' paths='src tests'"
    ));
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
    assert!(output.contains("template placeholders: message, paths"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git commit -m ship it -- src tests");
    assert!(state.draft_from_template);
}

#[test]
fn unresolved_template_placeholders_do_not_execute() {
    let mut state = AppState {
        draft_from_template: true,
        ..AppState::default()
    };
    state.draft.insert_str("echo {message}");
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
    assert!(output.contains("cannot execute unresolved template placeholders: message"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo {message}");
}

#[test]
fn template_show_prints_newest_matching_body() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["old deploy", "tail -f {file}", "new deploy"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let id = template_id("new deploy");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!("#template show {id}"));
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
    assert!(output.contains(&format!("template: {id}")));
    assert!(output.contains("new deploy"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_commands_report_usage_for_invalid_input() {
    let usage = template_usage();
    for (line, expected) in [
        ("#mt", "usage: #mt <template-body>"),
        ("#template rm", usage),
        ("#template replace deploy", usage),
        ("#template show", usage),
        ("#template use", usage),
        ("#template find", usage),
        ("#template", usage),
        ("#template unknown deploy", usage),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}

#[test]
fn encryption_and_sync_commands_report_current_state_without_side_effects() {
    for (line, expected) in [
        (
            "#encrypt on",
            "config path is not configured; #encrypt not saved",
        ),
        (
            "#set-remote git@example.invalid:aish.git",
            "config path is not configured; sync config not saved",
        ),
        (
            "#push",
            "sync remote is not configured; run #set-remote <git-url> first",
        ),
        ("#sync", "no git command run"),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
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
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}

#[test]
#[cfg(unix)]
fn encrypt_on_migrates_plaintext_storage_and_persists_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let ai_path = temp.path().join("history/ai.jsonl");
    let draft_path = temp.path().join("history/draft.jsonl");
    let notes_path = temp.path().join("history/notes.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    append_jsonl(
        &regular_path,
        &HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    append_jsonl(
        &ai_path,
        &AiSession {
            id: "ai-1".to_string(),
            t: 2,
            prompt: "list".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ls".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();
    append_jsonl(
        &draft_path,
        &DraftEntry {
            t: 3,
            text: "draft".to_string(),
        },
    )
    .unwrap();
    append_jsonl(
        &notes_path,
        &NoteEntry {
            tag: crate::commands::NoteTag::Note,
            text: "note".to_string(),
        },
    )
    .unwrap();
    append_template(&template_path, &TemplateEntry::new("echo {message}")).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        ai_history_path: Some(ai_path.clone()),
        draft_history_path: Some(draft_path.clone()),
        notes_path: Some(notes_path.clone()),
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt on test@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Encryption is now enabled for future writes."));
    assert!(output.contains("encryption=on"));
    assert!(state.encryption_config.enabled);
    assert_eq!(
        state.encryption_config.key_fingerprint,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    assert_eq!(state.encryption_config.recipient, "");
    let loaded = config::load_config(&config_path).unwrap();
    assert!(loaded.encryption.enabled);
    assert_eq!(
        loaded.encryption.key_fingerprint,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    assert_eq!(loaded.encryption.recipient, "");
    for path in [
        &regular_path,
        &ai_path,
        &draft_path,
        &notes_path,
        &template_path,
    ] {
        assert!(!path.exists(), "plaintext remained: {}", path.display());
        assert!(
            crate::encryption::encrypted_path(path).exists(),
            "encrypted file missing: {}",
            path.display()
        );
    }
}

#[test]
#[cfg(unix)]
fn encrypt_rotate_reencrypts_existing_storage_and_persists_fingerprint() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut config = config::Config::default();
    config.encryption.enabled = true;
    config.encryption.key_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
    config::save_config(&config_path, &config).unwrap();
    rewrite_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &regular_path,
        &[HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
    )
    .unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        encryption_config: config.encryption,
        ..AppState::default()
    };
    state
        .draft
        .insert_str("#encrypt rotate second@example.invalid");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("reencrypted_files=1"));
    assert_eq!(
        state.encryption_config.key_fingerprint,
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    );
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(
        loaded.encryption.key_fingerprint,
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    );
    let encrypted = fs::read_to_string(crate::encryption::encrypted_path(&regular_path)).unwrap();
    assert!(encrypted.starts_with("recipient:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB\n"));
    assert!(!regular_path.exists());
}

#[test]
#[cfg(unix)]
fn encrypted_completion_uses_cached_templates_without_gpg_on_keypress() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = temp.path().join("fail-gpg");
    fs::write(
        &fake_gpg,
        "#!/bin/sh\nprintf 'unexpected gpg call\\n' >&2\nexit 9\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).unwrap();
    }
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let mut state = AppState {
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        templates: vec![TemplateEntry::new("git add . && git commit")],
        ..AppState::default()
    };
    state.draft.insert_str("git");

    let candidates = state.completion_candidates().unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert_eq!(candidates[0].display, "git add . && git commit");
}

#[test]
#[cfg(unix)]
fn encrypted_history_append_does_not_block_command_completion() {
    let temp = tempfile::tempdir().unwrap();
    let release_path = temp.path().join("release-gpg");
    let fake_gpg = write_blocking_fake_gpg(&temp, &release_path);
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut cache = HashMap::new();
    cache.insert(regular_path.clone(), Vec::new());
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        encrypted_writer: Some(EncryptedWriteQueue::start(
            fake_gpg.display().to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            cache,
        )),
        ..AppState::default()
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let completed_quickly = std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = record_completed_command(
                &mut state,
                "echo async-encrypted-history".to_string(),
                "async-encrypted-history\n".to_string(),
                0,
                false,
            )
            .map_err(|error| error.to_string());
            done_tx.send(result).unwrap();
        });
        match done_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(result) => {
                result.unwrap();
                true
            }
            Err(_) => {
                fs::write(&release_path, b"go\n").unwrap();
                let result = done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("record_completed_command stayed blocked");
                result.unwrap();
                false
            }
        }
    });
    assert!(
        completed_quickly,
        "encrypted append blocked command completion"
    );
    assert_eq!(state.regular_history.len(), 1);
    assert!(
        !crate::encryption::encrypted_path(&regular_path).exists(),
        "background GPG finished before it was released"
    );

    fs::write(&release_path, b"go\n").unwrap();
    state.flush_encrypted_writes().unwrap();
    assert!(state.drain_encrypted_write_events());
    let loaded =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "echo async-encrypted-history");
}

#[test]
fn encrypt_rewrite_history_plan_reports_manual_confirmed_flow() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let mut state = AppState {
        config_path: Some(config_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt rewrite-history plan");
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
    assert!(output.contains("history rewrite plan"));
    assert!(output.contains("risk=rewrites commit ids"));
    assert!(
        output.contains(
            "next=#encrypt rewrite-history run <key-fingerprint> --confirm-rewrite-history"
        )
    );
}

#[test]
fn encrypt_rewrite_history_script_keeps_decrypted_temp_outside_rewrite_tree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let state = AppState {
        regular_history_path: Some(root.join("history/regular.jsonl")),
        ..AppState::default()
    };

    let script_path = write_history_rewrite_script(root, &state).unwrap();
    let script = fs::read_to_string(&script_path).unwrap();

    assert!(script.contains("mktemp -d"));
    assert!(script.contains("tmp=\"$tmp_dir/plain\""));
    assert!(script.contains("trap cleanup EXIT HUP INT TERM"));
    assert!(!script.contains("$enc.plain"));
    assert!(!script.contains(".plain.$$"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&script_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn encrypt_rewrite_history_run_requires_clean_git_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    run_test_git(root, ["init"]);
    fs::write(root.join("dirty.txt"), "uncommitted").unwrap();
    let config_path = root.join("config.toml");
    let mut state = AppState {
        config_path: Some(config_path),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str(
            "#encrypt rewrite-history run BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB --confirm-rewrite-history",
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

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("history rewrite requires a clean git worktree"));
}

#[test]
#[cfg(unix)]
fn encrypted_writes_use_gpg_files_without_plaintext_jsonl() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let regular_path = temp.path().join("history/regular.jsonl");
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        regular_history_path: Some(regular_path.clone()),
        template_store_path: Some(template_path.clone()),
        encryption_config: EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    state.draft.insert_str("echo encrypted-history");
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();
    state.draft.insert_str("#mt echo encrypted-template");
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();

    let loaded_history =
        load_encrypted_jsonl::<HistoryEntry>(fake_gpg.display().to_string(), &regular_path)
            .unwrap();
    let loaded_templates =
        load_encrypted_jsonl::<TemplateEntry>(fake_gpg.display().to_string(), &template_path)
            .unwrap();
    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    assert!(!regular_path.exists());
    assert!(!template_path.exists());
    assert!(crate::encryption::encrypted_path(&regular_path).exists());
    assert!(crate::encryption::encrypted_path(&template_path).exists());
    assert_eq!(loaded_history.items.len(), 1);
    assert_eq!(loaded_history.items[0].command, "echo encrypted-history");
    assert_eq!(loaded_templates.items.len(), 1);
    assert_eq!(loaded_templates.items[0].body, "echo encrypted-template");
}

#[test]
#[cfg(unix)]
fn key_set_encrypts_env_api_key_without_printing_secret() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
        std::env::set_var("AISH_TEST_API_KEY", "secret-test-key");
    }
    let key_path = temp.path().join("secrets/key.json.gpg");
    let events_path = temp.path().join("logs/events.jsonl");
    let mut state = AppState {
        secret_key_path: Some(key_path.clone()),
        events_path: Some(events_path.clone()),
        ai_config: AiConfig {
            env_key: "AISH_TEST_API_KEY".to_string(),
            ..AiConfig::default()
        },
        encryption_config: EncryptionConfig {
            enabled: false,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#key set");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let decrypted = gpg_decrypt_file(fake_gpg.display().to_string(), &key_path).unwrap();
    let record: StoredApiKey = serde_json::from_slice(&decrypted).unwrap();
    unsafe {
        std::env::remove_var("AISH_GPG");
        std::env::remove_var("AISH_TEST_API_KEY");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("stored key encrypted"));
    assert!(!output.contains("secret-test-key"));
    assert_eq!(record.env_key, "AISH_TEST_API_KEY");
    assert_eq!(record.value, "secret-test-key");
    assert!(key_path.exists());
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items[0].msg, "stored key encrypted");
}

#[test]
#[cfg(unix)]
fn ai_prompt_uses_gpg_stored_key_when_env_key_is_missing() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
        std::env::set_var("AISH_TEST_API_KEY", "secret-test-key");
    }
    let key_path = temp.path().join("secrets/key.json.gpg");
    let mut state = AppState {
        secret_key_path: Some(key_path),
        ai_config: AiConfig {
            model: "test-model".to_string(),
            base_url: "https://example.invalid/v1/chat/completions".to_string(),
            env_key: "AISH_TEST_API_KEY".to_string(),
            ..AiConfig::default()
        },
        ai_requester: ai_requester_requires_stored_key,
        encryption_config: EncryptionConfig {
            enabled: false,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            recipient: String::new(),
        },
        ..AppState::default()
    };
    state.draft.insert_str("#key set");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    execute_draft(
        &mut state,
        &mut backend,
        &mut Vec::new(),
        Duration::from_secs(5),
    )
    .unwrap();
    unsafe {
        std::env::remove_var("AISH_TEST_API_KEY");
    }
    state.draft.insert_str("# list files");
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("AI items generated: 1"));
    assert_eq!(state.ai_sessions.len(), 1);
    assert_eq!(state.ai_sessions[0].items[0].text, "pwd");
}

#[test]
#[cfg(unix)]
fn encrypt_off_decrypts_storage_and_persists_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }
    let config_path = temp.path().join("config.toml");
    let regular_path = temp.path().join("history/regular.jsonl");
    let mut config = config::Config::default();
    config.encryption.enabled = true;
    config.encryption.key_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
    config::save_config(&config_path, &config).unwrap();
    rewrite_encrypted_jsonl(
        fake_gpg.display().to_string(),
        "test@example.invalid",
        &regular_path,
        &[HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        }],
    )
    .unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        regular_history_path: Some(regular_path.clone()),
        encryption_config: config.encryption,
        ..AppState::default()
    };
    state.draft.insert_str("#encrypt off");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    unsafe {
        std::env::remove_var("AISH_GPG");
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("encryption=off"));
    assert!(!state.encryption_config.enabled);
    assert!(
        !config::load_config(&config_path)
            .unwrap()
            .encryption
            .enabled
    );
    assert!(regular_path.exists());
    assert!(!crate::encryption::encrypted_path(&regular_path).exists());
    let loaded = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
    assert_eq!(loaded.items[0].command, "pwd");
}

#[test]
fn sync_config_commands_persist_without_running_git() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        (
            "#set-remote git@example.invalid:aish.git",
            "sync.remote=git@example.invalid:aish.git",
        ),
        ("#sync 0 * * * *", "sync.schedule=0 * * * *"),
        ("#sync ai on", "sync.ai=true"),
        ("#sync history on", "sync.history=true"),
        ("#sync templates on", "sync.templates=true"),
        ("#sync drafts on", "sync.drafts=true"),
        ("#sync drafts off", "sync.drafts=false"),
        ("#sync off", "sync.enabled=false"),
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
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert!(
            output.contains("no git command run") || output.contains("no scheduler file created")
        );
    }

    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.sync.remote, "git@example.invalid:aish.git");
    assert!(!loaded.sync.enabled);
    assert!(loaded.sync.schedule.is_empty());
    assert!(loaded.sync.ai);
    assert!(loaded.sync.history);
    assert!(loaded.sync.templates);
    assert!(!loaded.sync.drafts);
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 8);
    assert!(
        events
            .items
            .iter()
            .all(|event| event.msg == "sync config changed")
    );
}

#[test]
fn push_sync_runs_against_configured_local_git_remote() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");

    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(&seed).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&seed, ["config", "commit.gpgsign", "false"]);
    fs::write(seed.join("README.md"), "seed\n").unwrap();
    run_test_git(&seed, ["add", "README.md"]);
    run_test_git(&seed, ["commit", "-m", "seed"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);
    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), root.to_str().unwrap()],
    );
    run_test_git(&root, ["config", "user.name", "Aish Test"]);
    run_test_git(&root, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&root, ["config", "commit.gpgsign", "false"]);

    let config_path = root.join("config.toml");
    let events_path = root.join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = root.clone();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        sync_config: config.sync,
        clock: || 11,
        ..AppState::default()
    };
    state.draft.insert_str("#push");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(10),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("sync step ok: git pull --rebase"),
        "{output}"
    );
    assert!(
        output.contains("sync step ok: git add -- .gitignore"),
        "{output}"
    );
    assert!(output.contains("sync step ok: git commit"), "{output}");
    assert!(output.contains("sync step ok: git push"), "{output}");
    assert!(output.contains("sync push completed"), "{output}");
    assert!(root.join(".gitignore").exists());
    let events = load_events(&events_path).unwrap();
    assert!(
        events
            .items
            .iter()
            .any(|event| event.msg == "sync push completed")
    );
}

#[test]
fn foreground_shell_args_use_login_compatible_command_mode() {
    assert_eq!(
        foreground_shell_args("/bin/bash", "less file"),
        ["-lc", "less file"]
    );
    assert_eq!(
        foreground_shell_args("/bin/zsh", "vim file"),
        ["-lc", "vim file"]
    );
    assert_eq!(
        foreground_shell_args("/usr/bin/fish", "less file"),
        ["-c", "less file"]
    );
}

#[test]
fn startup_sync_runs_due_schedule_against_local_git_remote() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    let config_path = root.join("config.toml");
    let events_path = root.join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = root.clone();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config.sync.enabled = true;
    config.sync.schedule = "@hourly".to_string();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        sync_config: config.sync,
        clock: || 3_600,
        ..AppState::default()
    };
    let mut output = Vec::new();

    run_startup_sync_check(&mut state, &root, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("startup sync due; running #push"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    assert_eq!(
        fs::read_to_string(root.join("cache/runtime/sync.last_attempt")).unwrap(),
        "3600\n"
    );
    let events = load_events(&events_path).unwrap();
    assert!(
        events
            .items
            .iter()
            .any(|event| event.msg == "sync push completed")
    );
}

#[test]
fn startup_sync_skips_not_due_schedule_without_running_git() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let last_attempt = root.join("cache/runtime/sync.last_attempt");
    write_last_sync_attempt(&last_attempt, 3_500).unwrap();
    let mut state = AppState {
        sync_config: SyncConfig {
            remote: "git@example.invalid:aish.git".to_string(),
            enabled: true,
            schedule: "@hourly".to_string(),
            ..SyncConfig::default()
        },
        clock: || 3_600,
        ..AppState::default()
    };
    let mut output = Vec::new();

    run_startup_sync_check(&mut state, root, &mut output).unwrap();

    assert!(String::from_utf8(output).unwrap().is_empty());
    assert_eq!(fs::read_to_string(last_attempt).unwrap(), "3500\n");
}

fn run_test_git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_local_remote(remote: &Path, seed: &Path, root: &Path) {
    run_test_git(
        remote.parent().unwrap(),
        ["init", "--bare", remote.to_str().unwrap()],
    );
    fs::create_dir_all(seed).unwrap();
    run_test_git(seed, ["init"]);
    run_test_git(seed, ["config", "user.name", "Aish Test"]);
    run_test_git(seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(seed, ["config", "commit.gpgsign", "false"]);
    fs::write(seed.join("README.md"), "seed\n").unwrap();
    run_test_git(seed, ["add", "README.md"]);
    run_test_git(seed, ["commit", "-m", "seed"]);
    run_test_git(seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(seed, ["push", "-u", "origin", "HEAD"]);
    run_test_git(
        remote.parent().unwrap(),
        ["clone", remote.to_str().unwrap(), root.to_str().unwrap()],
    );
    run_test_git(root, ["config", "user.name", "Aish Test"]);
    run_test_git(root, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(root, ["config", "commit.gpgsign", "false"]);
}

#[test]
fn sync_category_toggle_rejects_invalid_usage_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#sync ai maybe");
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
            .contains("usage: #sync ai|history|templates|drafts on|off")
    );
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.sync, SyncConfig::default());
}

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
    assert!(output.contains("Aish command not implemented yet: #statsu"));
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
    assert!(output.contains("keybindings=22"));
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
            .contains("usage: #history <count>")
    );
    assert!(state.draft.is_empty());
}

#[test]
fn unix_timestamp_returns_non_negative_seconds() {
    assert!(unix_timestamp() >= 0);
}

fn fixed_clock() -> i64 {
    42
}

#[test]
fn save_draft_if_configured_persists_non_empty_draft() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("draft.jsonl");
    let mut state = AppState {
        draft_history_path: Some(path.clone()),
        clock: fixed_clock,
        ..AppState::default()
    };
    state.draft.insert_str("git status");

    assert!(save_draft_if_configured(&state).unwrap());

    let loaded = crate::history::load_jsonl::<DraftEntry>(&path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].t, 42);
    assert_eq!(loaded.items[0].text, "git status");
}

#[test]
fn save_draft_if_configured_skips_empty_or_disabled_drafts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("draft.jsonl");
    let mut state = AppState {
        draft_history_path: Some(path.clone()),
        draft_persist: false,
        ..AppState::default()
    };
    state.draft.insert_str("git status");

    assert!(!save_draft_if_configured(&state).unwrap());
    assert!(!path.exists());

    let state = AppState {
        draft_history_path: Some(path.clone()),
        ..AppState::default()
    };
    assert!(!save_draft_if_configured(&state).unwrap());
    assert!(!path.exists());
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}
