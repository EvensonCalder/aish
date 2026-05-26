use super::*;
use std::path::Path;

fn current_ready_marker() -> &'static str {
    ready_marker()
}

fn current_start_marker() -> &'static str {
    start_marker()
}

#[test]
fn resolves_configured_shell_before_environment() {
    assert_eq!(resolve_shell("/bin/custom-shell"), "/bin/custom-shell");
    assert_eq!(resolve_shell("  /bin/custom-shell  "), "/bin/custom-shell");
}

#[test]
fn shell_command_builder_inherits_current_directory() {
    let cwd = env::current_dir().unwrap();
    let launch = shell_launch("/bin/bash");
    let command = shell_command_builder(&launch);

    assert_eq!(
        command.get_current_dir().map(|cwd| cwd.as_os_str()),
        Some(cwd.as_os_str())
    );
    let control_env = command
        .get_envs()
        .find(|(name, _)| *name == std::ffi::OsStr::new("AISH_CONTROL_FD"))
        .map(|(_, value)| value);
    assert_eq!(control_env, Some(None));
}

#[test]
fn shell_launch_recognizes_login_shell_and_case_variants() {
    let login_zsh = shell_launch("-zsh");
    assert_eq!(login_zsh.integration, ShellIntegration::ZshHooks);

    let upper_fish = shell_launch("/usr/local/bin/FISH");
    assert_eq!(upper_fish.integration, ShellIntegration::FishEvents);
}

#[test]
fn spawned_backend_reports_resolved_shell_program() {
    let backend = PtyBackend::spawn("/bin/bash").unwrap();

    assert_eq!(backend.shell_program(), "/bin/bash");
}

#[test]
fn bash_launch_uses_clean_startup_flags() {
    let launch = shell_launch("/bin/bash");
    assert_eq!(launch.program, "/bin/bash");
    assert_eq!(launch.args, ["-i"]);
    assert_eq!(launch.integration, ShellIntegration::BashPromptCommand);
    assert!(launch.init_command.contains(READY_MARKER));
    assert!(launch.init_command.contains("HISTCONTROL=ignorespace"));
    assert!(launch.init_command.contains("unset AISH_CONTROL_FD"));
    assert!(launch.init_command.contains("__aish_disable_history"));
    assert!(launch.init_command.contains("unset HISTFILE"));
    assert!(launch.init_command.contains("HISTSIZE=0"));
    assert!(launch.init_command.contains("HISTIGNORE='*'"));
    assert!(launch.init_command.contains("set +o history"));
    assert!(!launch.init_command.contains("set -o history"));
    assert!(launch.init_command.contains("enable-bracketed-paste off"));
    assert!(launch.init_command.contains("__aish_run_prompt_command"));
    assert!(launch.init_command.contains("__aish_emit_ready"));
    assert!(
        launch
            .init_command
            .contains("PROMPT_COMMAND=__aish_emit_ready")
    );
    assert!(launch.init_command.contains("PS0=''"));
    assert!(launch.init_command.contains("trap - DEBUG"));
}

#[test]
fn non_bash_launch_does_not_receive_bash_only_flags() {
    let launch = shell_launch("/bin/zsh");
    assert_eq!(launch.program, "/bin/zsh");
    assert_eq!(launch.args, ["-i", "-o", "histignorespace"]);
    assert!(launch.init_command.contains("unsetopt zle"));
    assert!(launch.init_command.contains("unset AISH_CONTROL_FD"));
    assert!(launch.init_command.contains("add-zsh-hook"));
    assert!(launch.init_command.contains("__aish_disable_history"));
    assert!(launch.init_command.contains("unset HISTFILE"));
    assert!(launch.init_command.contains("HISTSIZE=0"));
    assert!(launch.init_command.contains("SAVEHIST=0"));
    assert!(launch.init_command.contains("fc -p /dev/null 0 0"));
    assert!(launch.init_command.contains("__aish_user_preexec_function"));
    assert!(launch.init_command.contains("__aish_user_precmd_function"));
    assert!(launch.init_command.contains("__aish_preexec"));
    assert!(launch.init_command.contains("__aish_precmd"));
}

#[test]
fn fish_launch_uses_event_functions_after_user_config() {
    let launch = shell_launch("/usr/bin/fish");

    assert_eq!(launch.program, "/usr/bin/fish");
    if let Some(features_index) = launch.args.iter().position(|arg| arg == "--features") {
        assert_eq!(
            launch.args.get(features_index + 1).map(String::as_str),
            Some("no-query-term,no-mark-prompt")
        );
    }
    if launch.args.iter().any(|arg| arg == "--private") {
        assert!(!launch.args.contains(&"--no-config".to_string()));
    }
    assert_eq!(launch.integration, ShellIntegration::FishEvents);
    assert!(launch.init_command.contains("set -e AISH_CONTROL_FD"));
    assert!(launch.init_command.contains("set -g fish_history \"\""));
    assert!(launch.init_command.contains("__aish_clear_fish_history"));
    assert!(launch.init_command.contains("history clear-session"));
    assert!(
        launch
            .init_command
            .contains("__aish_user_fish_preexec_functions")
    );
    assert!(
        launch
            .init_command
            .contains("__aish_user_fish_postexec_functions")
    );
    assert!(launch.init_command.contains("--on-event fish_preexec"));
    assert!(launch.init_command.contains("--on-event fish_postexec"));
    assert!(launch.init_command.contains("__aish_run_user_fish_preexec"));
    assert!(
        launch
            .init_command
            .contains("__aish_run_user_fish_postexec")
    );
    assert!(launch.init_command.contains("__aish_emit_ready"));
    assert!(launch.init_command.contains("function fish_prompt"));
    assert!(!launch.args.contains(&"--noprofile".to_string()));
    assert!(!launch.args.contains(&"--no-config".to_string()));
}

#[test]
fn shell_integration_init_commands_keep_pty_input_lines_short() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let launch = shell_launch(shell);
        let longest = launch
            .init_command
            .lines()
            .map(str::len)
            .max()
            .unwrap_or_default();
        assert!(
            longest <= 240,
            "{shell} init command has a PTY input line of {longest} bytes"
        );
    }
}

#[test]
fn parses_marker_and_hides_it_from_output() {
    let marker = "__AISH_STATUS__123__";
    let raw = format!("hello\r\n{marker}7\r\n");
    let (output, status, cwd, started) = parse_marker_output(&raw, marker).unwrap();
    assert_eq!(output, "hello");
    assert_eq!(status, 7);
    assert_eq!(cwd, None);
    assert_eq!(started, None);
}

#[test]
fn parses_marker_cwd_when_present() {
    let marker = "__AISH_STATUS__123__";
    let raw = format!("hello\r\n{marker}7\t/tmp/aish\r\n");
    let (output, status, cwd, _) = parse_marker_output(&raw, marker).unwrap();
    assert_eq!(output, "hello");
    assert_eq!(status, 7);
    assert_eq!(cwd.as_deref(), Some("/tmp/aish"));
}

#[test]
fn parser_ignores_old_fixed_marker_in_user_output() {
    let marker = "__AISH_STATUS__123__";
    let raw = format!("before __AISH_STATUS__ after\r\n{marker}0\r\n");
    let (output, status, _, _) = parse_marker_output(&raw, marker).unwrap();
    assert_eq!(output, "before __AISH_STATUS__ after");
    assert_eq!(status, 0);
}

#[test]
fn parser_normalizes_pty_newlines() {
    let marker = "__AISH_STATUS__123__";
    let raw = format!("one\r\ntwo\r\n{marker}0\r\n");
    let (output, status, _, _) = parse_marker_output(&raw, marker).unwrap();
    assert_eq!(output, "one\ntwo");
    assert_eq!(status, 0);
}

#[test]
fn parser_reads_ready_marker_cwd() {
    let ready = current_ready_marker();
    let raw = format!("noise\r\n{ready}\t/tmp/aish\r\n");
    assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
    assert_eq!(parse_ready_cwd(READY_MARKER), None);
    assert_eq!(
        parse_ready_cwd(&format!("noise\r\n{READY_MARKER}\t/tmp/legacy\r\n")),
        None
    );
}

#[test]
fn parser_reads_ready_marker_cwd_when_status_is_present() {
    let ready = current_ready_marker();
    let raw = format!("noise\r\n{ready}\t0\t/tmp/aish\r\n");
    assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
}

#[test]
fn parser_waits_for_complete_ready_marker_line() {
    let ready = current_ready_marker();
    let status_only = format!("{ready}\t0");
    let partial_cwd = format!("{ready}\t0\t/tmp/aish");

    assert_eq!(parse_ready_cwd(&status_only), None);
    assert_eq!(parse_ready_cwd(&partial_cwd), None);
    assert_eq!(
        parse_ready_cwd(&format!("{partial_cwd}\n")).as_deref(),
        Some("/tmp/aish")
    );
}

#[test]
fn parser_strips_terminal_controls_from_ready_marker_cwd() {
    let ready = current_ready_marker();
    let raw = format!("noise\r\n\x1b[K{ready}\t0\t/tmp/aish\x1b[K\r\n");
    assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
}

#[test]
fn parser_ignores_ready_marker_in_echoed_init_command() {
    let ready = current_ready_marker();
    let raw = format!("stty -echo; printf '\\n{ready}\\t%s\\n' \"$PWD\"\r\n{ready}\t/tmp/aish\r\n");
    assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
}

#[test]
fn parser_uses_real_marker_when_command_echo_contains_marker() {
    let marker = "__AISH_STATUS__123__";
    let raw = format!("__aish_status=$?; printf marker {marker}\r\nactual\r\n{marker}0\t/tmp\r\n");
    let (output, status, cwd, _) = parse_marker_output(&raw, marker).unwrap();
    assert_eq!(output, "actual");
    assert_eq!(status, 0);
    assert_eq!(cwd.as_deref(), Some("/tmp"));
}

#[test]
fn parser_reads_start_marker_for_marker_shells() {
    let marker = "__AISH_STATUS__123__";
    let start = current_start_marker();
    let raw = format!("{start}\tprintf hello\nhello\n{marker}0\t/tmp\n");
    let (output, status, cwd, started) = parse_marker_output(&raw, marker).unwrap();

    assert_eq!(output, "hello");
    assert_eq!(status, 0);
    assert_eq!(cwd.as_deref(), Some("/tmp"));
    assert_eq!(started.as_deref(), Some("printf hello"));
}

#[test]
fn start_marker_command_quotes_shell_text_and_normalizes_multiline_display() {
    let command = start_marker_command("printf 'a\\n'\necho done");

    assert!(command.starts_with(' '));
    assert!(command.contains(START_MARKER));
    assert!(command.contains("'printf '\\''a\\n'\\''\\necho done'"));
}

#[test]
fn clean_marker_echo_hides_ready_marker_lines() {
    let ready = current_ready_marker();
    let output = clean_marker_echo(
        &format!("echoed\n{ready}\t/tmp/aish\nvisible"),
        "__AISH_STATUS__1__",
    );

    assert_eq!(output, "echoed\nvisible");
}

#[test]
fn clean_marker_echo_removes_prompt_ready_separators() {
    let ready = current_ready_marker();
    let output = clean_marker_echo(
        &format!("one\n\n{ready}\t0\t/tmp/aish\ntwo\n\n{ready}\t0\t/tmp/aish\n"),
        "__AISH_STATUS__1__",
    );

    assert_eq!(output, "one\ntwo\n");
}

#[test]
fn clean_marker_echo_preserves_user_blank_before_prompt_separator() {
    let ready = current_ready_marker();
    let output = clean_marker_echo(
        &format!("one\n\n\n{ready}\t0\t/tmp/aish\n"),
        "__AISH_STATUS__1__",
    );

    assert_eq!(output, "one\n\n");
}

#[test]
fn output_filter_hides_marker_lines_and_their_separator() {
    let marker = "__AISH_STATUS__123__";
    let mut filter = PtyOutputFilter::marker(marker);
    let start = current_start_marker();

    let output =
        filter.push(format!("\r\n{start}\techo hi\r\nhi\r\n\r\n{marker}0\t/tmp\r\n").as_bytes());

    assert_eq!(String::from_utf8(output).unwrap(), "hi\r\n");
}

#[test]
fn output_filter_suppresses_prompt_noise_after_status_marker() {
    let marker = "__AISH_STATUS__123__";
    let mut filter = PtyOutputFilter::marker(marker);

    let output =
        filter.push(format!("hi\r\n{marker}0\t/tmp\r\nprompt-command-noise\r\n").as_bytes());

    assert_eq!(String::from_utf8(output).unwrap(), "hi\r\n");
}

#[test]
fn output_filter_preserves_carriage_return_progress() {
    let mut filter = PtyOutputFilter::marker("__AISH_STATUS__123__");

    let output = filter.push(b"Counting objects:  50%\rCounting objects: 100%\r\n");

    assert_eq!(
        output,
        b"Counting objects:  50%\rCounting objects: 100%\r\n"
    );
}

#[test]
fn fish_output_filter_streams_only_command_output_between_markers() {
    let mut filter = PtyOutputFilter::shell_events(true);
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "prompt repaint\r\n{start}\tprintf 'fish-ok\\n'\r\nfish-ok\r\n{ready}\t0\t/tmp/aish\r\nnext prompt\r\n"
    );

    let output = filter.push(raw.as_bytes());

    assert_eq!(String::from_utf8(output).unwrap(), "fish-ok\r\n");
}

#[test]
fn fish_output_filter_drops_cursor_repaint_duplicate_before_plain_output() {
    let mut filter = PtyOutputFilter::shell_events(true);
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\tcat c/i | grep beta\r\n\x1b[50Cbeta\r\nbeta\r\n{ready}\t0\t/tmp/aish\r\n"
    );

    let output = filter.push(raw.as_bytes());

    assert_eq!(String::from_utf8(output).unwrap(), "beta\r\n");
}

#[test]
fn fish_output_filter_flushes_held_repaint_output_on_idle() {
    let mut filter = PtyOutputFilter::shell_events(true);
    let start = current_start_marker();
    let raw = format!("{start}\tsleep 1\r\n\x1b[30m\x0fstreamed\r\n");

    assert!(filter.push(raw.as_bytes()).is_empty());

    let output = filter.flush_pending();

    assert_eq!(output, b"\x1b[30m\x0fstreamed\r\n");
}

#[test]
fn fish_output_filter_preserves_carriage_return_progress_inside_command() {
    let mut filter = PtyOutputFilter::shell_events(true);
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw =
        format!("{start}\tprintf progress\r\nprogress 1\rprogress 2\r\n{ready}\t0\t/tmp/aish\r\n");

    let output = filter.push(raw.as_bytes());

    assert_eq!(output, b"progress 1\rprogress 2\r\n");
}

#[test]
fn parse_ready_status_output_reads_status_cwd_and_filters_hook_lines() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!("{start}\techo hello\nhello\n{ready}\t7\t/tmp/aish\n");

    assert_eq!(
        parse_ready_status_output(&raw, false).unwrap(),
        HookCommandResult {
            output: "hello\n".to_string(),
            exit_code: 7,
            cwd: "/tmp/aish".to_string(),
            started_command: Some("echo hello".to_string()),
        }
    );
}

#[test]
fn parse_ready_status_output_preserves_user_output_line_breaks() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!("{start}\tprintf first\\nsecond\\n\nfirst\nsecond\n{ready}\t0\t/tmp/aish\n");

    let parsed = parse_ready_status_output(&raw, false).unwrap();

    assert_eq!(parsed.output, "first\nsecond\n");
}

#[test]
fn parse_ready_status_output_ignores_prompt_noise_around_command_markers() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "old prompt\n\
             {ready}\t0\n\
             {start}\tprintf hi\n\
             hi\n\
             {ready}\t0\t/tmp/aish\n\
             user precmd noise\n\
             prompt> \n"
    );

    let parsed = parse_ready_status_output(&raw, false).unwrap();

    assert_eq!(parsed.output, "hi\n");
    assert_eq!(parsed.cwd, "/tmp/aish");
    assert_eq!(parsed.started_command.as_deref(), Some("printf hi"));
}

#[test]
fn parse_ready_status_output_preserves_legacy_marker_text_from_user_output() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\tprintf marker\n\
             {READY_MARKER}\t0\t/tmp/fake\n\
             real\n\
             {ready}\t0\t/tmp/aish\n"
    );

    let parsed = parse_ready_status_output(&raw, false).unwrap();

    assert_eq!(
        parsed.output,
        format!("{READY_MARKER}\t0\t/tmp/fake\nreal\n")
    );
    assert_eq!(parsed.cwd, "/tmp/aish");
}

#[test]
fn parse_ready_status_output_can_filter_fish_repaint_sequences() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\tprintf 'fish-ok\\n'\n\
             printf \n\
             \x1b[50C\x1b[?2004l\x1b[?2031l\x1b[>4;0m\x1b>'fish-ok\\n'\n\
             \x1b[61C\x1b[18Dprintf 'fish-ok\\n'\n\
             \x1b[61C\n\
             \x1b[m\n\
             \x1b]0;printf 'fish-ok\\n' ~/aish\x07\x1b[m\n\
             fish-ok\n\
             \x1b[?25h\x1b[2m\u{23ce}\x1b[m\n\
             \u{23ce} \n\
             \x1b[K\x1b]0;~/aish\x07\x1b[m\x1b[?2004h\x1b[?2031h\x1b[>4;1m\x1b=\x1b[K\n\
             \x1b[43C\n\
             {ready}\t0\t/tmp/aish\n"
    );

    let parsed = parse_ready_status_output(&raw, true).unwrap();

    assert_eq!(parsed.output, "fish-ok\n");
    assert_eq!(
        parsed.started_command.as_deref(),
        Some("printf 'fish-ok\\n'")
    );
}

#[test]
fn fish_repaint_filter_preserves_plain_output_matching_command_suffix() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\tcat common/items.txt | grep beta\n\
             \x1b[50Ccommon/items.txt\n\
             \x1b[50C|\n\
             \x1b[50Cgrep\n\
             \x1b[50Cbeta\n\
             beta\n\
             {ready}\t0\t/tmp/aish\n"
    );

    let parsed = parse_ready_status_output(&raw, true).unwrap();

    assert_eq!(parsed.output, "beta\n");
}

#[test]
fn fish_repaint_filter_removes_semicolon_command_fragments() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\ttest -f c/i; and echo file-exists\n\
             c/i;\n\
             file-exists\n\
             {ready}\t0\t/tmp/aish\n"
    );

    let parsed = parse_ready_status_output(&raw, true).unwrap();

    assert_eq!(parsed.output, "file-exists\n");
}

#[test]
fn fish_repaint_filter_removes_variable_command_fragments() {
    let start = current_start_marker();
    let ready = current_ready_marker();
    let raw = format!(
        "{start}\tprintf '%s\\n' $AISH_FISH_RC_ENV\n\
             $AISH_FISH_RC_ENV\n\
             env-from-fish-config\n\
             {ready}\t0\t/tmp/aish\n"
    );

    let parsed = parse_ready_status_output(&raw, true).unwrap();

    assert_eq!(parsed.output, "env-from-fish-config\n");
}

#[test]
fn continuation_detection_uses_stable_lexical_state_not_shell_errors() {
    let quoted = input_needs_more_lines("/bin/bash", "echo \"").unwrap();
    assert!(quoted.needs_more);
    assert_eq!(quoted.prompt.as_deref(), Some("dquote> "));

    let single = input_needs_more_lines("/bin/zsh", "echo '").unwrap();
    assert!(single.needs_more);
    assert_eq!(single.prompt.as_deref(), Some("quote> "));

    let shell_block = input_needs_more_lines("/usr/bin/fish", "if true").unwrap();
    assert!(!shell_block.needs_more);
}

#[test]
fn line_continuation_detects_odd_trailing_backslashes() {
    assert!(
        input_needs_more_lines("/bin/bash", "echo aa \\")
            .unwrap()
            .needs_more
    );
    assert!(
        !input_needs_more_lines("/bin/bash", "echo aa \\\\")
            .unwrap()
            .needs_more
    );
    assert!(
        !input_needs_more_lines("/bin/bash", "echo aa")
            .unwrap()
            .needs_more
    );
    assert!(
        !input_needs_more_lines("/bin/bash", "echo 'aa\\'")
            .unwrap()
            .needs_more
    );
}

#[test]
fn bash_continuation_check_detects_incomplete_lexical_input_without_hanging() {
    let backend = PtyBackend::spawn("/bin/bash").unwrap();

    let continued = backend.input_needs_more_lines("echo aa \\").unwrap();
    assert!(continued.needs_more);
    assert_eq!(continued.prompt.as_deref(), Some("> "));

    let unclosed = backend.input_needs_more_lines("echo \"").unwrap();
    assert!(unclosed.needs_more);
    assert_eq!(unclosed.prompt.as_deref(), Some("dquote> "));

    let single = backend.input_needs_more_lines("echo '").unwrap();
    assert!(single.needs_more);
    assert_eq!(single.prompt.as_deref(), Some("quote> "));

    let complete = backend.input_needs_more_lines("echo \"ok\"").unwrap();
    assert!(!complete.needs_more);
    assert!(complete.prompt.is_none());
}

#[test]
fn zsh_continuation_check_detects_incomplete_lexical_input_without_hanging() {
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let unclosed = backend.input_needs_more_lines("echo \"").unwrap();
    assert!(unclosed.needs_more);
    assert_eq!(unclosed.prompt.as_deref(), Some("dquote> "));

    let complete = backend.input_needs_more_lines("echo \"ok\"").unwrap();
    assert!(!complete.needs_more);
}

#[test]
fn marker_status_requires_digits_and_line_end() {
    let marker = "__AISH_STATUS__123__";
    assert!(!marker_status_is_complete("hello", marker));
    assert!(!marker_status_is_complete(marker, marker));
    assert!(!marker_status_is_complete("__AISH_STATUS__123__", marker));
    assert!(!marker_status_is_complete(
        "__AISH_STATUS__123__x\n",
        marker
    ));
    assert!(marker_status_is_complete(
        "hello\r\n__AISH_STATUS__123__0\r\n",
        marker
    ));
}

#[test]
fn marker_command_preserves_status_without_external_sh() {
    let command = status_marker_command("__AISH_STATUS__test__");

    assert!(command.contains("__aish_preserve_status"));
    assert!(!command.contains("sh -c"));
}
