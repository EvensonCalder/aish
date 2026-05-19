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

#[test]
fn template_remote_add_list_rm_persists_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("home");
    let remote = temp.path().join("shared templates.git");
    let mut state = template_sharing_state(&root);

    let output = run_template_private_command(
        &mut state,
        &format!("remote add shared {}", remote.display()),
    );

    assert!(output.contains(&format!("template.remote.shared={}", remote.display())));
    assert!(output.contains("no git command run"));
    assert_eq!(state.template_sharing_config.remotes.len(), 1);
    assert_eq!(
        state.template_sharing_config.remotes[0].remote,
        remote.display().to_string()
    );
    let loaded = config::load_config(&root.join("config.toml")).unwrap();
    assert_eq!(loaded.template_sharing.remotes.len(), 1);
    assert_eq!(loaded.template_sharing.remotes[0].name, "shared");

    let output = run_template_private_command(&mut state, "remote list");
    assert!(output.contains(&format!("template remote shared\t{}", remote.display())));

    let output = run_template_private_command(&mut state, "remote rm shared");
    assert!(output.contains("template remote removed: shared"));
    let loaded = config::load_config(&root.join("config.toml")).unwrap();
    assert!(loaded.template_sharing.remotes.is_empty());
}

#[test]
fn template_remote_add_rejects_invalid_name_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("home");
    let mut state = template_sharing_state(&root);

    let output =
        run_template_private_command(&mut state, "remote add ../bad /tmp/aish-templates.git");

    assert!(output.contains("usage: #template remote add <name> <git-url>"));
    assert!(state.template_sharing_config.remotes.is_empty());
    let loaded = config::load_config(&root.join("config.toml")).unwrap();
    assert!(loaded.template_sharing.remotes.is_empty());
}

#[test]
fn template_publish_writes_template_only_remote() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("templates.git");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    let root = temp.path().join("home");
    let mut state = template_sharing_state(&root);
    run_template_private_command(
        &mut state,
        &format!("remote add shared {}", remote.display()),
    );
    state
        .append_template(&TemplateEntry::new("kubectl get pods -n {namespace}"))
        .unwrap();
    state
        .append_template(&TemplateEntry::new("rsync -avz {from} {to}"))
        .unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(root.join("history/regular.jsonl"), "private history\n").unwrap();

    let output = run_template_private_command(&mut state, "publish shared");

    assert!(
        output.contains("template publish completed: shared (local=2, remote=2, encryption=none)")
    );
    let tree = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "ls-tree",
            "-r",
            "--name-only",
            "main",
        ],
    );
    assert!(tree.contains(".aish-template-remote.toml"));
    assert!(tree.contains("README.md"));
    assert!(tree.contains("templates/templates.jsonl"));
    assert!(!tree.contains("history/"));
    assert!(!tree.contains("config.toml"));
    let templates = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "main:templates/templates.jsonl",
        ],
    );
    assert!(templates.contains("kubectl get pods"));
    assert!(templates.contains("rsync -avz"));
}

#[test]
fn template_fetch_pending_and_import_are_reviewable_and_deduplicated() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("templates.git");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);

    let publisher_root = temp.path().join("publisher");
    let mut publisher = template_sharing_state(&publisher_root);
    run_template_private_command(
        &mut publisher,
        &format!("remote add shared {}", remote.display()),
    );
    publisher
        .append_template(&TemplateEntry::new("kubectl get pods -n {namespace}"))
        .unwrap();
    publisher
        .append_template(&TemplateEntry::new("rsync -avz {from} {to}"))
        .unwrap();
    run_template_private_command(&mut publisher, "publish shared");

    let consumer_root = temp.path().join("consumer");
    let mut consumer = template_sharing_state(&consumer_root);
    run_template_private_command(
        &mut consumer,
        &format!("remote add shared {}", remote.display()),
    );
    consumer
        .append_template(&TemplateEntry::new("kubectl get pods -n {namespace}"))
        .unwrap();

    let output = run_template_private_command(&mut consumer, "fetch shared");
    assert!(output.contains("template fetch completed: shared (templates=2)"));

    let output = run_template_private_command(&mut consumer, "pending shared rsync");
    let rsync_id = template_id("rsync -avz {from} {to}");
    assert!(output.contains(&format!("template {rsync_id}\trsync -avz")));
    assert!(!output.contains("kubectl get pods"));

    let output = run_template_private_command(&mut consumer, "analyze shared");
    assert!(output.contains(&format!("template {rsync_id}\tnew\trsync -avz")));
    assert!(output.contains("present\tkubectl get pods"));
    assert!(output.contains("template analysis completed: fetched=2 matched=2 new=1 present=1"));

    let output = run_template_private_command(&mut consumer, &format!("import shared {rsync_id}"));
    assert!(output.contains(&format!("template imported: {rsync_id}")));
    assert!(output.contains("template import completed: imported=1 skipped=0"));
    let loaded = consumer.load_templates().unwrap();
    assert_eq!(loaded.items.len(), 2);

    let output = run_template_private_command(&mut consumer, "import shared all");
    assert!(output.contains("template import completed: imported=0 skipped=2"));
    let loaded = consumer.load_templates().unwrap();
    assert_eq!(loaded.items.len(), 2);
}

#[test]
fn template_fetch_refuses_private_sync_repository() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("private-sync.git");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    let seed = temp.path().join("seed");
    fs::create_dir_all(&seed).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    fs::write(seed.join(".aish-sync.toml"), "version = 1\n").unwrap();
    fs::create_dir_all(seed.join("templates")).unwrap();
    fs::write(
        seed.join("templates/templates.jsonl"),
        "{\"body\":\"should not import\"}\n",
    )
    .unwrap();
    run_test_git(
        &seed,
        ["add", ".aish-sync.toml", "templates/templates.jsonl"],
    );
    run_test_git(&seed, ["commit", "-m", "seed private sync"]);
    run_test_git(&seed, ["branch", "-M", "main"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);

    let root = temp.path().join("home");
    let mut state = template_sharing_state(&root);
    run_template_private_command(
        &mut state,
        &format!("remote add shared {}", remote.display()),
    );

    let output = run_template_private_command(&mut state, "fetch shared");

    assert!(output.contains(
        "template remote appears to be a private Aish sync repository; use a separate template remote"
    ));
    assert!(!output.contains("template fetch completed"));
}

#[cfg(unix)]
#[test]
fn encrypted_template_remote_publish_analyze_and_import_use_local_private_key() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fake_gpg = write_fake_gpg(&temp);
    let old_gpg = std::env::var_os("AISH_GPG");
    unsafe {
        std::env::set_var("AISH_GPG", &fake_gpg);
    }

    let remote = temp.path().join("templates.git");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);

    let seed_root = temp.path().join("seed-publisher");
    let mut seed = template_sharing_state(&seed_root);
    run_template_private_command(
        &mut seed,
        &format!("remote add shared {}", remote.display()),
    );
    seed.append_template(&TemplateEntry::new("remote only template"))
        .unwrap();
    let output = run_template_private_command(&mut seed, "publish shared");
    assert!(output.contains("encryption=none"), "{output}");

    let encrypted_root = temp.path().join("encrypted-publisher");
    let mut publisher = template_sharing_state(&encrypted_root);
    run_template_private_command(
        &mut publisher,
        &format!("remote add shared {}", remote.display()),
    );
    publisher
        .append_template(&TemplateEntry::new("secret deploy {target}"))
        .unwrap();
    let output = run_template_private_command(
        &mut publisher,
        "publish shared --encrypt test@example.invalid",
    );
    assert!(output.contains("encryption=gpg"), "{output}");

    let tree = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "ls-tree",
            "-r",
            "--name-only",
            "main",
        ],
    );
    assert!(tree.contains("README.md"));
    assert!(tree.contains(".aish-template-remote.toml"));
    assert!(tree.contains("templates/templates.jsonl.gpg"));
    assert!(!tree.contains("templates/templates.jsonl\n"));
    let metadata = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "main:.aish-template-remote.toml",
        ],
    );
    assert!(metadata.contains("encryption = \"gpg\""));
    assert!(metadata.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));

    let consumer_root = temp.path().join("consumer");
    let mut consumer = template_sharing_state(&consumer_root);
    run_template_private_command(
        &mut consumer,
        &format!("remote add shared {}", remote.display()),
    );
    let output = run_template_private_command(&mut consumer, "fetch shared");
    assert!(output.contains("template fetch completed: shared (templates=2)"));

    let output = run_template_private_command(&mut consumer, "analyze shared secret");
    let secret_id = template_id("secret deploy {target}");
    assert!(output.contains(&format!("template {secret_id}\tnew\tsecret deploy")));
    assert!(output.contains("template analysis completed: fetched=2 matched=1 new=1 present=0"));

    let output = run_template_private_command(&mut consumer, "import shared all");
    assert!(output.contains("template import completed: imported=2 skipped=0"));
    let loaded = consumer.load_templates().unwrap();
    assert_eq!(loaded.items.len(), 2);
    assert!(
        loaded
            .items
            .iter()
            .any(|template| template.body == "remote only template")
    );
    assert!(
        loaded
            .items
            .iter()
            .any(|template| template.body == "secret deploy {target}")
    );

    unsafe {
        match old_gpg {
            Some(value) => std::env::set_var("AISH_GPG", value),
            None => std::env::remove_var("AISH_GPG"),
        }
    }
}

fn template_sharing_state(root: &Path) -> AppState {
    fs::create_dir_all(root).unwrap();
    let config_path = root.join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    AppState {
        config_path: Some(config_path),
        template_store_path: Some(root.join("templates/templates.jsonl")),
        events_path: Some(root.join("events.jsonl")),
        ..AppState::default()
    }
}

fn run_template_private_command(state: &mut AppState, args: &str) -> String {
    let mut output = Vec::new();
    super::super::private_commands::execute_private_command(state, &mut output, "template", args)
        .unwrap();
    String::from_utf8(output).unwrap()
}
