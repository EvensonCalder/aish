use super::*;

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
