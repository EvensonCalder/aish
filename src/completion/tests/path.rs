use super::*;

#[test]
fn complete_path_returns_sorted_matching_file_and_directory_candidates() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("alpha.txt"), "").unwrap();
    std::fs::create_dir(temp.path().join("app")).unwrap();
    std::fs::write(temp.path().join("beta.txt"), "").unwrap();

    assert_eq!(
        complete_path("a", temp.path()),
        [
            CompletionCandidate {
                display: "alpha.txt".to_string(),
                replacement: "alpha.txt".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
            CompletionCandidate {
                display: "app/".to_string(),
                replacement: "app/".to_string(),
                is_dir: true,
                source: CompletionSource::Path,
            },
        ]
    );
}

#[test]
fn complete_path_uses_relative_directory_prefix() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

    assert_eq!(
        complete_path("src/m", temp.path()),
        [CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_completes_prefix_intermediate_directory_component() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();

    assert_eq!(
        complete_path("sr/ma", temp.path()),
        [CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_corrects_typo_intermediate_directory_component() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();

    assert_eq!(
        complete_path("srd/ma", temp.path()),
        [CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_non_first_token_intermediate_directory_typos_respect_fuzzy_switch() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();

    let candidates = complete_non_first_token_for_line_with_options(
        "cat srd/ma",
        "cat srd/ma".len(),
        temp.path(),
        &[],
        &[],
        CompletionOptions {
            fuzzy_enabled: false,
            ..CompletionOptions::default()
        },
    );

    assert!(candidates.is_empty());
}

#[test]
fn complete_non_first_token_intermediate_directory_prefixes_ignore_fuzzy_switch() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();

    let candidates = complete_non_first_token_for_line_with_options(
        "cat sr/ma",
        "cat sr/ma".len(),
        temp.path(),
        &[],
        &[],
        CompletionOptions {
            fuzzy_enabled: false,
            ..CompletionOptions::default()
        },
    );

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn path_like_line_completion_ignores_unrelated_structural_history() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "cat unique-target.txt".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "cat src/m",
        "cat src/m".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn path_like_line_completion_keeps_matching_structural_history() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "cat src/lib.rs".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "cat src/",
        "cat src/".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs"]
    );
}

#[test]
fn complete_path_preserves_relative_dot_prefix_for_component_completion() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();

    assert_eq!(
        complete_path("./sr/ma", temp.path()),
        [CompletionCandidate {
            display: "./src/main.rs".to_string(),
            replacement: "./src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[cfg(unix)]
#[test]
fn complete_path_marks_symlinked_directories_with_trailing_slash() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("target-dir")).unwrap();
    std::os::unix::fs::symlink("target-dir", temp.path().join("linked-dir")).unwrap();

    assert_eq!(
        complete_path("linked", temp.path()),
        [CompletionCandidate {
            display: "linked-dir/".to_string(),
            replacement: "linked-dir/".to_string(),
            is_dir: true,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_preserves_opening_quote_in_replacement_only() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("my file.txt"), "").unwrap();

    assert_eq!(
        complete_path("'my", temp.path()),
        [CompletionCandidate {
            display: "my file.txt".to_string(),
            replacement: "'my file.txt'".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_escapes_unquoted_shell_metacharacters() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("my file.txt"), "").unwrap();
    std::fs::write(temp.path().join("hash#file.txt"), "").unwrap();

    assert_eq!(
        complete_path("my", temp.path()),
        [CompletionCandidate {
            display: "my file.txt".to_string(),
            replacement: "my\\ file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
    assert_eq!(
        complete_path("hash", temp.path()),
        [CompletionCandidate {
            display: "hash#file.txt".to_string(),
            replacement: "hash\\#file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_handles_escaped_space_in_typed_directory() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("my dir")).unwrap();
    std::fs::write(temp.path().join("my dir/file.txt"), "").unwrap();

    assert_eq!(
        complete_path("my\\ dir/f", temp.path()),
        [CompletionCandidate {
            display: "my dir/file.txt".to_string(),
            replacement: "my\\ dir/file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_closes_quotes_and_escapes_quote_sensitive_chars() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("it's.txt"), "").unwrap();
    std::fs::write(temp.path().join("price$1.txt"), "").unwrap();

    assert_eq!(
        complete_path("'it", temp.path()),
        [CompletionCandidate {
            display: "it's.txt".to_string(),
            replacement: "'it'\\''s.txt'".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
    assert_eq!(
        complete_path("\"price", temp.path()),
        [CompletionCandidate {
            display: "price$1.txt".to_string(),
            replacement: "\"price\\$1.txt\"".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_does_not_expand_quoted_or_escaped_literal_tilde() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("~")).unwrap();
    std::fs::write(temp.path().join("~/local.txt"), "").unwrap();

    assert_eq!(
        complete_path("'~/lo", temp.path()),
        [CompletionCandidate {
            display: "~/local.txt".to_string(),
            replacement: "'~/local.txt'".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
    assert_eq!(
        complete_path("\\~/lo", temp.path()),
        [CompletionCandidate {
            display: "~/local.txt".to_string(),
            replacement: "\\~/local.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_path_orders_hidden_entries_after_visible_entries() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".alpha"), "").unwrap();
    std::fs::write(temp.path().join("beta"), "").unwrap();

    assert_eq!(
        complete_path("", temp.path()),
        [
            CompletionCandidate {
                display: "beta".to_string(),
                replacement: "beta".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
            CompletionCandidate {
                display: ".alpha".to_string(),
                replacement: ".alpha".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
        ]
    );
}
