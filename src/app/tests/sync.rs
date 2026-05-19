use super::*;

fn git_env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap()
}

const LOCAL_TEST_FINGERPRINT: &str = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
const REMOTE_TEST_FINGERPRINT: &str = "FEDCBA9876543210FEDCBA9876543210FEDCBA98";

#[test]
fn sync_config_commands_persist_without_running_git() {
    let _guard = git_env_guard();
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
fn sync_now_runs_against_configured_local_git_remote() {
    let _guard = git_env_guard();
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
    fs::write(seed.join(".seed"), "seed\n").unwrap();
    run_test_git(&seed, ["add", ".seed"]);
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
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("history/ai.jsonl"), "{\"text\":\"ai\"}\n").unwrap();
    fs::write(root.join("history/draft.jsonl"), "{\"text\":\"draft\"}\n").unwrap();
    fs::write(root.join("history/notes.jsonl"), "{\"text\":\"note\"}\n").unwrap();
    fs::write(
        root.join("history/regular.jsonl"),
        "{\"command\":\"regular\"}\n",
    )
    .unwrap();
    fs::write(
        root.join("templates/templates.jsonl"),
        "{\"body\":\"template\"}\n",
    )
    .unwrap();

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
    state.draft.insert_str("#sync now");
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
        output.contains("sync step ok: git add -- .aish-sync.toml .gitattributes .gitignore README.md history/ai.jsonl history/draft.jsonl history/notes.jsonl history/regular.jsonl templates/templates.jsonl"),
        "{output}"
    );
    assert!(
        output.contains("sync step ok: git pull --no-rebase --no-edit"),
        "{output}"
    );
    assert!(output.contains("sync step ok: git commit"), "{output}");
    assert!(output.contains("sync step ok: git push"), "{output}");
    assert!(output.contains("sync push completed"), "{output}");
    assert!(root.join(".gitignore").exists());
    assert!(root.join(".gitattributes").exists());
    assert!(root.join("README.md").exists());
    let pushed_metadata = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:.aish-sync.toml",
        ],
    );
    assert!(pushed_metadata.contains("enabled = false"));
    let pushed_history = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:history/regular.jsonl",
        ],
    );
    assert!(pushed_history.contains("regular"));
    let events = load_events(&events_path).unwrap();
    assert!(
        events
            .items
            .iter()
            .any(|event| event.msg == "sync push completed")
    );
}

#[test]
fn sync_bootstraps_local_git_identity_when_missing() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let empty_global = temp.path().join("empty-gitconfig");
    fs::write(&empty_global, "").unwrap();
    let old_global = std::env::var_os("GIT_CONFIG_GLOBAL");
    let old_nosystem = std::env::var_os("GIT_CONFIG_NOSYSTEM");
    unsafe {
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty_global);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }

    let remote = temp.path().join("remote.git");
    let root = temp.path().join("aish-home");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(
        root.join("history/regular.jsonl"),
        "{\"command\":\"identity\"}\n",
    )
    .unwrap();

    let mut state = sync_state_for_root(&root, &remote);
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    unsafe {
        match old_global {
            Some(value) => std::env::set_var("GIT_CONFIG_GLOBAL", value),
            None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
        }
        match old_nosystem {
            Some(value) => std::env::set_var("GIT_CONFIG_NOSYSTEM", value),
            None => std::env::remove_var("GIT_CONFIG_NOSYSTEM"),
        }
    }
    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("sync step ok: git config --local user.name Aish Sync"),
        "{output}"
    );
    assert!(
        output.contains("sync step ok: git config --local user.email aish-sync@localhost"),
        "{output}"
    );
    assert!(
        output.contains("sync step ok: git config --local commit.gpgsign false"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    assert_eq!(
        run_test_git_stdout(&root, ["config", "--local", "user.name"]).trim(),
        "Aish Sync"
    );
    assert_eq!(
        run_test_git_stdout(&root, ["config", "--local", "user.email"]).trim(),
        "aish-sync@localhost"
    );
    assert_eq!(
        run_test_git_stdout(&root, ["config", "--local", "commit.gpgsign"]).trim(),
        "false"
    );
}

#[test]
fn first_sync_merges_existing_remote_when_local_home_was_not_git_repo() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");

    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(seed.join("history")).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&seed, ["config", "commit.gpgsign", "false"]);
    crate::sync::maintain_managed_gitignore(seed.join(".gitignore")).unwrap();
    crate::sync::maintain_managed_gitattributes(seed.join(".gitattributes")).unwrap();
    crate::sync::maintain_sync_readme(seed.join("README.md")).unwrap();
    fs::write(
        seed.join("history/regular.jsonl"),
        "{\"command\":\"from-remote\"}\n",
    )
    .unwrap();
    run_test_git(
        &seed,
        [
            "add",
            "--",
            ".gitattributes",
            ".gitignore",
            "README.md",
            "history/regular.jsonl",
        ],
    );
    run_test_git(&seed, ["commit", "-m", "seed sync"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(
        root.join("history/regular.jsonl"),
        "{\"command\":\"from-local\"}\n",
    )
    .unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("git pull --no-rebase --no-edit --allow-unrelated-histories origin"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    let pushed_history = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:history/regular.jsonl",
        ],
    );
    assert!(pushed_history.contains("from-remote"), "{pushed_history}");
    assert!(pushed_history.contains("from-local"), "{pushed_history}");
}

#[test]
fn first_sync_uses_single_remote_branch_when_remote_head_is_unborn() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");

    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(seed.join("history")).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["branch", "-M", "sync-data"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&seed, ["config", "commit.gpgsign", "false"]);
    crate::sync::maintain_managed_gitignore(seed.join(".gitignore")).unwrap();
    crate::sync::maintain_managed_gitattributes(seed.join(".gitattributes")).unwrap();
    crate::sync::maintain_sync_readme(seed.join("README.md")).unwrap();
    fs::write(
        seed.join("history/regular.jsonl"),
        "{\"command\":\"remote-sync-branch\"}\n",
    )
    .unwrap();
    run_test_git(
        &seed,
        [
            "add",
            "--",
            ".gitattributes",
            ".gitignore",
            "README.md",
            "history/regular.jsonl",
        ],
    );
    run_test_git(&seed, ["commit", "-m", "seed sync"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(
        root.join("history/regular.jsonl"),
        "{\"command\":\"local-sync-branch\"}\n",
    )
    .unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("sync step ok: git branch -M sync-data"),
        "{output}"
    );
    assert!(
        output.contains(
            "git pull --no-rebase --no-edit --allow-unrelated-histories origin sync-data"
        ),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    let pushed_history = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "sync-data:history/regular.jsonl",
        ],
    );
    assert!(
        pushed_history.contains("remote-sync-branch"),
        "{pushed_history}"
    );
    assert!(
        pushed_history.contains("local-sync-branch"),
        "{pushed_history}"
    );
}

#[test]
fn existing_repo_retries_unrelated_remote_pull_with_merge_option() {
    let _guard = git_env_guard();
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
    fs::write(seed.join(".remote-seed"), "remote\n").unwrap();
    run_test_git(&seed, ["add", ".remote-seed"]);
    run_test_git(&seed, ["commit", "-m", "remote seed"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);

    fs::create_dir_all(&root).unwrap();
    run_test_git(&root, ["init"]);
    run_test_git(&root, ["config", "user.name", "Aish Test"]);
    run_test_git(&root, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&root, ["config", "commit.gpgsign", "false"]);
    fs::write(root.join(".local-seed"), "local\n").unwrap();
    run_test_git(&root, ["add", ".local-seed"]);
    run_test_git(&root, ["commit", "-m", "local seed"]);

    let mut state = sync_state_for_root(&root, &remote);
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains(
            "sync pull needs unrelated-history merge; retrying with --allow-unrelated-histories"
        ),
        "{output}"
    );
    assert!(
        output.contains("git pull --no-rebase --no-edit --allow-unrelated-histories origin"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    assert_eq!(
        run_test_git_stdout(
            temp.path(),
            [
                "--git-dir",
                remote.to_str().unwrap(),
                "show",
                "HEAD:.remote-seed"
            ],
        ),
        "remote\n"
    );
    assert_eq!(
        run_test_git_stdout(
            temp.path(),
            [
                "--git-dir",
                remote.to_str().unwrap(),
                "show",
                "HEAD:.local-seed"
            ],
        ),
        "local\n"
    );
    let readme = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:README.md",
        ],
    );
    assert!(readme.contains("Aish Sync Repository"), "{readme}");
}

#[test]
fn sync_skips_commit_when_only_untracked_unmanaged_files_exist() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    let config_path = root.join("config.toml");
    let mut config = config::Config::default();
    config.storage.home = root.clone();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config::save_config(&config_path, &config).unwrap();
    let mut state = AppState {
        config_path: Some(config_path),
        sync_config: config.sync,
        clock: || 12,
        ..AppState::default()
    };

    let mut first = Vec::new();
    run_manual_sync_push(&mut state, &mut first).unwrap();
    assert!(
        String::from_utf8(first)
            .unwrap()
            .contains("sync push completed")
    );

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(root.join("history/unmanaged.txt"), "local scratch\n").unwrap();
    let mut second = Vec::new();
    run_manual_sync_push(&mut state, &mut second).unwrap();

    let output = String::from_utf8(second).unwrap();
    assert!(
        output.contains("sync step skipped: nothing to commit"),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
}

#[test]
fn sync_warns_when_existing_managed_files_are_excluded_by_category() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(root.join("history/ai.jsonl.gpg"), "encrypted ai\n").unwrap();
    fs::write(root.join("history/draft.jsonl.gpg"), "encrypted draft\n").unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    state.sync_config.ai = false;
    state.sync_config.drafts = false;
    state.encryption_config = EncryptionConfig {
        enabled: true,
        key_fingerprint: LOCAL_TEST_FINGERPRINT.to_string(),
        ..EncryptionConfig::default()
    };

    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains(
            "warning: sync.ai=false; not staging existing Aish file history/ai.jsonl.gpg; run #sync ai on to include it"
        ),
        "{output}"
    );
    assert!(
        output.contains(
            "warning: sync.drafts=false; not staging existing Aish file history/draft.jsonl.gpg; run #sync drafts on to include it"
        ),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    let status = run_test_git_stdout(
        &root,
        [
            "status",
            "--short",
            "--",
            "history/ai.jsonl.gpg",
            "history/draft.jsonl.gpg",
        ],
    );
    assert!(status.contains("?? history/ai.jsonl.gpg"), "{status}");
    assert!(status.contains("?? history/draft.jsonl.gpg"), "{status}");
}

#[test]
fn encrypted_sync_writes_repository_key_metadata() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(
        root.join("history/regular.jsonl.gpg"),
        "encrypted regular\n",
    )
    .unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    state.encryption_config = EncryptionConfig {
        enabled: true,
        key_fingerprint: LOCAL_TEST_FINGERPRINT.to_string(),
        ..EncryptionConfig::default()
    };

    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains(
            "sync step ok: git add -- .aish-sync.toml .gitattributes .gitignore README.md history/regular.jsonl.gpg"
        ),
        "{output}"
    );
    assert!(output.contains("sync push completed"), "{output}");
    let pushed_metadata = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:.aish-sync.toml",
        ],
    );
    assert!(pushed_metadata.contains("enabled = true"));
    assert!(pushed_metadata.contains(LOCAL_TEST_FINGERPRINT));
}

#[test]
fn encrypted_sync_requires_full_local_fingerprint() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let root = temp.path().join("aish-home");
    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(&root).unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    state.encryption_config = EncryptionConfig {
        enabled: true,
        recipient: "evenson@example.invalid".to_string(),
        ..EncryptionConfig::default()
    };

    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("sync encryption key is not configured as a full GPG fingerprint"),
        "{output}"
    );
    assert!(output.contains("local key_fingerprint=evenson@example.invalid"));
    assert!(!output.contains("sync push completed"));
    assert!(!root.join(".git").exists());
}

#[test]
fn sync_rejects_repository_key_mismatch_before_staging() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);
    crate::sync::write_sync_repository_metadata(
        root.join(crate::sync::sync_repository_metadata_path()),
        &crate::sync::sync_repository_metadata_for(
            &SyncConfig::default(),
            true,
            REMOTE_TEST_FINGERPRINT,
        ),
    )
    .unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    state.encryption_config = EncryptionConfig {
        enabled: true,
        key_fingerprint: LOCAL_TEST_FINGERPRINT.to_string(),
        ..EncryptionConfig::default()
    };

    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("sync encryption key mismatch"), "{output}");
    assert!(output.contains(REMOTE_TEST_FINGERPRINT), "{output}");
    assert!(output.contains(LOCAL_TEST_FINGERPRINT), "{output}");
    assert!(output.contains("#encrypt rotate <chosen-full-key-fingerprint>"));
    assert!(!output.contains("sync push completed"));
}

#[test]
fn sync_rejects_remote_key_mismatch_before_staging_local_changes() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    let other = temp.path().join("other");
    seed_local_remote(&remote, &seed, &root);

    crate::sync::write_sync_repository_metadata(
        root.join(crate::sync::sync_repository_metadata_path()),
        &crate::sync::sync_repository_metadata_for(
            &SyncConfig::default(),
            true,
            LOCAL_TEST_FINGERPRINT,
        ),
    )
    .unwrap();
    run_test_git(&root, ["add", ".aish-sync.toml"]);
    run_test_git(&root, ["commit", "-m", "local metadata"]);
    run_test_git(&root, ["push"]);

    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), other.to_str().unwrap()],
    );
    run_test_git(&other, ["config", "user.name", "Aish Test"]);
    run_test_git(&other, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&other, ["config", "commit.gpgsign", "false"]);
    crate::sync::write_sync_repository_metadata(
        other.join(crate::sync::sync_repository_metadata_path()),
        &crate::sync::sync_repository_metadata_for(
            &SyncConfig::default(),
            true,
            REMOTE_TEST_FINGERPRINT,
        ),
    )
    .unwrap();
    run_test_git(&other, ["add", ".aish-sync.toml"]);
    run_test_git(&other, ["commit", "-m", "remote metadata"]);
    run_test_git(&other, ["push"]);

    let mut state = sync_state_for_root(&root, &remote);
    state.encryption_config = EncryptionConfig {
        enabled: true,
        key_fingerprint: LOCAL_TEST_FINGERPRINT.to_string(),
        ..EncryptionConfig::default()
    };
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("sync encryption key mismatch"), "{output}");
    assert!(!output.contains("sync step ok: git add"), "{output}");
    assert!(!output.contains("sync step ok: git push"), "{output}");
    assert!(!output.contains("sync push completed"));
}

#[test]
fn sync_adopts_remote_content_options_before_staging_local_files() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root = temp.path().join("aish-home");
    seed_local_remote(&remote, &seed, &root);

    let remote_config = SyncConfig {
        ai: false,
        history: false,
        templates: false,
        drafts: false,
        ..SyncConfig::default()
    };
    crate::sync::write_sync_repository_metadata(
        seed.join(crate::sync::sync_repository_metadata_path()),
        &crate::sync::sync_repository_metadata_for(&remote_config, false, ""),
    )
    .unwrap();
    run_test_git(&seed, ["add", ".aish-sync.toml"]);
    run_test_git(&seed, ["commit", "-m", "remote content options"]);
    run_test_git(&seed, ["push"]);

    fs::create_dir_all(root.join("history")).unwrap();
    fs::write(
        root.join("history/regular.jsonl"),
        "{\"command\":\"local-only\"}\n",
    )
    .unwrap();
    let mut state = sync_state_for_root(&root, &remote);
    let mut output = Vec::new();
    run_manual_sync_push(&mut state, &mut output).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("warning: repository sync content options differ"),
        "{output}"
    );
    assert!(
        output.contains(
            "warning: sync.history=false; not staging existing Aish file history/regular.jsonl; run #sync history on to include it"
        ),
        "{output}"
    );
    assert!(
        output.contains(
            "sync step ok: git add -- .aish-sync.toml .gitattributes .gitignore README.md"
        ),
        "{output}"
    );
    assert!(
        !output.contains(
            "sync step ok: git add -- .aish-sync.toml .gitattributes .gitignore README.md history/regular.jsonl"
        ),
        "{output}"
    );
    assert!(!state.sync_config.history);
    assert!(!state.sync_config.templates);
    let loaded = config::load_config(&root.join("config.toml")).unwrap();
    assert!(!loaded.sync.history);
    assert!(!loaded.sync.templates);

    let status = run_test_git_stdout(&root, ["status", "--short", "--", "history/regular.jsonl"]);
    assert!(status.contains("?? history/regular.jsonl"), "{status}");
}

#[test]
fn sync_plaintext_history_uses_union_merge_across_two_clones() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let root_a = temp.path().join("aish-a");
    let root_b = temp.path().join("aish-b");
    seed_local_remote(&remote, &seed, &root_a);
    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), root_b.to_str().unwrap()],
    );
    run_test_git(&root_b, ["config", "user.name", "Aish Test"]);
    run_test_git(&root_b, ["config", "user.email", "aish@example.invalid"]);
    run_test_git(&root_b, ["config", "commit.gpgsign", "false"]);

    let mut state_a = sync_state_for_root(&root_a, &remote);
    fs::create_dir_all(root_a.join("history")).unwrap();
    fs::write(
        root_a.join("history/regular.jsonl"),
        "{\"command\":\"from-a\"}\n",
    )
    .unwrap();
    let mut out_a = Vec::new();
    run_manual_sync_push(&mut state_a, &mut out_a).unwrap();
    assert!(
        String::from_utf8(out_a)
            .unwrap()
            .contains("sync push completed")
    );

    let mut state_b = sync_state_for_root(&root_b, &remote);
    fs::create_dir_all(root_b.join("history")).unwrap();
    fs::write(
        root_b.join("history/regular.jsonl"),
        "{\"command\":\"from-b\"}\n",
    )
    .unwrap();
    let mut out_b = Vec::new();
    run_manual_sync_push(&mut state_b, &mut out_b).unwrap();

    let output_b = String::from_utf8(out_b).unwrap();
    assert!(!output_b.contains("sync aborted on conflict"), "{output_b}");
    assert!(output_b.contains("sync push completed"), "{output_b}");
    let pushed_history = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:history/regular.jsonl",
        ],
    );
    assert!(pushed_history.contains("from-a"), "{pushed_history}");
    assert!(pushed_history.contains("from-b"), "{pushed_history}");
}

#[test]
fn sync_resolve_union_command_keeps_both_sides_after_conflict() {
    let _guard = git_env_guard();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let other = temp.path().join("other");
    let root = temp.path().join("aish-home");

    run_test_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()]);
    fs::create_dir_all(&seed).unwrap();
    run_test_git(&seed, ["init"]);
    run_test_git(&seed, ["config", "user.name", "Aish Test"]);
    run_test_git(&seed, ["config", "user.email", "aish@example.invalid"]);
    fs::write(
        seed.join(".gitignore"),
        "seed-line\n# BEGIN AISH MANAGED\ncache/\nlogs/\nsecrets/\n*.tmp\n# END AISH MANAGED\n",
    )
    .unwrap();
    run_test_git(&seed, ["add", ".gitignore"]);
    run_test_git(&seed, ["commit", "-m", "seed"]);
    run_test_git(&seed, ["remote", "add", "origin", remote.to_str().unwrap()]);
    run_test_git(&seed, ["push", "-u", "origin", "HEAD"]);
    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), root.to_str().unwrap()],
    );
    run_test_git(
        temp.path(),
        ["clone", remote.to_str().unwrap(), other.to_str().unwrap()],
    );
    for repo in [&root, &other] {
        run_test_git(repo, ["config", "user.name", "Aish Test"]);
        run_test_git(repo, ["config", "user.email", "aish@example.invalid"]);
        run_test_git(repo, ["config", "commit.gpgsign", "false"]);
    }

    fs::write(
        root.join(".gitignore"),
        "aish-local-line\n# BEGIN AISH MANAGED\ncache/\nlogs/\nsecrets/\n*.tmp\n# END AISH MANAGED\n",
    )
    .unwrap();
    run_test_git(&root, ["add", ".gitignore"]);
    run_test_git(&root, ["commit", "-m", "local-change"]);

    fs::write(
        other.join(".gitignore"),
        "remote-line\n# BEGIN AISH MANAGED\ncache/\nlogs/\nsecrets/\n*.tmp\n# END AISH MANAGED\n",
    )
    .unwrap();
    run_test_git(&other, ["add", ".gitignore"]);
    run_test_git(&other, ["commit", "-m", "remote-change"]);
    run_test_git(&other, ["push"]);

    let mut state = sync_state_for_root(&root, &remote);
    let mut conflict_output = Vec::new();
    run_manual_sync_push(&mut state, &mut conflict_output).unwrap();
    let conflict_output = String::from_utf8(conflict_output).unwrap();
    assert!(
        conflict_output.contains("sync aborted on conflict"),
        "{conflict_output}"
    );
    assert!(conflict_output.contains("#sync resolve-union"));

    state.draft.insert_str("#sync resolve-union");
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut resolve_output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut resolve_output,
        Duration::from_secs(10),
    )
    .unwrap();

    let resolve_output = String::from_utf8(resolve_output).unwrap();
    assert!(
        resolve_output.contains("sync conflict union-resolved: .gitignore"),
        "{resolve_output}"
    );
    assert!(
        resolve_output.contains("sync push completed"),
        "{resolve_output}"
    );
    let pushed_gitignore = run_test_git_stdout(
        temp.path(),
        [
            "--git-dir",
            remote.to_str().unwrap(),
            "show",
            "HEAD:.gitignore",
        ],
    );
    assert!(pushed_gitignore.contains("aish-local-line"));
    assert!(pushed_gitignore.contains("remote-line"));
}

fn sync_state_for_root(root: &Path, remote: &Path) -> AppState {
    let config_path = root.join("config.toml");
    let events_path = root.join("logs/events.jsonl");
    let mut config = config::Config::default();
    config.storage.home = root.to_path_buf();
    config.sync.remote = remote.to_string_lossy().into_owned();
    config::save_config(&config_path, &config).unwrap();
    AppState {
        config_path: Some(config_path),
        events_path: Some(events_path),
        sync_config: config.sync,
        clock: fixed_clock,
        ..AppState::default()
    }
}

#[test]
fn foreground_shell_args_use_login_compatible_command_mode() {
    let _guard = git_env_guard();
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
    let _guard = git_env_guard();
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
        output.contains("startup sync due; running #sync now"),
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
    let _guard = git_env_guard();
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
    assert!(output.contains("startup sync enabled; running #sync now"));
    assert!(output.contains("sync remote is not configured"));
    assert_eq!(
        fs::read_to_string(root.join("cache/runtime/sync.last_attempt")).unwrap(),
        "42\n"
    );
}

#[test]
fn startup_sync_skips_not_due_schedule_without_running_git() {
    let _guard = git_env_guard();
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
    let _guard = git_env_guard();
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
