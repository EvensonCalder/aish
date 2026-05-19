use super::*;

#[test]
fn sync_lock_allows_single_holder_and_removes_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("runtime/sync.lock");

    let lock = SyncLock::acquire(&path)
        .unwrap()
        .expect("first lock acquired");
    assert_eq!(lock.path(), path.as_path());
    assert!(path.exists());
    assert!(SyncLock::acquire(&path).unwrap().is_none());

    drop(lock);

    assert!(!path.exists());
    assert!(SyncLock::acquire(&path).unwrap().is_some());
}

#[test]
fn sync_lock_creates_parent_directory() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested/locks/sync.lock");

    let _lock = SyncLock::acquire(&path).unwrap().expect("lock acquired");

    assert!(path.exists());
}

#[test]
fn managed_gitignore_preserves_user_content_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".gitignore");
    fs::write(&path, "user-file\n").unwrap();

    maintain_managed_gitignore(&path).unwrap();
    let first = fs::read_to_string(&path).unwrap();
    maintain_managed_gitignore(&path).unwrap();
    let second = fs::read_to_string(&path).unwrap();

    assert_eq!(first, second);
    assert!(first.contains("user-file\n"));
    assert!(first.contains(GITIGNORE_BEGIN));
    assert!(first.contains("cache/\n"));
    assert!(first.contains("logs/\n"));
    assert!(first.contains("secrets/\n"));
    assert!(first.contains("config.toml\n"));
    assert!(first.contains(GITIGNORE_END));
}

#[test]
fn managed_gitignore_replaces_existing_managed_section() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".gitignore");
    fs::write(
        &path,
        "before\n# BEGIN AISH MANAGED\nold\n# END AISH MANAGED\nafter\n",
    )
    .unwrap();

    maintain_managed_gitignore(&path).unwrap();
    let updated = fs::read_to_string(&path).unwrap();

    assert!(updated.contains("before\n"));
    assert!(updated.contains("after\n"));
    assert!(!updated.contains("old\n"));
    assert_eq!(updated.matches(GITIGNORE_BEGIN).count(), 1);
    assert_eq!(updated.matches(GITIGNORE_END).count(), 1);
}

#[test]
fn managed_gitattributes_preserves_user_content_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".gitattributes");
    fs::write(&path, "*.md text\n").unwrap();

    maintain_managed_gitattributes(&path).unwrap();
    let first = fs::read_to_string(&path).unwrap();
    maintain_managed_gitattributes(&path).unwrap();
    let second = fs::read_to_string(&path).unwrap();

    assert_eq!(first, second);
    assert!(first.contains("*.md text\n"));
    assert!(first.contains(GITATTRIBUTES_BEGIN));
    assert!(first.contains("history/*.jsonl merge=union\n"));
    assert!(first.contains("templates/*.jsonl merge=union\n"));
    assert!(first.contains(GITATTRIBUTES_END));
}

#[test]
fn sync_readme_is_created_once_and_preserves_user_edits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("SYNC.md");

    maintain_sync_readme(&path).unwrap();
    let first = fs::read_to_string(&path).unwrap();
    maintain_sync_readme(&path).unwrap();
    let second = fs::read_to_string(&path).unwrap();

    assert_eq!(first, second);
    assert!(first.contains("Aish Sync Repository"));
    assert!(first.contains("managed by Aish sync"));

    fs::write(&path, "custom sync readme\n").unwrap();
    maintain_sync_readme(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "custom sync readme\n");
}

#[test]
fn tracked_managed_files_warning_lists_managed_tracked_paths() {
    let warning = tracked_managed_files_warning([
        "README.md",
        "cache/model.json",
        "config.toml",
        "./logs/events.jsonl",
        "secrets/key.json.gpg",
        "notes.tmp",
        "cache/model.json",
    ])
    .expect("tracked managed paths are warned");

    assert_eq!(
        warning.paths,
        vec![
            "cache/model.json",
            "config.toml",
            "logs/events.jsonl",
            "notes.tmp",
            "secrets/key.json.gpg"
        ]
    );
    assert!(warning.message.contains("5 Aish-managed path(s)"));
    assert!(warning.message.contains("not running git rm --cached"));
}

#[test]
fn tracked_managed_files_warning_ignores_unmanaged_paths() {
    assert!(tracked_managed_files_warning(["README.md", "src/main.rs", "tmp/notes"]).is_none());
}

#[test]
fn log_sync_failure_records_error_event() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("logs/events.jsonl");

    log_sync_failure(&path, 7, SyncFailureKind::Failure, "git push exited 1").unwrap();

    let loaded = crate::log::load_events(&path).unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].t, 7);
    assert_eq!(loaded.items[0].level, EventLevel::Error);
    assert_eq!(loaded.items[0].msg, "sync failed: git push exited 1");
}

#[test]
fn log_sync_conflict_redacts_secret_like_detail() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("logs/events.jsonl");

    log_sync_failure(
        &path,
        8,
        SyncFailureKind::Conflict,
        "merge conflict near sk-secret-token",
    )
    .unwrap();

    let loaded = crate::log::load_events(&path).unwrap();
    assert_eq!(loaded.items[0].level, EventLevel::Error);
    assert_eq!(
        loaded.items[0].msg,
        "sync conflict: merge conflict near [redacted]"
    );
}

#[test]
fn startup_sync_decision_skips_when_not_configured() {
    let mut config = SyncConfig::default();
    assert_eq!(
        startup_sync_decision(&config, 100, None),
        StartupSyncDecision::Disabled
    );

    config.enabled = true;
    assert_eq!(
        startup_sync_decision(&config, 100, None),
        StartupSyncDecision::MissingRemote
    );

    config.remote = "git@example.test:aish.git".to_string();
    assert_eq!(
        startup_sync_decision(&config, 100, None),
        StartupSyncDecision::MissingSchedule
    );
}

#[test]
fn startup_sync_decision_handles_supported_schedules_conservatively() {
    let config = SyncConfig {
        enabled: true,
        remote: "git@example.test:aish.git".to_string(),
        schedule: "*/15 * * * *".to_string(),
        ai: false,
        history: false,
        templates: false,
        drafts: false,
        ..SyncConfig::default()
    };

    assert_eq!(
        startup_sync_decision(&config, 100, None),
        StartupSyncDecision::Due
    );
    assert_eq!(
        startup_sync_decision(&config, 1000, Some(200)),
        StartupSyncDecision::NotDue { next_due_at: 1100 }
    );
    assert_eq!(
        startup_sync_decision(&config, 1100, Some(200)),
        StartupSyncDecision::Due
    );
}

#[test]
fn startup_sync_decision_rejects_unsupported_cron_without_side_effects() {
    let config = SyncConfig {
        enabled: true,
        remote: "git@example.test:aish.git".to_string(),
        schedule: "5 4 * * mon".to_string(),
        ai: false,
        history: false,
        templates: false,
        drafts: false,
        ..SyncConfig::default()
    };

    assert_eq!(
        startup_sync_decision(&config, 100, Some(0)),
        StartupSyncDecision::UnsupportedSchedule("5 4 * * mon".to_string())
    );
}

#[test]
fn classify_git_sync_step_continues_on_success() {
    assert_eq!(
        classify_git_sync_step(true, "already up to date", ""),
        SyncStepOutcome::Continue
    );
}

#[test]
fn classify_git_sync_step_aborts_on_conflict_like_output() {
    let outcome = classify_git_sync_step(
        false,
        "CONFLICT (content): Merge conflict in history/regular.jsonl",
        "error: could not apply abc123",
    );

    assert_eq!(
            outcome,
            SyncStepOutcome::AbortConflict {
                detail: "CONFLICT (content): Merge conflict in history/regular.jsonl\nerror: could not apply abc123".to_string()
            }
        );
}

#[test]
fn classify_git_sync_step_aborts_on_non_conflict_failure() {
    assert_eq!(
        classify_git_sync_step(false, "", "fatal: unable to access remote"),
        SyncStepOutcome::AbortFailure {
            detail: "fatal: unable to access remote".to_string()
        }
    );
}

#[test]
fn managed_add_plan_syncs_user_content_categories_by_default() {
    let config = SyncConfig::default();

    assert_eq!(
        managed_add_plan(&config),
        ManagedAddPlan {
            paths: vec![
                ".gitattributes".to_string(),
                ".gitignore".to_string(),
                "SYNC.md".to_string(),
                "history/ai.jsonl".to_string(),
                "history/draft.jsonl".to_string(),
                "history/notes.jsonl".to_string(),
                "history/regular.jsonl".to_string(),
                "templates/templates.jsonl".to_string(),
            ]
        }
    );
}

#[test]
fn managed_add_plan_can_disable_all_content_categories() {
    let config = SyncConfig {
        ai: false,
        history: false,
        templates: false,
        drafts: false,
        ..SyncConfig::default()
    };

    assert_eq!(
        managed_add_plan(&config).paths,
        vec![".gitattributes", ".gitignore", "SYNC.md"]
    );
}

#[test]
fn managed_add_plan_uses_gpg_paths_when_encryption_is_enabled() {
    let config = SyncConfig {
        ai: true,
        history: true,
        templates: true,
        drafts: true,
        ..SyncConfig::default()
    };

    assert_eq!(
        managed_add_plan_with_encryption(&config, true).paths,
        vec![
            ".gitattributes",
            ".gitignore",
            "SYNC.md",
            "history/ai.jsonl.gpg",
            "history/draft.jsonl.gpg",
            "history/notes.jsonl.gpg",
            "history/regular.jsonl.gpg",
            "templates/templates.jsonl.gpg",
        ]
    );
}

#[test]
fn existing_managed_add_plan_skips_missing_enabled_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("history")).unwrap();
    fs::write(temp.path().join(".gitignore"), "").unwrap();
    fs::write(temp.path().join("history/regular.jsonl"), "{}").unwrap();
    let config = SyncConfig {
        ai: false,
        drafts: false,
        history: true,
        templates: true,
        ..SyncConfig::default()
    };

    let plan = existing_managed_add_plan(temp.path(), &config);

    assert_eq!(
        plan.paths,
        vec![
            ".gitattributes",
            ".gitignore",
            "SYNC.md",
            "history/regular.jsonl"
        ]
    );
    assert_eq!(
        plan.missing_paths,
        vec!["history/notes.jsonl", "templates/templates.jsonl"]
    );
}

#[test]
fn pull_merge_plan_uses_fixed_git_arguments() {
    assert_eq!(
        pull_merge_plan(),
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "pull".to_string(),
                "--no-rebase".to_string(),
                "--no-edit".to_string()
            ]
        }
    );
}

#[test]
fn pull_merge_allow_unrelated_plan_uses_fixed_git_arguments() {
    assert_eq!(
        pull_merge_allow_unrelated_plan(),
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "pull".to_string(),
                "--no-rebase".to_string(),
                "--no-edit".to_string(),
                "--allow-unrelated-histories".to_string()
            ]
        }
    );
}

#[test]
fn default_sync_commit_plan_uses_fixed_git_arguments() {
    assert_eq!(
        default_sync_commit_plan(),
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "commit".to_string(),
                "-m".to_string(),
                "sync aish data".to_string()
            ]
        }
    );
}

#[test]
fn commit_plan_sanitizes_message_without_shell_interpolation() {
    assert_eq!(
        commit_plan("\n  sync now && rm -rf /\nsecond line").unwrap(),
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "commit".to_string(),
                "-m".to_string(),
                "sync now && rm -rf /".to_string()
            ]
        }
    );
}

#[test]
fn commit_plan_rejects_empty_message() {
    assert_eq!(commit_plan("\n\t\n"), None);
}

#[test]
fn push_plan_uses_fixed_git_arguments() {
    assert_eq!(
        push_plan(),
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "push".to_string(),
                "-u".to_string(),
                "origin".to_string(),
                "HEAD".to_string()
            ]
        }
    );
}

#[test]
fn init_repo_plan_uses_fixed_git_arguments() {
    assert_eq!(
        init_repo_plan(" git@example.test:aish.git ").unwrap(),
        InitRepoPlan {
            commands: vec![
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec!["init".to_string()]
                },
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec![
                        "remote".to_string(),
                        "add".to_string(),
                        "origin".to_string(),
                        "git@example.test:aish.git".to_string()
                    ]
                }
            ]
        }
    );
}

#[test]
fn init_repo_plan_rejects_empty_or_control_character_remote() {
    assert_eq!(init_repo_plan(""), None);
    assert_eq!(
        init_repo_plan("git@example.test:aish.git\n--upload-pack=x"),
        None
    );
}

#[test]
fn conservative_sync_plan_orders_fixed_steps() {
    let config = SyncConfig::default();

    assert_eq!(
        conservative_sync_plan(&config),
        ConservativeSyncPlan {
            commands: vec![
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec![
                        "add".to_string(),
                        "--".to_string(),
                        ".gitattributes".to_string(),
                        ".gitignore".to_string(),
                        "SYNC.md".to_string(),
                        "history/ai.jsonl".to_string(),
                        "history/draft.jsonl".to_string(),
                        "history/notes.jsonl".to_string(),
                        "history/regular.jsonl".to_string(),
                        "templates/templates.jsonl".to_string()
                    ]
                },
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec![
                        "commit".to_string(),
                        "-m".to_string(),
                        "sync aish data".to_string()
                    ]
                },
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec![
                        "pull".to_string(),
                        "--no-rebase".to_string(),
                        "--no-edit".to_string()
                    ]
                },
                GitCommandPlan {
                    program: "git".to_string(),
                    args: vec![
                        "push".to_string(),
                        "-u".to_string(),
                        "origin".to_string(),
                        "HEAD".to_string()
                    ]
                }
            ]
        }
    );
}

#[test]
fn conservative_sync_plan_adds_only_metadata_when_categories_are_disabled() {
    let config = SyncConfig {
        ai: false,
        history: false,
        templates: false,
        drafts: false,
        ..SyncConfig::default()
    };

    assert_eq!(
        conservative_sync_plan(&config).commands[0],
        GitCommandPlan {
            program: "git".to_string(),
            args: vec![
                "add".to_string(),
                "--".to_string(),
                ".gitattributes".to_string(),
                ".gitignore".to_string(),
                "SYNC.md".to_string()
            ]
        }
    );
}
