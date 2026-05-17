use super::*;
use crate::config::CompletionTabAccept;
use crate::display_width::display_width;

#[test]
fn current_token_detects_first_token_prefix() {
    assert_eq!(
        current_token_context("git sta", 3),
        TokenContext {
            start: 0,
            end: 3,
            text: "git".to_string(),
            is_first_token: true,
            quote: None,
            path_like: false,
        }
    );
}

#[test]
fn current_token_detects_non_first_token_at_cursor() {
    assert_eq!(
        current_token_context("git sta", 7),
        TokenContext {
            start: 4,
            end: 7,
            text: "sta".to_string(),
            is_first_token: false,
            quote: None,
            path_like: false,
        }
    );
}

#[test]
fn current_token_keeps_quoted_whitespace_inside_token() {
    assert_eq!(
        current_token_context("echo \"hello wo", 14),
        TokenContext {
            start: 5,
            end: 14,
            text: "\"hello wo".to_string(),
            is_first_token: false,
            quote: Some('"'),
            path_like: false,
        }
    );
}

#[test]
fn current_token_keeps_escaped_whitespace_inside_token() {
    assert_eq!(
        current_token_context("cd my\\ dir/fi", 13),
        TokenContext {
            start: 3,
            end: 13,
            text: "my\\ dir/fi".to_string(),
            is_first_token: false,
            quote: None,
            path_like: true,
        }
    );
}

#[test]
fn current_token_handles_cursor_inside_line() {
    assert_eq!(
        current_token_context("git checkout main", 12),
        TokenContext {
            start: 4,
            end: 12,
            text: "checkout".to_string(),
            is_first_token: false,
            quote: None,
            path_like: false,
        }
    );
}

#[test]
fn path_like_detection_covers_common_shell_path_prefixes() {
    for token in ["/tmp", "./src", "../src", "~/src", "src/main.rs", "'./src"] {
        assert!(is_path_like_token(token), "{token:?} should be path-like");
    }
    for token in ["git", "status", "--flag"] {
        assert!(
            !is_path_like_token(token),
            "{token:?} should not be path-like"
        );
    }
}

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
fn complete_path_preserves_opening_quote_in_replacement_only() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("my file.txt"), "").unwrap();

    assert_eq!(
        complete_path("'my", temp.path()),
        [CompletionCandidate {
            display: "my file.txt".to_string(),
            replacement: "'my file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        }]
    );
}

#[test]
fn complete_first_token_orders_templates_history_then_executables() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("git-now");
    std::fs::write(&executable, "#!/bin/sh\n").unwrap();
    make_executable(&executable);
    let templates = vec![TemplateEntry::new("git add . && git commit")];
    let history = vec![HistoryEntry {
        t: 2,
        command: "git status".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    assert_eq!(
        complete_first_token("git", &templates, &history, &[bin]),
        [
            CompletionCandidate {
                display: "git add . && git commit".to_string(),
                replacement: "git add . && git commit".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
            CompletionCandidate {
                display: "git status".to_string(),
                replacement: "git status".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
            CompletionCandidate {
                display: "git-now".to_string(),
                replacement: "git-now".to_string(),
                is_dir: false,
                source: CompletionSource::Executable,
            },
        ]
    );
}

#[test]
fn complete_first_token_deduplicates_each_source() {
    let templates = vec![
        TemplateEntry::new("docker deploy"),
        TemplateEntry::new("docker deploy"),
    ];
    let history = vec![
        HistoryEntry {
            t: 2,
            command: "docker ps".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
        HistoryEntry {
            t: 1,
            command: "docker ps".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
    ];

    assert_eq!(
        complete_first_token("d", &templates, &history, &[]),
        [
            CompletionCandidate {
                display: "docker deploy".to_string(),
                replacement: "docker deploy".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
            CompletionCandidate {
                display: "docker ps".to_string(),
                replacement: "docker ps".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
        ]
    );
}

#[test]
fn complete_first_token_can_match_while_ignoring_spaces_and_limit_results() {
    let templates = vec![TemplateEntry::new("git add . && git commit")];
    let history = vec![
        HistoryEntry {
            t: 2,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
        HistoryEntry {
            t: 1,
            command: "git stash".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
    ];

    assert_eq!(
        complete_first_token_with_options(
            "g s",
            &templates,
            &history,
            &[],
            CompletionOptions {
                max_results: 2,
                ignore_spaces: true,
                fuzzy_enabled: true,
                match_threshold_percent: 50,
                typo_threshold_percent: 80,
            },
        ),
        [
            CompletionCandidate {
                display: "git status".to_string(),
                replacement: "git status".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
            CompletionCandidate {
                display: "git stash".to_string(),
                replacement: "git stash".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
        ]
    );
}

#[test]
fn complete_non_first_token_orders_history_arguments_before_path_candidates() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/main.rs"), "").unwrap();
    let history = vec![HistoryEntry {
        t: 2,
        command: "git add src/lib.rs".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    assert_eq!(
        complete_non_first_token("src/", temp.path(), &history, &[]),
        [
            CompletionCandidate {
                display: "src/lib.rs".to_string(),
                replacement: "src/lib.rs".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
            CompletionCandidate {
                display: "src/main.rs".to_string(),
                replacement: "src/main.rs".to_string(),
                is_dir: false,
                source: CompletionSource::Path,
            },
        ]
    );
}

#[test]
fn complete_non_first_token_includes_plain_path_prefixes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("one.txt"), "").unwrap();

    let candidates = complete_non_first_token_with_options(
        "o",
        temp.path(),
        &[],
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].display, "one.txt");
    assert_eq!(candidates[0].source, CompletionSource::Path);
}

#[test]
fn complete_non_first_token_promotes_matching_directories_over_history_arguments() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src-file.txt"), "").unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "cat src-file-from-history".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "cat sr",
        "cat sr".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "src/".to_string(),
            replacement: "src/".to_string(),
            is_dir: true,
            source: CompletionSource::Path,
        })
    );
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.display.as_str())
            .collect::<Vec<_>>(),
        ["src/", "src-file-from-history"]
    );
}

#[test]
fn complete_non_first_token_corrects_directory_typos() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("srd-file.txt"), "").unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "cat srd-from-history".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "cat srd",
        "cat srd".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "src/".to_string(),
            replacement: "src/".to_string(),
            is_dir: true,
            source: CompletionSource::Path,
        })
    );
}

#[test]
fn complete_non_first_token_directory_typos_respect_fuzzy_switch() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();

    let candidates = complete_non_first_token_for_line_with_options(
        "cat srd",
        "cat srd".len(),
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
fn complete_non_first_token_includes_history_arguments_without_path_prefix() {
    let history = vec![
        HistoryEntry {
            t: 2,
            command: "kubectl get pods".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
        HistoryEntry {
            t: 1,
            command: "docker get pods".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
    ];

    let templates = vec![TemplateEntry::new("kubectl logs {pod_name}")];

    assert_eq!(
        complete_non_first_token("po", Path::new("/"), &history, &templates),
        [
            CompletionCandidate {
                display: "{pod_name}".to_string(),
                replacement: "{pod_name}".to_string(),
                is_dir: false,
                source: CompletionSource::TemplatePlaceholder,
            },
            CompletionCandidate {
                display: "pods".to_string(),
                replacement: "pods".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            }
        ]
    );
}

#[test]
fn complete_non_first_token_applies_options_to_history_and_placeholders() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git commit featurebranch".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let templates = vec![TemplateEntry::new("git checkout {featurebranch}")];

    assert_eq!(
        complete_non_first_token_with_options(
            "feature b",
            Path::new("/"),
            &history,
            &templates,
            CompletionOptions {
                max_results: 1,
                ignore_spaces: true,
                fuzzy_enabled: true,
                match_threshold_percent: 50,
                typo_threshold_percent: 80,
            },
        ),
        [CompletionCandidate {
            display: "{featurebranch}".to_string(),
            replacement: "{featurebranch}".to_string(),
            is_dir: false,
            source: CompletionSource::TemplatePlaceholder,
        }]
    );
}

#[test]
fn indexed_primary_completion_matches_unindexed_results() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("feature-file"), "").unwrap();
    let history = vec![
        HistoryEntry {
            t: 2,
            command: "git checkout featurebranch".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
        HistoryEntry {
            t: 1,
            command: "git commit feature-file".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
    ];
    let templates = vec![TemplateEntry::new("git checkout {featurebranch}")];
    let indexed_history = index_history_entries(&history);
    let indexed_templates = index_template_entries(&templates);

    let options = CompletionOptions::default();
    let unindexed = complete_non_first_token_for_line_with_options(
        "git checkout feature",
        "git checkout feature".len(),
        temp.path(),
        &history,
        &templates,
        options,
    );
    let indexed = complete_non_first_token_for_line_with_indexed_options(
        "git checkout feature",
        "git checkout feature".len(),
        temp.path(),
        &indexed_history,
        &indexed_templates,
        options,
    );

    assert_eq!(indexed, unindexed);
}

#[test]
fn indexed_typo_completion_recomputes_non_monotonic_similarity() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git status".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let indexed_history = index_history_entries(&history);
    let indexed_templates = index_template_entries(&[]);

    let early = complete_non_first_token_typos_for_line_with_indexed_options(
        "git stx",
        "git stx".len(),
        &indexed_history,
        &indexed_templates,
        CompletionOptions::default(),
    );
    assert!(early.is_empty());

    let later_unindexed = complete_non_first_token_typos_for_line_with_options(
        "git statuz",
        "git statuz".len(),
        &history,
        &[],
        CompletionOptions::default(),
    );
    let later_indexed = complete_non_first_token_typos_for_line_with_indexed_options(
        "git statuz",
        "git statuz".len(),
        &indexed_history,
        &indexed_templates,
        CompletionOptions::default(),
    );

    assert_eq!(later_indexed, later_unindexed);
    assert_eq!(
        later_indexed.first(),
        Some(&CompletionCandidate {
            display: "git status".to_string(),
            replacement: "git status".to_string(),
            is_dir: false,
            source: CompletionSource::HistoryTypo,
        })
    );
}

#[test]
fn strict_structural_threshold_filters_current_position_mismatch() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "command add 100 file".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "command add 200",
        "command add 200".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions {
            match_threshold_percent: 80,
            ..CompletionOptions::default()
        },
    );

    assert!(candidates.is_empty());
}

#[test]
fn completion_match_threshold_filters_weak_partial_matches() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git status --short".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let default_threshold_candidates = complete_non_first_token_for_line_with_options(
        "git stx",
        "git stx".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions::default(),
    );
    assert_eq!(
        default_threshold_candidates.first(),
        Some(&CompletionCandidate {
            display: "status --short".to_string(),
            replacement: "status --short".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );

    let strict_threshold_candidates = complete_non_first_token_for_line_with_options(
        "git stx",
        "git stx".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions {
            match_threshold_percent: 80,
            ..CompletionOptions::default()
        },
    );
    assert!(strict_threshold_candidates.is_empty());
}

#[test]
fn trailing_space_completes_structural_history_without_path_noise() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("local-file"), "").unwrap();
    let history = vec![
        HistoryEntry {
            t: 2,
            command: "git commit -m message".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
        HistoryEntry {
            t: 1,
            command: "git status --short".to_string(),
            exit_code: Some(0),
            source: crate::history::HistorySource::User,
        },
    ];

    let candidates = complete_non_first_token_for_line_with_options(
        "git ",
        "git ".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [
            CompletionCandidate {
                display: "commit -m message".to_string(),
                replacement: "commit -m message".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
            CompletionCandidate {
                display: "status --short".to_string(),
                replacement: "status --short".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            },
        ]
    );
}

#[test]
fn trailing_space_uses_previous_word_match_threshold() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git status --short".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "gi ",
        "gi ".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "status --short".to_string(),
            replacement: "status --short".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        }]
    );

    let strict_candidates = complete_non_first_token_for_line_with_options(
        "gix ",
        "gix ".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions::default(),
    );
    assert!(strict_candidates.is_empty());
}

#[test]
fn trailing_space_requires_structural_prefix_match() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git status --short".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "gix ",
        "gix ".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert!(candidates.is_empty());
}

#[test]
fn trailing_space_prefers_structural_templates() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "kubectl get pods".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let templates = vec![TemplateEntry::new("kubectl get {resource}")];

    let candidates = complete_non_first_token_for_line_with_options(
        "kubectl get ",
        "kubectl get ".len(),
        Path::new("/"),
        &history,
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [
            CompletionCandidate {
                display: "kubectl get {resource}".to_string(),
                replacement: "{resource}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
            CompletionCandidate {
                display: "pods".to_string(),
                replacement: "pods".to_string(),
                is_dir: false,
                source: CompletionSource::History,
            }
        ]
    );
}

#[test]
fn trailing_space_templates_use_previous_word_match_threshold() {
    let templates = vec![TemplateEntry::new("kubectl get {resource}")];

    let candidates = complete_non_first_token_for_line_with_options(
        "kubectl g ",
        "kubectl g ".len(),
        Path::new("/"),
        &[],
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "kubectl get {resource}".to_string(),
            replacement: "{resource}".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        }]
    );
}

#[test]
fn private_command_completion_uses_aish_commands_only() {
    let candidates = complete_private_commands("#sta", usize::MAX);

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "#status".to_string(),
            replacement: "#status".to_string(),
            is_dir: false,
            source: CompletionSource::PrivateCommand,
        }]
    );
    assert!(complete_private_commands("#", usize::MAX).is_empty());
    assert!(complete_private_commands("# ", usize::MAX).is_empty());
}

#[test]
fn private_command_completion_includes_nested_arguments() {
    let candidates =
        complete_private_command_line("#completion ", "#completion ".len(), usize::MAX);

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "mode")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "tab-accept")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "display-delay-ms")
    );

    let prompt_candidates = complete_private_command_line("#prompt ", "#prompt ".len(), usize::MAX);
    assert_eq!(
        prompt_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["draft", "history", "ai", "reset"]
    );

    let mode_candidates =
        complete_private_command_line("#completion mode ", "#completion mode ".len(), usize::MAX);

    assert_eq!(
        mode_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["auto", "tab", "off"]
    );

    let partial_arg_candidates =
        complete_private_command_line("#completion m", "#completion m".len(), usize::MAX);

    assert_eq!(
        partial_arg_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["mode", "max", "match-threshold"]
    );

    let partial_nested_candidates =
        complete_private_command_line("#completion mode t", "#completion mode t".len(), usize::MAX);

    assert_eq!(
        partial_nested_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["tab"]
    );

    let help_candidates = complete_private_command_line("#help ", "#help ".len(), usize::MAX);
    assert_eq!(
        help_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        [
            "commands",
            "keys",
            "ai",
            "completion",
            "templates",
            "sync",
            "encryption",
            "config"
        ]
    );

    let partial_help_candidates = complete_private_command_line("#help c", "#help c".len(), 10);
    assert_eq!(
        partial_help_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["commands", "completion", "config"]
    );

    assert!(complete_private_command_line("# ", "# ".len(), usize::MAX).is_empty());
}

#[test]
fn complete_non_first_token_for_line_matches_template_placeholder_name_without_braces() {
    let templates = vec![TemplateEntry::new("echo {something}")];

    let candidates = complete_non_first_token_for_line_with_options(
        "echo something",
        "echo something".len(),
        Path::new("/"),
        &[],
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "echo {something}".to_string(),
            replacement: "{something}".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        })
    );
}

#[test]
fn complete_non_first_token_for_line_treats_template_as_whole_command_shape() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "command add 100 other".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let templates = vec![TemplateEntry::new("command add {amount} file")];

    let candidates = complete_non_first_token_for_line_with_options(
        "command add 200",
        "command add 200".len(),
        Path::new("/"),
        &history,
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "command add {amount} file".to_string(),
            replacement: "200 file".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        })
    );
}

#[test]
fn complete_non_first_token_for_line_prefers_structural_template_position() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "echo {a} {something}".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let templates = vec![
        TemplateEntry::new("echo {a} {older}"),
        TemplateEntry::new("echo {a} {b} {c}"),
    ];

    let candidates = complete_non_first_token_for_line_with_options(
        "echo {a} {something}",
        "echo {a} {something}".len(),
        Path::new("/"),
        &history,
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [
            CompletionCandidate {
                display: "echo {a} {b} {c}".to_string(),
                replacement: "{b} {c}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
            CompletionCandidate {
                display: "echo {a} {older}".to_string(),
                replacement: "{older}".to_string(),
                is_dir: false,
                source: CompletionSource::Template,
            },
        ]
    );
}

#[test]
fn complete_non_first_token_for_line_keeps_whole_history_suffix() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "echo word-alpha word-beta word-gamma".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "echo word",
        "echo word".len(),
        Path::new("/"),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "word-alpha word-beta word-gamma".to_string(),
            replacement: "word-alpha word-beta word-gamma".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );
}

#[test]
fn matches_completion_prefix_can_ignore_spaces() {
    assert!(matches_completion_prefix("git status", "g s", true));
    assert!(!matches_completion_prefix("git status", "g s", false));
    assert!(!matches_completion_prefix_with_threshold(
        "git status",
        "",
        true,
        0
    ));
    assert!(!matches_completion_prefix_with_threshold(
        "git status",
        "g x",
        true,
        50
    ));
    assert!(!matches_completion_prefix_with_threshold(
        "git status",
        "gs",
        true,
        50
    ));
    assert!(!matches_completion_prefix_with_threshold(
        "git status",
        "gs",
        true,
        49
    ));
    assert!(!matches_completion_prefix_with_threshold(
        "status", "stx", true, 50
    ));
}

#[test]
fn typo_candidates_use_dedicated_typo_threshold() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "git status --short".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_typos_for_line_with_options(
        "git statuz",
        "git statuz".len(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "git status --short".to_string(),
            replacement: "git status --short".to_string(),
            is_dir: false,
            source: CompletionSource::HistoryTypo,
        })
    );

    let token = current_token_context("git statuz", "git statuz".len());
    let accepted = accept_completion("git statuz", &token, &candidates[0]);
    assert_eq!(accepted.line, "git status --short");

    let previous_word_typo_candidates = complete_non_first_token_typos_for_line_with_options(
        "git statuz --",
        "git statuz --".len(),
        &history,
        &[],
        CompletionOptions::default(),
    );
    let token = current_token_context("git statuz --", "git statuz --".len());
    let accepted = accept_completion(
        "git statuz --",
        &token,
        previous_word_typo_candidates.first().unwrap(),
    );
    assert_eq!(accepted.line, "git status --short");

    let strict_candidates = complete_non_first_token_typos_for_line_with_options(
        "git statuz",
        "git statuz".len(),
        &history,
        &[],
        CompletionOptions {
            typo_threshold_percent: 90,
            ..CompletionOptions::default()
        },
    );
    assert!(strict_candidates.is_empty());

    let disabled_candidates = complete_non_first_token_typos_for_line_with_options(
        "git statuz",
        "git statuz".len(),
        &history,
        &[],
        CompletionOptions {
            fuzzy_enabled: false,
            ..CompletionOptions::default()
        },
    );
    assert!(disabled_candidates.is_empty());
}

#[test]
fn typo_candidates_rank_before_partial_structural_history_suffixes() {
    let mut candidates = vec![
        CompletionCandidate {
            display: "--short".to_string(),
            replacement: "--short".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        },
        CompletionCandidate {
            display: "git status --short".to_string(),
            replacement: "git status --short".to_string(),
            is_dir: false,
            source: CompletionSource::HistoryTypo,
        },
    ];

    rank_completion_candidates(&mut candidates);

    assert_eq!(candidates[0].source, CompletionSource::HistoryTypo);
    assert_eq!(candidates[0].replacement, "git status --short");
}

#[test]
fn first_token_typo_candidates_replace_whole_command() {
    let history = vec![HistoryEntry {
        t: 1,
        command: "kubectl get pods".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];
    let templates = vec![TemplateEntry::new("kubectl apply -f {file}")];

    let candidates = complete_first_token_typos_with_options(
        "kubectx",
        &history,
        &templates,
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates,
        [
            CompletionCandidate {
                display: "kubectl apply -f {file}".to_string(),
                replacement: "kubectl apply -f {file}".to_string(),
                is_dir: false,
                source: CompletionSource::TemplateTypo,
            },
            CompletionCandidate {
                display: "kubectl get pods".to_string(),
                replacement: "kubectl get pods".to_string(),
                is_dir: false,
                source: CompletionSource::HistoryTypo,
            },
        ]
    );

    let token = current_token_context("kubectx", "kubectx".len());
    let accepted = accept_completion("kubectx", &token, &candidates[0]);
    assert_eq!(accepted.line, "kubectl apply -f {file}");
}

#[test]
fn render_completion_candidates_labels_sources_without_mutating_input() {
    let candidates = vec![
        CompletionCandidate {
            display: "deploy".to_string(),
            replacement: "kubectl apply -f {file}".to_string(),
            is_dir: false,
            source: CompletionSource::Template,
        },
        CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        },
    ];

    assert_eq!(
        render_completion_candidates(&candidates),
        ["template\tdeploy", "file\tsrc/main.rs"]
    );
}

#[test]
fn render_completion_candidates_labels_directories_separately_from_files() {
    let candidates = vec![
        CompletionCandidate {
            display: "src/".to_string(),
            replacement: "src/".to_string(),
            is_dir: true,
            source: CompletionSource::Path,
        },
        CompletionCandidate {
            display: "src/main.rs".to_string(),
            replacement: "src/main.rs".to_string(),
            is_dir: false,
            source: CompletionSource::Path,
        },
    ];

    assert_eq!(
        render_completion_candidates(&candidates),
        ["dir\tsrc/", "file\tsrc/main.rs"]
    );
}

#[test]
fn render_completion_candidates_for_width_elides_without_wrapping() {
    let token = current_token_context("cat very-long", "cat very-long".len());
    let candidates = vec![CompletionCandidate {
        display: "very-long-file-name-that-will-not-fit.txt".to_string(),
        replacement: "very-long-file-name-that-will-not-fit.txt".to_string(),
        is_dir: false,
        source: CompletionSource::Path,
    }];

    let rows = render_completion_candidates_for_width(&candidates, "cat very-long", &token, 5, 24);

    assert_eq!(rows, ["file ...will-not-fit.txt"]);
    assert!(display_width(&rows[0]) <= 24);
}

#[test]
fn render_completion_candidates_for_width_elides_cjk_by_display_width() {
    let token = current_token_context("echo", "echo".len());
    let candidates = vec![CompletionCandidate {
        display: "echo alpha 中文路径 beta".to_string(),
        replacement: "echo alpha 中文路径 beta".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    }];

    let rows = render_completion_candidates_for_width(&candidates, "echo", &token, 8, 18);

    assert_eq!(rows, ["history ... beta"]);
    assert!(display_width(&rows[0]) <= 18);
}

#[test]
fn render_completion_candidates_for_width_keeps_source_label_when_possible() {
    let token = current_token_context("git", "git".len());
    let candidates = vec![CompletionCandidate {
        display: "git status --short".to_string(),
        replacement: "git status --short".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    }];

    let rows = render_completion_candidates_for_width(&candidates, "git", &token, 8, 80);

    assert_eq!(rows, ["history git status --short"]);
}

#[test]
fn render_completion_candidates_for_width_shows_replacement_for_non_suffix_candidates() {
    let token = current_token_context("echo {a} {something}", "echo {a} {something}".len());
    let candidates = vec![CompletionCandidate {
        display: "echo {a} {b} {c}".to_string(),
        replacement: "{b} {c}".to_string(),
        is_dir: false,
        source: CompletionSource::Template,
    }];

    let rows =
        render_completion_candidates_for_width(&candidates, "echo {a} {something}", &token, 9, 80);

    assert_eq!(rows, ["template echo {a} {b} {c}"]);
}

#[test]
fn render_completion_candidates_for_width_left_elides_by_words() {
    let token = current_token_context("kubectl", "kubectl".len());
    let candidates = vec![CompletionCandidate {
        display: "kubectl apply -f deployment.yaml --namespace production".to_string(),
        replacement: "kubectl apply -f deployment.yaml --namespace production".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    }];

    let rows = render_completion_candidates_for_width(&candidates, "kubectl", &token, 8, 34);

    assert_eq!(rows, ["history ... --namespace production"]);
}

#[test]
fn ghost_completion_suffix_is_display_only_tail() {
    let token = current_token_context("git sta", "git sta".len());
    let candidate = CompletionCandidate {
        display: "status".to_string(),
        replacement: "status".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        ghost_completion_suffix(&token, &candidate),
        Some("tus".to_string())
    );
}

#[test]
fn ghost_completion_suffix_works_across_completion_sources() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("mytool");
    std::fs::write(&executable, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
    }
    let templates = vec![TemplateEntry::new("git add . && git commit")];
    let history = vec![HistoryEntry {
        t: 1,
        command: "git checkout feature/test".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let template_token = current_token_context("git", 3);
    let template = complete_first_token("git", &templates, &[], &[]);
    assert_eq!(
        ghost_completion_suffix(&template_token, &template[0]),
        Some(" add . && git commit".to_string())
    );

    let executable_token = current_token_context("my", 2);
    let executable = complete_first_token("my", &[], &[], &[bin]);
    assert_eq!(
        ghost_completion_suffix(&executable_token, &executable[0]),
        Some("tool".to_string())
    );

    let path_token = current_token_context("cat sr", "cat sr".len());
    let path = complete_path("sr", temp.path());
    assert_eq!(
        ghost_completion_suffix(&path_token, &path[0]),
        Some("c/".to_string())
    );

    let argument_token = current_token_context("git checkout fea", "git checkout fea".len());
    let argument = complete_non_first_token("fea", temp.path(), &history, &[]);
    assert_eq!(
        ghost_completion_suffix(&argument_token, &argument[0]),
        Some("ture/test".to_string())
    );
}

#[test]
fn accept_completion_replaces_token_and_returns_new_cursor() {
    let line = "git sta --short";
    let token = current_token_context(line, 7);
    let candidate = CompletionCandidate {
        display: "status".to_string(),
        replacement: "status".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        accept_completion(line, &token, &candidate),
        AcceptedCompletion {
            line: "git status --short".to_string(),
            cursor: 10,
        }
    );
}

#[test]
fn accept_completion_word_mode_stops_at_next_word_boundary() {
    let line = "kub";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "kubectl apply -f file.yaml".to_string(),
        replacement: "kubectl apply -f file.yaml".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "kubectl".to_string(),
            cursor: "kubectl".len(),
        }
    );
}

#[test]
fn accept_completion_word_mode_includes_leading_space_and_next_word() {
    let line = "git";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "git status --short".to_string(),
        replacement: "git status --short".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "git status".to_string(),
            cursor: "git status".len(),
        }
    );
}

#[test]
fn accept_completion_word_mode_uses_full_suffix_without_boundary() {
    let line = "cat Car";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "Cargo.toml".to_string(),
        replacement: "Cargo.toml".to_string(),
        is_dir: false,
        source: CompletionSource::Path,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "cat Cargo.toml".to_string(),
            cursor: "cat Cargo.toml".len(),
        }
    );
}

#[test]
fn accept_completion_word_mode_stops_at_next_word_for_non_prefix_replacement() {
    let line = "echo {a} {something}";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "echo {a} {b} {c}".to_string(),
        replacement: "{b} {c}".to_string(),
        is_dir: false,
        source: CompletionSource::Template,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "echo {a} {b}".to_string(),
            cursor: "echo {a} {b}".len(),
        }
    );
}

#[test]
fn command_arguments_preserve_quoted_argument_spaces() {
    assert_eq!(
        command_arguments("git commit -m 'hello world' -- file"),
        ["commit", "-m", "hello world", "--", "file"]
    );
}

#[test]
fn complete_path_returns_empty_for_missing_directory() {
    let temp = tempfile::tempdir().unwrap();

    assert!(complete_path("missing/file", temp.path()).is_empty());
}

#[test]
fn cursor_is_snapped_to_previous_utf8_boundary() {
    assert_eq!(current_token_context("echo λ", 6).end, 5);
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
