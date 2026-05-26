use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use aish::app::{AppState, execute_draft};
use aish::config::AiConfig;
use aish::editor::prepare_editor_file;
use aish::history::{
    AiCommandIndex, AiItem, AiItemKind, AiSession, HistoryEntry, HistorySource, append_jsonl,
    load_jsonl,
};
use aish::modes::Mode;
use aish::pty::PtyBackend;

#[path = "draft_execution/ai.rs"]
mod ai;
#[path = "draft_execution/basic.rs"]
mod basic;
#[path = "draft_execution/editor.rs"]
mod editor;
#[path = "draft_execution/history.rs"]
mod history;

fn fixed_clock() -> i64 {
    1234567890
}

static PTY_EXECUTION_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn pty_execution_guard() -> MutexGuard<'static, ()> {
    PTY_EXECUTION_TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fake_echo_message_ai_items(_: &AiConfig, prompt: &str) -> anyhow::Result<Vec<AiItem>> {
    assert_eq!(prompt, "how to echo something?");
    Ok(vec![AiItem {
        kind: AiItemKind::Command,
        text: "echo {message}".to_string(),
        name: None,
    }])
}

fn ai_state(
    commands: &[&str],
    selected_ai_index: usize,
    history_path: std::path::PathBuf,
) -> AppState {
    AppState {
        mode: Mode::Ai,
        regular_history_path: Some(history_path),
        ai_sessions: vec![AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "commands".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: commands
                .iter()
                .map(|command| AiItem {
                    kind: AiItemKind::Command,
                    text: (*command).to_string(),
                    name: None,
                })
                .collect(),
        }],
        ai_command_indices: commands
            .iter()
            .enumerate()
            .map(|(item_index, _)| AiCommandIndex {
                session_index: 0,
                item_index,
            })
            .collect(),
        selected_ai_index: Some(selected_ai_index),
        clock: fixed_clock,
        ..AppState::default()
    }
}
