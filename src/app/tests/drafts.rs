use super::*;

#[test]
fn unix_timestamp_returns_non_negative_seconds() {
    assert!(unix_timestamp() >= 0);
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

    assert!(save_draft_if_configured(&mut state).unwrap());

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

    assert!(!save_draft_if_configured(&mut state).unwrap());
    assert!(!path.exists());

    let mut state = AppState {
        draft_history_path: Some(path.clone()),
        ..AppState::default()
    };
    assert!(!save_draft_if_configured(&mut state).unwrap());
    assert!(!path.exists());
}
