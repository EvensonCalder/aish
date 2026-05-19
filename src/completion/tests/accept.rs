use super::*;

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
fn completion_edit_records_token_replacement_span() {
    let line = "git sta --short";
    let token = current_token_context(line, 7);
    let candidate = CompletionCandidate {
        display: "status".to_string(),
        replacement: "status".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    let edit = completion_edit_for_candidate(line, &token, &candidate, CompletionTabAccept::Full);

    assert_eq!(
        edit,
        CompletionEdit {
            start: 4,
            end: 7,
            replacement: "status".to_string(),
        }
    );
    assert_eq!(
        edit.apply_to_line(line),
        AcceptedCompletion {
            line: "git status --short".to_string(),
            cursor: 10,
        }
    );
}

#[test]
fn completion_edit_records_whole_line_replacement_for_typos() {
    let line = "git statuz";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "git status --short".to_string(),
        replacement: "git status --short".to_string(),
        is_dir: false,
        source: CompletionSource::HistoryTypo,
    };

    assert_eq!(
        completion_edit_for_candidate(line, &token, &candidate, CompletionTabAccept::Word),
        CompletionEdit {
            start: 0,
            end: line.len(),
            replacement: "git status --short".to_string(),
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
fn accept_completion_word_mode_keeps_quoted_shell_word_intact() {
    let line = "git commit -m h";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "\"hello world\" -- file.txt".to_string(),
        replacement: "\"hello world\" -- file.txt".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "git commit -m \"hello world\"".to_string(),
            cursor: "git commit -m \"hello world\"".len(),
        }
    );

    let quoted_line = "git commit -m \"h";
    let quoted_token = current_token_context(quoted_line, quoted_line.len());

    assert_eq!(
        accept_completion_with_mode(
            quoted_line,
            &quoted_token,
            &candidate,
            CompletionTabAccept::Word,
        ),
        AcceptedCompletion {
            line: "git commit -m \"hello world\"".to_string(),
            cursor: "git commit -m \"hello world\"".len(),
        }
    );
}

#[test]
fn accept_completion_word_mode_keeps_escaped_space_shell_word_intact() {
    let line = "printf '<%s>\\n' h";
    let token = current_token_context(line, line.len());
    let candidate = CompletionCandidate {
        display: "hello\\ world next".to_string(),
        replacement: "hello\\ world next".to_string(),
        is_dir: false,
        source: CompletionSource::History,
    };

    assert_eq!(
        accept_completion_with_mode(line, &token, &candidate, CompletionTabAccept::Word),
        AcceptedCompletion {
            line: "printf '<%s>\\n' hello\\ world".to_string(),
            cursor: "printf '<%s>\\n' hello\\ world".len(),
        }
    );
}
