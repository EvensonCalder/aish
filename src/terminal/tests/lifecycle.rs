use super::*;

#[test]
fn terminal_event_source_maps_crossterm_events() {
    assert_eq!(
        terminal_event_from_crossterm(Event::Key(key(KeyCode::Char('x')))),
        TerminalEvent::Key(key(KeyCode::Char('x')))
    );
    assert_eq!(
        terminal_event_from_crossterm(Event::Paste("echo ok".to_string())),
        TerminalEvent::Paste("echo ok".to_string())
    );
    assert_eq!(
        terminal_event_from_crossterm(Event::Resize(100, 30)),
        TerminalEvent::Resize(100, 30)
    );
    assert_eq!(
        terminal_event_from_crossterm(Event::FocusGained),
        TerminalEvent::Ignore
    );
}

#[test]
#[cfg(unix)]
fn encrypted_write_completion_event_refreshes_live_completion() {
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_copying_fake_gpg(&temp);
    let history_path = temp.path().join("history/regular.jsonl");
    let mut cache = HashMap::new();
    cache.insert(history_path.clone(), Vec::new());
    let entry = HistoryEntry {
        t: 1,
        command: "git status".to_string(),
        exit_code: Some(0),
        source: HistorySource::User,
    };
    let mut state = AppState {
        regular_history_path: Some(history_path.clone()),
        regular_history: vec![entry.clone()],
        encryption_config: crate::config::EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: crate::config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_writer: Some(EncryptedWriteQueue::start(
            fake_gpg.display().to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            cache,
        )),
        ..AppState::default()
    };
    state.draft.insert_str("git");
    state
        .encrypted_writer
        .as_ref()
        .unwrap()
        .enqueue_append_jsonl(&history_path, &entry)
        .unwrap();
    state.flush_encrypted_writes().unwrap();
    assert!(state.completion_inline.is_none());

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
    assert_eq!(state.completion_inline.as_ref().unwrap().suffix, " status");
}

#[test]
#[cfg(unix)]
fn exit_persistence_boundary_flushes_pending_encrypted_draft() {
    let temp = tempfile::tempdir().unwrap();
    let started_path = temp.path().join("gpg-started");
    let release_path = temp.path().join("release-gpg");
    let fake_gpg = write_blocking_fake_gpg(&temp, &started_path, &release_path);
    let draft_path = temp.path().join("history/draft.jsonl");
    let mut cache = HashMap::new();
    cache.insert(draft_path.clone(), Vec::new());
    let mut state = AppState {
        draft_history_path: Some(draft_path.clone()),
        draft_persist: true,
        encryption_config: crate::config::EncryptionConfig {
            enabled: true,
            key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            startup_unlock: crate::config::EncryptionStartupUnlockMode::Lazy,
            recipient: String::new(),
        },
        encrypted_writer: Some(EncryptedWriteQueue::start(
            fake_gpg.display().to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            cache,
        )),
        ..AppState::default()
    };
    state.draft.insert_str("echo pending-draft");
    let release_for_thread = release_path.clone();
    let started_for_thread = started_path.clone();
    let releaser = std::thread::spawn(move || {
        for _ in 0..200 {
            if started_for_thread.exists() {
                std::thread::sleep(Duration::from_millis(120));
                std::fs::write(&release_for_thread, b"go\n").unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("fake gpg did not start");
    });

    let started = Instant::now();
    persist_draft_and_flush_before_exit(&mut state, &mut Vec::new()).unwrap();
    let elapsed = started.elapsed();
    releaser.join().unwrap();

    assert!(
        elapsed >= Duration::from_millis(100),
        "exit returned before pending encrypted write was released"
    );
    let loaded = crate::encryption::load_encrypted_jsonl::<DraftEntry>(
        fake_gpg.display().to_string(),
        &draft_path,
    )
    .unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].text, "echo pending-draft");
}

#[test]
fn exit_persistence_boundary_runs_enabled_exit_sync() {
    let mut state = AppState {
        sync_config: crate::config::SyncConfig {
            exit: true,
            ..crate::config::SyncConfig::default()
        },
        ..AppState::default()
    };
    let mut output = Vec::new();

    persist_draft_and_flush_before_exit(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        output,
        "exit sync enabled; running #sync now\r\nsync remote is not configured; run #set-remote <git-url> first\r\n"
    );
}

#[test]
fn crlf_writer_normalizes_lf_without_double_converting_crlf() {
    let mut output = Vec::new();
    {
        let mut writer = CrLfWriter::new(&mut output);
        write!(writer, "one\ntwo\r\nthree\r").unwrap();
        write!(writer, "\nfour").unwrap();
    }

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "one\r\ntwo\r\nthree\r\nfour"
    );
}

#[test]
fn panic_cleanup_hook_can_be_installed_without_panicking() {
    install_panic_cleanup();
}
