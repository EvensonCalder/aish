use super::*;

#[test]
fn mt_command_persists_template_entry() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#mt rsync {from} {to}");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    let id = template_id("rsync {from} {to}");
    assert!(output.contains(&format!("template stored: {id}")));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].body, "rsync {from} {to}");
}

#[test]
fn template_list_prints_template_bodies_newest_first() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(&template_path, &TemplateEntry::new("rsync {from} {to}")).unwrap();
    append_template(&template_path, &TemplateEntry::new("tail -f {file}")).unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template list");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(output, "tail -f {file}\nrsync {from} {to}\n");
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_find_prints_matching_hash_ids() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(&template_path, &TemplateEntry::new("rsync {from} {to}")).unwrap();
    append_template(&template_path, &TemplateEntry::new("tail -f {file}")).unwrap();
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template find rsync");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!(
        "template {}\trsync {{from}} {{to}}",
        template_id("rsync {from} {to}")
    )));
    assert!(!output.contains("tail -f"));
}

#[test]
fn template_rm_removes_matching_templates() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["rsync {from} {to}", "tail -f {file}", "rsync {from} {to}"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    let id = template_id("rsync {from} {to}");
    state.draft.insert_str(&format!("#template rm {id}"));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!("template removed: {id} (2)")));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].body, "tail -f {file}");
}

#[test]
fn template_replace_rewrites_matching_templates() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["old deploy", "tail -f {file}", "old deploy"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path.clone()),
        ..AppState::default()
    };
    let old_id = template_id("old deploy");
    let new_id = template_id("new deploy body");
    state
        .draft
        .insert_str(&format!("#template replace {old_id} new deploy body"));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!(
        "template replaced: {old_id} -> {new_id} (removed 2)"
    )));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());

    let loaded = load_templates(&template_path).unwrap();
    assert_eq!(loaded.errors, []);
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].body, "tail -f {file}");
    assert_eq!(loaded.items[1].body, "new deploy body");
}

#[test]
fn template_use_copies_newest_matching_body_to_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in [
        "old deploy",
        "tail -f {file}",
        "rsync {from} {user}@{host}:{to} {from}",
    ] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    let id = template_id("rsync {from} {user}@{host}:{to} {from}");
    state.draft.insert_str(&format!(
        "#template use {id} from=src host=prod to=/srv/app zextra=ignored aextra=unused"
    ));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!("template copied to draft: {id}")));
    assert!(output.contains("template placeholders: from, user, host, to"));
    assert!(output.contains("unresolved template placeholders: user"));
    assert!(output.contains("unused template values: aextra, zextra"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "rsync src {user}@prod:/srv/app src");
    assert_eq!(state.draft.cursor(), state.draft.as_str().len());
}

#[test]
fn template_use_reports_missing_template_without_changing_draft() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str("#template use missing");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("template not found: missing"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_use_supports_quoted_values_with_spaces() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(
        &template_path,
        &TemplateEntry::new("echo {message} && cd {path}"),
    )
    .unwrap();
    let id = template_id("echo {message} && cd {path}");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!(
        "#template use {id} message=\"hello world\" path='/tmp/my dir'"
    ));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!("template copied to draft: {id}")));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo hello world && cd /tmp/my dir");
}

#[test]
fn template_use_supports_described_and_variadic_placeholders() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    append_template(
        &template_path,
        &TemplateEntry::new("git commit -m {message:commit message} -- {paths...}"),
    )
    .unwrap();
    let id = template_id("git commit -m {message:commit message} -- {paths...}");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!(
        "#template use {id} message='ship it' paths='src tests'"
    ));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("template placeholders: message, paths"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "git commit -m ship it -- src tests");
    assert!(state.draft_from_template);
}

#[test]
fn unresolved_template_placeholders_do_not_execute() {
    let mut state = AppState {
        draft_from_template: true,
        ..AppState::default()
    };
    state.draft.insert_str("echo {message}");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("cannot execute unresolved template placeholders: message"));
    assert_eq!(state.last_status, None);
    assert_eq!(state.mode, Mode::Draft);
    assert_eq!(state.draft.as_str(), "echo {message}");
}

#[test]
fn template_show_prints_newest_matching_body() {
    let temp = tempfile::tempdir().unwrap();
    let template_path = temp.path().join("templates/templates.jsonl");
    for body in ["old deploy", "tail -f {file}", "new deploy"] {
        append_template(&template_path, &TemplateEntry::new(body)).unwrap();
    }
    let id = template_id("new deploy");
    let mut state = AppState {
        template_store_path: Some(template_path),
        ..AppState::default()
    };
    state.draft.insert_str(&format!("#template show {id}"));
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!("template: {id}")));
    assert!(output.contains("new deploy"));
    assert_eq!(state.last_status, None);
    assert!(state.draft.is_empty());
}

#[test]
fn template_commands_report_usage_for_invalid_input() {
    let usage = template_usage();
    for (line, expected) in [
        ("#mt", "usage: #mt <template-body>"),
        ("#template rm", usage),
        ("#template replace deploy", usage),
        ("#template show", usage),
        ("#template use", usage),
        ("#template find", usage),
        ("#template", usage),
        ("#template unknown deploy", usage),
    ] {
        let mut state = AppState::default();
        state.draft.insert_str(line);
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        execute_draft(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
        assert_eq!(state.last_status, None);
        assert!(state.draft.is_empty());
    }
}
