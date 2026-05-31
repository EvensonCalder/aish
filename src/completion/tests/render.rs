use super::*;

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
        CompletionCandidate {
            display: "status".to_string(),
            replacement: "status".to_string(),
            is_dir: false,
            source: CompletionSource::BackendShell,
        },
    ];

    assert_eq!(
        render_completion_candidates(&candidates),
        ["template\tdeploy", "file\tsrc/main.rs", "shell\tstatus"]
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
