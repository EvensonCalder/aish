use super::*;

#[test]
fn prompt_config_commands_persist_apply_and_reset() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        current_cwd: Some(repo),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#prompt", "prompt.draft=\"{mode} \""),
        (
            "#prompt draft \"[{basename}] > \"",
            "prompt.draft=\"[{basename}] > \"",
        ),
        (
            "#prompt history \"hist {mode} \"",
            "prompt.history=\"hist {mode} \"",
        ),
        ("#prompt ai 'ai {mode} '", "prompt.ai=\"ai {mode} \""),
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
        assert!(state.draft.is_empty());
    }

    assert_eq!(state.render_prompt_line(), "[repo] > ");
    state.mode = Mode::History;
    assert_eq!(state.render_prompt_line(), "hist $ ");
    state.mode = Mode::Ai;
    assert_eq!(state.render_prompt_line(), "ai % ");
    state.mode = Mode::Draft;

    let loaded = config::load_config(&config_path).unwrap();
    assert_eq!(loaded.prompt.draft, "[{basename}] > ");
    assert_eq!(loaded.prompt.history, "hist {mode} ");
    assert_eq!(loaded.prompt.ai, "ai {mode} ");

    state.draft.insert_str("#prompt reset");
    let mut output = Vec::new();
    execute_draft(
        &mut state,
        &mut backend,
        &mut output,
        Duration::from_secs(5),
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("prompt.draft=\"{user}@{host} {cwd} > \""));
    assert_eq!(state.prompt_templates, PromptConfig::default().into());
    assert_eq!(
        config::load_config(&config_path).unwrap().prompt,
        PromptConfig::default()
    );
}

#[test]
fn prompt_config_commands_report_usage_and_missing_config() {
    for (line, expected) in [
        (
            "#prompt draft",
            "usage: #prompt [draft|history|ai <template>|reset]",
        ),
        ("#prompt draft \"\"", "prompt template must not be empty"),
        (
            "#prompt nope value",
            "usage: #prompt [draft|history|ai <template>|reset]",
        ),
        (
            "#prompt draft \"x> \"",
            "config path is not configured; #prompt not saved",
        ),
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
        assert!(state.draft.is_empty());
    }
}

#[test]
fn completion_config_commands_persist_and_reject_invalid_values() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    config::save_config(&config_path, &config::Config::default()).unwrap();
    let mut state = AppState {
        config_path: Some(config_path.clone()),
        ..AppState::default()
    };
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    for (line, expected) in [
        ("#completion off", "completion.mode=off"),
        ("#completion on", "completion.mode=auto"),
        ("#completion mode tab", "completion.mode=tab"),
        ("#completion mode auto", "completion.mode=auto"),
        ("#completion max 2", "completion.max_results=2"),
        ("#completion coalesce-ms 75", "completion.coalesce_ms=75"),
        ("#completion coalesce 50", "completion.coalesce_ms=50"),
        (
            "#completion display-delay-ms 180",
            "completion.display_delay_ms=180",
        ),
        (
            "#completion display-delay 120",
            "completion.display_delay_ms=120",
        ),
        ("#completion inline off", "completion.mode=tab"),
        ("#completion backend off", "completion.backend=false"),
        ("#completion shell on", "completion.backend=true"),
        ("#completion tab-accept word", "completion.tab_accept=word"),
        ("#completion fuzzy off", "completion.fuzzy=false"),
        ("#completion fuzzy on", "completion.fuzzy=true"),
        (
            "#completion match-threshold 80",
            "completion.match_threshold_percent=80",
        ),
        (
            "#completion typo-threshold 85",
            "completion.typo_threshold_percent=85",
        ),
        (
            "#completion max 0",
            "completion max results must be greater than 0",
        ),
        ("#completion max nope", "usage: #completion max <count>"),
        (
            "#completion coalesce-ms 1001",
            "completion coalesce ms must be between 0 and 1000",
        ),
        (
            "#completion coalesce-ms nope",
            "usage: #completion coalesce-ms <0-1000>",
        ),
        (
            "#completion display-delay-ms 1001",
            "completion display delay ms must be between 0 and 1000",
        ),
        (
            "#completion display-delay-ms nope",
            "usage: #completion display-delay-ms <0-1000>",
        ),
        (
            "#completion inline maybe",
            "usage: #completion inline on|off",
        ),
        (
            "#completion backend maybe",
            "usage: #completion backend on|off",
        ),
        (
            "#completion mode manual",
            "usage: #completion mode auto|tab|off",
        ),
        ("#completion fuzzy maybe", "usage: #completion fuzzy on|off"),
        (
            "#completion tab-accept line",
            "usage: #completion tab-accept full|word",
        ),
        (
            "#completion match-threshold 101",
            "completion match threshold must be between 0 and 100",
        ),
        (
            "#completion match-threshold nope",
            "usage: #completion match-threshold <0-100>",
        ),
        (
            "#completion typo-threshold 101",
            "completion typo threshold must be between 0 and 100",
        ),
        (
            "#completion typo-threshold nope",
            "usage: #completion typo-threshold <0-100>",
        ),
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
        assert!(state.draft.is_empty());
    }

    assert!(state.completion_config.enabled);
    assert_eq!(state.completion_config.mode(), CompletionMode::Tab);
    assert_eq!(state.completion_config.max_results, 2);
    assert_eq!(state.completion_config.coalesce_ms, 50);
    assert_eq!(state.completion_config.display_delay_ms, 120);
    assert!(!state.completion_config.inline);
    assert!(state.completion_config.fuzzy);
    assert_eq!(
        state.completion_config.tab_accept,
        CompletionTabAccept::Word
    );
    assert_eq!(state.completion_config.match_threshold_percent, 80);
    assert_eq!(state.completion_config.typo_threshold_percent, 85);
    let loaded = config::load_config(&config_path).unwrap().completion;
    assert!(loaded.enabled);
    assert_eq!(loaded.mode(), CompletionMode::Tab);
    assert_eq!(loaded.max_results, 2);
    assert_eq!(loaded.coalesce_ms, 50);
    assert_eq!(loaded.display_delay_ms, 120);
    assert!(!loaded.inline);
    assert!(loaded.fuzzy);
    assert_eq!(loaded.tab_accept, CompletionTabAccept::Word);
    assert_eq!(loaded.match_threshold_percent, 80);
    assert_eq!(loaded.typo_threshold_percent, 85);
}
