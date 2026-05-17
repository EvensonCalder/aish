use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TestEntry {
    command: String,
    exit_code: Option<i32>,
}

#[test]
fn append_and_load_jsonl_items() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history/regular.jsonl");

    append_jsonl(
        &path,
        &TestEntry {
            command: "pwd".to_string(),
            exit_code: Some(0),
        },
    )
    .unwrap();
    append_jsonl(
        &path,
        &TestEntry {
            command: "false".to_string(),
            exit_code: Some(1),
        },
    )
    .unwrap();

    let loaded = load_jsonl::<TestEntry>(&path).unwrap();

    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].command, "pwd");
    assert_eq!(loaded.items[1].exit_code, Some(1));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }
}

#[test]
fn missing_jsonl_file_loads_as_empty() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = load_jsonl::<TestEntry>(&temp.path().join("missing.jsonl")).unwrap();

    assert!(loaded.items.is_empty());
    assert!(loaded.errors.is_empty());
}

#[test]
fn bad_jsonl_lines_are_reported_and_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("regular.jsonl");
    fs::write(
            &path,
            "{\"command\":\"pwd\",\"exit_code\":0}\nnot-json\n\n{\"command\":\"false\",\"exit_code\":1}\n",
        )
        .unwrap();

    let loaded = load_jsonl::<TestEntry>(&path).unwrap();

    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.errors.len(), 1);
    assert_eq!(loaded.errors[0].line, 2);
    assert_eq!(loaded.errors[0].path, path);
    assert!(loaded.errors[0].message.contains("expected"));
}

#[test]
fn rewrite_jsonl_replaces_existing_contents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("regular.jsonl");
    append_jsonl(
        &path,
        &TestEntry {
            command: "old".to_string(),
            exit_code: Some(0),
        },
    )
    .unwrap();

    rewrite_jsonl(
        &path,
        &[TestEntry {
            command: "new".to_string(),
            exit_code: Some(1),
        }],
    )
    .unwrap();

    let loaded = load_jsonl::<TestEntry>(&path).unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].command, "new");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
    }
}

#[test]
fn trim_regular_history_keeps_newest_entries_and_skips_bad_lines() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("regular.jsonl");
    fs::write(
        &path,
        [
            "{\"t\":1,\"command\":\"one\",\"exit_code\":0,\"source\":\"user\"}",
            "bad-json",
            "{\"t\":2,\"command\":\"two\",\"exit_code\":0,\"source\":\"user\"}",
            "{\"t\":3,\"command\":\"three\",\"exit_code\":1,\"source\":\"user\"}",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let before_trim = trim_regular_history(&path, 2).unwrap();
    let after_trim = load_jsonl::<HistoryEntry>(&path).unwrap();

    assert_eq!(before_trim.items.len(), 3);
    assert_eq!(before_trim.errors.len(), 1);
    assert_eq!(after_trim.errors, []);
    assert_eq!(after_trim.items.len(), 2);
    assert_eq!(after_trim.items[0].command, "two");
    assert_eq!(after_trim.items[1].command, "three");
}

#[test]
fn trim_combined_history_limits_regular_plus_ai_command_items() {
    let temp = tempfile::tempdir().unwrap();
    let regular_path = temp.path().join("regular.jsonl");
    let ai_path = temp.path().join("ai.jsonl");

    for (t, command) in [(1, "one"), (2, "two"), (3, "three")] {
        append_jsonl(
            &regular_path,
            &HistoryEntry {
                t,
                command: command.to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        )
        .unwrap();
    }

    append_jsonl(
        &ai_path,
        &AiSession {
            id: "a_1".to_string(),
            t: 4,
            prompt: "older".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "ai one".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Template,
                    text: "template one".to_string(),
                    name: Some("t1".to_string()),
                },
            ],
        },
    )
    .unwrap();
    append_jsonl(
        &ai_path,
        &AiSession {
            id: "a_2".to_string(),
            t: 5,
            prompt: "newer".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "ai two".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Command,
                    text: "ai three".to_string(),
                    name: None,
                },
            ],
        },
    )
    .unwrap();

    let before_trim = trim_combined_history(&regular_path, &ai_path, 2).unwrap();
    let after_regular = load_jsonl::<HistoryEntry>(&regular_path).unwrap();
    let after_ai = load_jsonl::<AiSession>(&ai_path).unwrap();

    assert_eq!(before_trim.regular.items.len(), 3);
    assert_eq!(before_trim.ai_sessions.items.len(), 2);
    assert_eq!(after_regular.items.len(), 2);
    assert_eq!(after_regular.items[0].command, "two");
    assert_eq!(after_regular.items[1].command, "three");
    assert!(after_ai.items.is_empty());
}

#[test]
fn history_entry_serializes_source_as_snake_case() {
    let entry = HistoryEntry {
        t: 123,
        command: "pwd".to_string(),
        exit_code: Some(0),
        source: HistorySource::User,
    };

    let raw = serde_json::to_string(&entry).unwrap();

    assert!(raw.contains("\"source\":\"user\""));
    assert!(raw.contains("\"t\":123"));
}

#[test]
fn note_entry_serializes_tag_as_snake_case() {
    let entry = NoteEntry {
        tag: NoteTag::Fixme,
        text: "clean this up".to_string(),
    };

    let raw = serde_json::to_string(&entry).unwrap();

    assert!(raw.contains("\"tag\":\"fixme\""));
}

#[test]
fn draft_entry_roundtrips_through_json() {
    let entry = DraftEntry {
        t: 123,
        text: "git status".to_string(),
    };

    let raw = serde_json::to_string(&entry).unwrap();
    let parsed: DraftEntry = serde_json::from_str(&raw).unwrap();

    assert_eq!(parsed, entry);
}

#[test]
fn ai_session_roundtrips_through_jsonl() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history/ai.jsonl");
    let session = AiSession {
        id: "a_123".to_string(),
        t: 123,
        prompt: "set git user".to_string(),
        ctx: false,
        model: "test-model".to_string(),
        items: vec![
            AiItem {
                kind: AiItemKind::Command,
                text: "git config --global user.name \"{name}\"".to_string(),
                name: None,
            },
            AiItem {
                kind: AiItemKind::Template,
                text: "git config --global user.email \"{email}\"".to_string(),
                name: Some("git-email".to_string()),
            },
        ],
    };

    append_jsonl(&path, &session).unwrap();
    let loaded = load_jsonl::<AiSession>(&path).unwrap();

    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items, [session]);
    assert_eq!(loaded.items[0].items[0].kind, AiItemKind::Command);
    assert_eq!(loaded.items[0].items[1].kind, AiItemKind::Template);
}

#[test]
fn ai_item_kind_serializes_as_snake_case() {
    let item = AiItem {
        kind: AiItemKind::Command,
        text: "pwd".to_string(),
        name: None,
    };

    let raw = serde_json::to_string(&item).unwrap();

    assert!(raw.contains("\"kind\":\"command\""));
    assert!(!raw.contains("name"));
}

#[test]
fn history_store_loads_all_history_categories() {
    let temp = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(temp.path().join("aish-home"));
    layout.create_dirs().unwrap();

    append_jsonl(
        &layout.regular_history,
        &HistoryEntry {
            t: 1,
            command: "pwd".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        },
    )
    .unwrap();
    append_jsonl(
        &layout.draft_history,
        &DraftEntry {
            t: 2,
            text: "git status".to_string(),
        },
    )
    .unwrap();
    append_jsonl(
        &layout.ai_history,
        &AiSession {
            id: "a_1".to_string(),
            t: 3,
            prompt: "list files".to_string(),
            ctx: false,
            model: "test-model".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "ls".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();
    append_jsonl(
        &layout.notes,
        &NoteEntry {
            tag: NoteTag::Todo,
            text: "ship it".to_string(),
        },
    )
    .unwrap();

    let store = HistoryStore::load(&layout).unwrap();

    assert_eq!(store.errors, []);
    assert_eq!(store.regular.len(), 1);
    assert_eq!(store.regular_newest_indices, [0]);
    assert_eq!(store.drafts.len(), 1);
    assert_eq!(store.ai_sessions.len(), 1);
    assert_eq!(store.ai_command_indices.len(), 1);
    assert_eq!(store.notes.len(), 1);
    assert_eq!(store.regular[0].command, "pwd");
    assert_eq!(store.drafts[0].text, "git status");
    assert_eq!(store.ai_sessions[0].items[0].text, "ls");
    assert_eq!(store.notes[0].text, "ship it");
}

#[test]
fn history_store_indexes_regular_history_newest_first() {
    let temp = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(temp.path().join("aish-home"));
    layout.create_dirs().unwrap();

    for (t, command) in [(1, "one"), (2, "two"), (3, "three")] {
        append_jsonl(
            &layout.regular_history,
            &HistoryEntry {
                t,
                command: command.to_string(),
                exit_code: Some(0),
                source: HistorySource::User,
            },
        )
        .unwrap();
    }

    let store = HistoryStore::load(&layout).unwrap();
    let commands: Vec<_> = store
        .regular_newest()
        .map(|entry| entry.command.as_str())
        .collect();

    assert_eq!(store.regular_newest_indices, [2, 1, 0]);
    assert_eq!(commands, ["three", "two", "one"]);
    assert_eq!(store.regular_by_newest_index(1).unwrap().command, "two");
    assert!(store.regular_by_newest_index(3).is_none());
}

#[test]
fn history_store_indexes_ai_command_items_in_execution_order() {
    let temp = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(temp.path().join("aish-home"));
    layout.create_dirs().unwrap();

    append_jsonl(
        &layout.ai_history,
        &AiSession {
            id: "a_1".to_string(),
            t: 1,
            prompt: "setup".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![
                AiItem {
                    kind: AiItemKind::Command,
                    text: "one".to_string(),
                    name: None,
                },
                AiItem {
                    kind: AiItemKind::Template,
                    text: "skip-template".to_string(),
                    name: Some("template".to_string()),
                },
                AiItem {
                    kind: AiItemKind::Command,
                    text: "two".to_string(),
                    name: None,
                },
            ],
        },
    )
    .unwrap();
    append_jsonl(
        &layout.ai_history,
        &AiSession {
            id: "a_2".to_string(),
            t: 2,
            prompt: "next".to_string(),
            ctx: false,
            model: "test".to_string(),
            items: vec![AiItem {
                kind: AiItemKind::Command,
                text: "three".to_string(),
                name: None,
            }],
        },
    )
    .unwrap();

    let store = HistoryStore::load(&layout).unwrap();
    let commands: Vec<_> = store
        .ai_commands()
        .map(|(_, item)| item.text.as_str())
        .collect();

    assert_eq!(
        store.ai_command_indices,
        [
            AiCommandIndex {
                session_index: 0,
                item_index: 0
            },
            AiCommandIndex {
                session_index: 0,
                item_index: 2
            },
            AiCommandIndex {
                session_index: 1,
                item_index: 0
            },
        ]
    );
    assert_eq!(commands, ["one", "two", "three"]);
    assert_eq!(store.ai_command_by_index(1).unwrap().1.text, "two");
    assert!(store.ai_command_by_index(3).is_none());
}

#[test]
fn split_logical_commands_splits_simple_non_empty_lines() {
    let commands = split_logical_commands("\ncd /tmp\n\npwd\n");

    assert_eq!(commands, ["cd /tmp", "pwd"]);
}

#[test]
fn split_logical_commands_preserves_backslash_continuations() {
    let commands = split_logical_commands("echo foo \\\n+bar\npwd");

    assert_eq!(commands, ["echo foo \\\n+bar", "pwd"]);
}

#[test]
fn split_logical_commands_skips_standalone_comments() {
    let commands = split_logical_commands("# comment\npwd\n  # another\necho done");

    assert_eq!(commands, ["pwd", "echo done"]);
}

#[test]
fn split_logical_commands_can_extract_comment_only_notes() {
    let input =
        "# TODO: ship it\npwd\n  # NOTE: check logs\necho '# TODO: not a note'\n# plain comment\n";

    let (commands, notes) = split_logical_commands_and_comment_notes(input);

    assert_eq!(commands, ["pwd", "echo '# TODO: not a note'"]);
    assert_eq!(
        notes,
        [
            (NoteTag::Todo, "ship it".to_string()),
            (NoteTag::Note, "check logs".to_string())
        ]
    );
}

#[test]
fn split_logical_commands_preserves_inline_hash_content() {
    let commands = split_logical_commands("echo '# not a comment'\necho value # inline");

    assert_eq!(commands, ["echo '# not a comment'", "echo value # inline"]);
}

#[test]
fn split_logical_commands_preserves_single_quoted_newlines() {
    let commands = split_logical_commands("printf 'one\ntwo'\npwd");

    assert_eq!(commands, ["printf 'one\ntwo'", "pwd"]);
}

#[test]
fn split_logical_commands_preserves_double_quoted_newlines() {
    let commands = split_logical_commands("printf \"one\ntwo\"\npwd");

    assert_eq!(commands, ["printf \"one\ntwo\"", "pwd"]);
}

#[test]
fn split_logical_commands_ignores_escaped_quotes() {
    let commands = split_logical_commands("echo \"one \\\"two\\\"\"\npwd");

    assert_eq!(commands, ["echo \"one \\\"two\\\"\"", "pwd"]);
}

#[test]
fn split_logical_commands_preserves_heredoc_blocks() {
    let input = "cat <<EOF\none\ntwo\nEOF\npwd";
    let commands = split_logical_commands(input);

    assert_eq!(commands, ["cat <<EOF\none\ntwo\nEOF", "pwd"]);
}

#[test]
fn split_logical_commands_preserves_quoted_heredoc_delimiter() {
    let input = "cat <<'EOF'\n$literal\nEOF\npwd";
    let commands = split_logical_commands(input);

    assert_eq!(commands, ["cat <<'EOF'\n$literal\nEOF", "pwd"]);
}

#[test]
fn history_store_aggregates_load_errors_across_categories() {
    let temp = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(temp.path().join("aish-home"));
    layout.create_dirs().unwrap();
    fs::write(&layout.regular_history, "bad-regular\n").unwrap();
    fs::write(&layout.ai_history, "bad-ai\n").unwrap();

    let store = HistoryStore::load(&layout).unwrap();

    assert!(store.regular.is_empty());
    assert!(store.ai_sessions.is_empty());
    assert_eq!(store.errors.len(), 2);
    assert!(
        store
            .errors
            .iter()
            .any(|error| error.path == layout.regular_history)
    );
    assert!(
        store
            .errors
            .iter()
            .any(|error| error.path == layout.ai_history)
    );
}
