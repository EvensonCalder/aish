use super::*;

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
fn completion_matcher_centralizes_structural_and_typo_rules() {
    let matcher = CompletionMatcher::new(true, 50, 80);

    assert!(matcher.prefix_matches("git status", "g s"));
    assert!(matcher.words_match_threshold(
        &["git".to_string(), "status".to_string()],
        &["git".to_string(), "sta".to_string()],
    ));
    assert!(matcher.typo_matches("status", "statuz"));
    assert!(matcher.words_match_threshold_with_typos(
        &["git".to_string(), "status".to_string()],
        &["git".to_string(), "statuz".to_string()],
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
