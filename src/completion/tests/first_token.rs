use super::*;

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
