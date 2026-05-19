use super::*;

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
fn complete_non_first_token_for_line_preserves_quoted_history_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "git commit -m \"hello world\" -- file.txt".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "git commit -m h",
        "git commit -m h".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "\"hello world\" -- file.txt".to_string(),
            replacement: "\"hello world\" -- file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );

    let quoted_candidates = complete_non_first_token_for_line_with_options(
        "git commit -m \"h",
        "git commit -m \"h".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        quoted_candidates.first(),
        Some(&CompletionCandidate {
            display: "\"hello world\" -- file.txt".to_string(),
            replacement: "\"hello world\" -- file.txt".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );
}

#[test]
fn complete_non_first_token_for_line_preserves_mixed_quoted_history_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "cmd a\"b c\"d next".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    let candidates = complete_non_first_token_for_line_with_options(
        "cmd ab",
        "cmd ab".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions {
            match_threshold_percent: 100,
            ..CompletionOptions::default()
        },
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "a\"b c\"d next".to_string(),
            replacement: "a\"b c\"d next".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );
}

#[test]
fn complete_non_first_token_preserves_quoted_history_argument() {
    let temp = tempfile::tempdir().unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "git commit -m \"hello world\"".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    assert_eq!(
        complete_non_first_token("h", temp.path(), &history, &[]),
        [CompletionCandidate {
            display: "\"hello world\"".to_string(),
            replacement: "\"hello world\"".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        }]
    );

    assert_eq!(
        complete_non_first_token("\"h", temp.path(), &history, &[]),
        [CompletionCandidate {
            display: "\"hello world\"".to_string(),
            replacement: "\"hello world\"".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        }]
    );
}

#[test]
fn complete_non_first_token_preserves_escaped_space_history_argument() {
    let temp = tempfile::tempdir().unwrap();
    let history = vec![HistoryEntry {
        t: 1,
        command: "printf '<%s>\\n' hello\\ world".to_string(),
        exit_code: Some(0),
        source: crate::history::HistorySource::User,
    }];

    assert_eq!(
        complete_non_first_token("h", temp.path(), &history, &[]),
        [CompletionCandidate {
            display: "hello\\ world".to_string(),
            replacement: "hello\\ world".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        }]
    );

    let candidates = complete_non_first_token_for_line_with_options(
        "printf '<%s>\\n' hello\\ w",
        "printf '<%s>\\n' hello\\ w".len(),
        temp.path(),
        &history,
        &[],
        CompletionOptions::default(),
    );

    assert_eq!(
        candidates.first(),
        Some(&CompletionCandidate {
            display: "hello\\ world".to_string(),
            replacement: "hello\\ world".to_string(),
            is_dir: false,
            source: CompletionSource::History,
        })
    );
}
