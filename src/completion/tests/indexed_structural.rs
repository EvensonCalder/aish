use super::*;

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
