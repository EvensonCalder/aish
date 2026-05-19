use super::*;

#[test]
fn sync_config_commands_persist_without_running_git() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let events_path = temp.path().join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = temp.path().to_path_buf();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        events_path: Some(events_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        (
            "#set-remote git@example.invalid:aish.git",
            "sync.remote=git@example.invalid:aish.git",
        ),
        ("#sync 0 * * * *", "sync.schedule=0 * * * *"),
        ("#sync ai on", "sync.ai=true"),
        ("#sync history on", "sync.history=true"),
        ("#sync templates on", "sync.templates=true"),
        ("#sync drafts on", "sync.drafts=true"),
        ("#sync drafts off", "sync.drafts=false"),
        ("#sync startup on", "sync.startup=true"),
        ("#sync exit on", "sync.exit=true"),
        ("#sync off", "sync.enabled=false"),
    ] {
        state.draft.insert_str(line);
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
        assert!(
            output.contains("no git command run") || output.contains("no scheduler file created")
        );
    }

    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.sync.remote, "git@example.invalid:aish.git");
    assert!(!loaded.sync.enabled);
    assert!(loaded.sync.schedule.is_empty());
    assert!(loaded.sync.ai);
    assert!(loaded.sync.history);
    assert!(loaded.sync.templates);
    assert!(!loaded.sync.drafts);
    assert!(loaded.sync.startup);
    assert!(loaded.sync.exit);
    let events = load_events(&events_path).unwrap();
    assert_eq!(events.items.len(), 10);
    assert!(
        events
            .items
            .iter()
            .all(|event| event.msg == "sync config changed")
    );
}

#[test]
fn push_sync_runs_against_configured_local_git_remote() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");

    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(&seed).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&seed, ["config", "commit.gpgsign", "false"]);
    fs::write(seed.join("README.md"), "seed\n").unwrap();
    run_test_git(&seed, ["add", "README.md"]);
    run_test_git(&seed, ["commit", "-m", "seed"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);
    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), root.to_str().unwrap()],
    );
    run_test_git(&root, ["config", "user.name", "Aish Test"]);
    run_test_git(&root, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&root, ["config", "commit.gpgsign", "false"]);

    let config_path = root.join("config.toml");
    let events_path = root.join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = root.clone();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        sync_config: config.sync,
        clock: || 11,
        ..AppState::default()
    };
    state.draft.insert_str("#push");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(10),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("sync step ok: git pull --rebase"),
        "{output}"
    );
    assert!(
        output.contains("sync step ok: git add -- .gitignore"),
        "{output}"
    );
    assert!(output.contains("sync step ok: git commit"), "{output}");
    assert!(output.contains("sync step ok: git push"), "{output}");
    assert!(output.contains("sync push completed"), "{output}");
    assert!(root.join(".gitignore").exists());
    let events = load_events(&events_path).unwrap();
    assert!(
        events
            .items
            .iter()
            .any(|event| event.msg == "sync push completed")
    );
}

#[test]
fn foreground_shell_args_use_login_compatible_command_mode() {
    assert_eq!(
        foreground_shell_args("/bin/bash", "less file"),
        ["-lc", "less file"]
    );
    assert_eq!(
        foreground_shell_args("/bin/zsh", "vim file"),
        ["-lc", "vim file"]
    );
    assert_eq!(
        foreground_shell_args("/usr/bin/fish", "less file"),
        ["-c", "less file"]
    );
    assert_eq!(
        foreground_shell_args("  /usr/bin/FISH  ", "less file"),
        ["-c", "less file"]
    );
    assert_eq!(
        foreground_shell_args("/bin/sh", "less file"),
        ["-c", "less file"]
    );
    assert_eq!(
        foreground_shell_args("/bin/dash", "less file"),
        ["-c", "less file"]
    );
}

#[test]
fn startup_sync_runs_due_schedule_against_local_git_remote() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    let config_path = root.join("config.toml");
    let events_path = root.join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = root.clone();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config.sync.enabled = true;
    config.sync.schedule = "@hourly".to_string();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        events_path: Some(events_path.clone()),
        sync_config: config.sync,
        clock: || 3_600,
        ..AppState::default()
    };
    let mut output = Vec::new();

    run_startup_sync_check(&mut state, &root, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("startup sync due; running #push"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    assert_eq!(
        fs::read_to_string(root.join("cache/runtime/sync.last_attempt")).unwrap(),
        "3600\n"
    );
    let events = load_events(&events_path).unwrap();
    assert!(
        events
            .items
            .iter()
            .any(|event| event.msg == "sync push completed")
    );
}

#[test]
fn startup_sync_trigger_runs_without_periodic_schedule() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut state = AppState {
        sync_config: SyncConfig {
            startup: true,
            ..SyncConfig::default()
        },
        clock: || 42,
        ..AppState::default()
    };
    let mut output = Vec::new();

    run_startup_sync_check(&mut state, root, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("startup sync enabled; running #push"));
    assert!(output.contains("sync remote is not configured"));
    assert_eq!(
        fs::read_to_string(root.join("cache/runtime/sync.last_attempt")).unwrap(),
        "42\n"
    );
}

#[test]
fn startup_sync_skips_not_due_schedule_without_running_git() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let last_attempt = root.join("cache/runtime/sync.last_attempt");
    write_last_sync_attempt(&last_attempt, 3_500).unwrap();
    let mut state = AppState {
        sync_config: SyncConfig {
            remote: "git@example.invalid:aish.git".to_string(),
            enabled: true,
            schedule: "@hourly".to_string(),
            ..SyncConfig::default()
        },
        clock: || 3_600,
        ..AppState::default()
    };
    let mut output = Vec::new();

    run_startup_sync_check(&mut state, root, &mut output).unwrap();

    assert!(String::from_utf8(output).unwrap().is_empty());
    assert_eq!(fs::read_to_string(last_attempt).unwrap(), "3500\n");
}

#[test]
fn sync_category_toggle_rejects_invalid_usage_without_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    state.draft.insert_str("#sync ai maybe");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output = Vec::new();

    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("usage: #sync ai|history|templates|drafts on|off")
    );
    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.sync, SyncConfig::default());
}
