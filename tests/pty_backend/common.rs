use std::fs;
use std::time::{Duration, Instant};

use aish::pty::{PtyBackend, PtyCommandEvent};

use crate::support::{EnvVarGuard, command_available, field_value, pty_test_guard, stty_has_flag};

#[test]
fn pty_backend_runs_commands_and_preserves_shell_state() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let first_pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(first_pwd.exit_code, 0);
    assert_eq!(first_pwd.started_command.as_deref(), Some("pwd"));
    assert!(!first_pwd.output.trim().is_empty());
    assert_eq!(backend.initial_cwd(), Some(first_pwd.output.trim()));
    assert!(!first_pwd.output.contains("__AISH_STATUS__"));
    assert!(!first_pwd.output.contains("\x1b[?2004"));

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let second_pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(second_pwd.exit_code, 0);
    assert_eq!(second_pwd.output.trim(), "/tmp");
}

#[test]
fn bash_pty_backend_suppresses_readline_prompt_protocol_noise() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("printf 'bash-visible\\n'", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output, "bash-visible\n");
    assert!(!result.output.contains("\x1b[?2004"));
    assert!(!result.output.starts_with('\n'));
    assert!(!result.output.ends_with("\n\n"));
}

#[test]
fn bash_pty_backend_loads_user_bashrc_without_prompt_noise() {
    let _guard = pty_test_guard();
    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("from-aish-rc-path"),
        "#!/bin/sh\nprintf 'path-from-bashrc\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(bin_dir.join("from-aish-rc-path"))
        .status()
        .unwrap();
    fs::write(
        home.path().join(".bashrc"),
        format!(
            "[ -z \"$PS1\" ] && return\n\
             alias aish_alias_from_rc='printf alias-from-bashrc\\\\n'\n\
             aish_function_from_rc() {{ printf 'function-from-bashrc\\n'; }}\n\
             export AISH_RC_ENV=env-from-bashrc\n\
             export PATH=\"{}:$PATH\"\n\
             PS1='bashrc-prompt> '\n\
             PROMPT_COMMAND='export AISH_BASH_PROMPT_COMMAND=ran; printf prompt-command-noise\\\\n'\n",
            bin_dir.display()
        ),
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let alias = backend
        .run_command("aish_alias_from_rc", Duration::from_secs(5))
        .unwrap();
    let function = backend
        .run_command("aish_function_from_rc", Duration::from_secs(5))
        .unwrap();
    let env = backend
        .run_command("printf '%s\\n' \"$AISH_RC_ENV\"", Duration::from_secs(5))
        .unwrap();
    let path = backend
        .run_command("from-aish-rc-path", Duration::from_secs(5))
        .unwrap();
    let prompt_command = backend
        .run_command(
            "printf '%s\\n' \"$AISH_BASH_PROMPT_COMMAND\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(alias.exit_code, 0);
    assert_eq!(alias.output.trim(), "alias-from-bashrc");
    assert_eq!(function.exit_code, 0);
    assert_eq!(function.output.trim(), "function-from-bashrc");
    assert_eq!(env.exit_code, 0);
    assert_eq!(env.output.trim(), "env-from-bashrc");
    assert_eq!(path.exit_code, 0);
    assert_eq!(path.output.trim(), "path-from-bashrc");
    assert_eq!(prompt_command.exit_code, 0);
    assert_eq!(prompt_command.output.trim(), "ran");
    for result in [&alias, &function, &env, &path, &prompt_command] {
        assert!(!result.output.contains("bashrc-prompt"), "{result:?}");
        assert!(
            !result.output.contains("prompt-command-noise"),
            "{result:?}"
        );
        assert!(!result.output.contains("__AISH_STATUS__"), "{result:?}");
    }
}

#[test]
fn bash_pty_backend_suppresses_ps0_noise_from_bashrc() {
    let _guard = pty_test_guard();
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".bashrc"), "PS0='bash-ps0-noise\n'\n").unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("printf 'bash-after-ps0\\n'", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output.trim(), "bash-after-ps0");
    assert!(!result.output.contains("bash-ps0-noise"), "{result:?}");
}

#[test]
fn pty_backend_streams_output_before_command_completion() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let started = Instant::now();
    let mut first_chunk_at = None;
    let mut displayed = Vec::new();

    let result = backend
        .run_command_streaming_with_wait_callback(
            "printf 'stream-first\\n'; sleep 2; printf 'stream-second\\n'",
            Duration::from_secs(5),
            |_| Ok(false),
            |chunk| {
                if first_chunk_at.is_none() {
                    first_chunk_at = Some(started.elapsed());
                }
                displayed.extend_from_slice(chunk);
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(
        first_chunk_at.unwrap() < Duration::from_millis(1500),
        "first streamed chunk arrived after command completion: {first_chunk_at:?}"
    );
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("stream-first"), "{displayed:?}");
    assert!(displayed.contains("stream-second"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_STATUS__"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_START__"), "{displayed:?}");
}

#[test]
fn pty_backend_command_events_include_output_poll_and_idle_ticks() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut output_events = 0;
    let mut poll_events = 0;
    let mut idle_events = 0;
    let mut displayed = Vec::new();

    let result = backend
        .run_command_with_event_callback(
            "printf 'event-first\\n'; sleep 1; printf 'event-second\\n'",
            Duration::from_secs(5),
            |_, event| {
                match event {
                    PtyCommandEvent::Output(chunk) => {
                        output_events += 1;
                        displayed.extend_from_slice(chunk);
                    }
                    PtyCommandEvent::PollInput => {
                        poll_events += 1;
                    }
                    PtyCommandEvent::Idle => {
                        idle_events += 1;
                    }
                }
                Ok(false)
            },
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(output_events > 0, "missing output events");
    assert!(poll_events > 0, "missing input poll events");
    assert!(idle_events > 0, "missing idle events");
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("event-first"), "{displayed:?}");
    assert!(displayed.contains("event-second"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_STATUS__"), "{displayed:?}");
}

#[test]
fn pty_backend_streams_partial_prompt_and_accepts_stdin_reply() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut displayed = Vec::new();
    let mut answered = false;

    let result = backend
        .run_command_with_event_callback(
            "printf 'confirm-remove? '; IFS= read -r answer; printf 'answer=%s\\n' \"$answer\"",
            Duration::from_secs(5),
            |backend, event| {
                if let PtyCommandEvent::Output(chunk) = event {
                    displayed.extend_from_slice(chunk);
                    let visible = String::from_utf8_lossy(&displayed);
                    if !answered && visible.contains("confirm-remove? ") {
                        answered = true;
                        backend.write_raw("no\r")?;
                    }
                }
                Ok(false)
            },
        )
        .unwrap();

    assert!(
        answered,
        "partial prompt was not streamed before stdin reply"
    );
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.started_command.as_deref(),
        Some("printf 'confirm-remove? '; IFS= read -r answer; printf 'answer=%s\\n' \"$answer\"")
    );
    assert!(
        result.output.contains("confirm-remove? answer=no"),
        "{result:?}"
    );
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("confirm-remove? "), "{displayed:?}");
    assert!(!displayed.contains("__AISH_STATUS__"), "{displayed:?}");
    assert!(!result.output.contains("__AISH_STATUS__"), "{result:?}");
}

#[test]
fn pty_backend_passthrough_enables_child_echo_and_restores_backend_echo_off() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut displayed = Vec::new();

    let result = backend
        .run_command_passthrough_with_event_callback("stty -a", |_, event| {
            if let PtyCommandEvent::Output(chunk) = event {
                displayed.extend_from_slice(chunk);
            }
            Ok(false)
        })
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(stty_has_flag(&result.output, "echo"), "{result:?}");
    assert!(!result.output.contains("__aish_passthrough_status"));
    assert!(!result.output.contains("stty echo"));

    let restored = backend
        .run_command("stty -a", Duration::from_secs(5))
        .unwrap();
    assert_eq!(restored.exit_code, 0);
    assert!(stty_has_flag(&restored.output, "-echo"), "{restored:?}");

    let displayed = String::from_utf8(displayed).unwrap();
    assert!(stty_has_flag(&displayed, "echo"), "{displayed:?}");
    assert!(!displayed.contains("__aish_passthrough_status"));
}

#[test]
fn pty_backend_external_child_owns_foreground_process_group() {
    let _guard = pty_test_guard();
    if !command_available("python3") {
        return;
    }
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command(
            "python3 -c 'import os; print(f\"isatty={int(os.isatty(0))} pgrp={os.getpgrp()} tpgid={os.tcgetpgrp(0)}\")'",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let line = result.output.trim();
    assert!(line.starts_with("isatty=1 "), "{result:?}");
    let pgrp = field_value(line, "pgrp=").unwrap();
    let tpgid = field_value(line, "tpgid=").unwrap();
    assert_eq!(pgrp, tpgid, "{result:?}");
}

#[test]
fn pty_backend_streaming_preserves_carriage_return_progress_for_display() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut displayed = Vec::new();

    let result = backend
        .run_command_streaming_with_wait_callback(
            "printf 'progress 1\\rprogress 2\\rfinal\\n'",
            Duration::from_secs(5),
            |_| Ok(false),
            |chunk| {
                displayed.extend_from_slice(chunk);
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(
        displayed.windows(2).any(|window| window == b"1\r")
            || displayed.windows(2).any(|window| window == b"2\r"),
        "displayed output did not preserve carriage returns: {displayed:?}"
    );
    let displayed_text = String::from_utf8(displayed).unwrap();
    assert!(displayed_text.contains("final"), "{displayed_text:?}");
    assert!(
        !displayed_text.contains("__AISH_STATUS__"),
        "{displayed_text:?}"
    );
    assert!(
        !displayed_text.contains("__AISH_START__"),
        "{displayed_text:?}"
    );
}

#[test]
fn pty_backend_resizes_visible_columns_for_child_commands() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    backend.resize(aish::pty::pty_size(132, 40)).unwrap();

    let size = backend
        .run_command("stty size", Duration::from_secs(5))
        .unwrap();

    assert_eq!(size.exit_code, 0);
    assert_eq!(size.output.trim(), "40 132");
}

#[test]
fn pty_backend_clear_output_does_not_end_with_newline() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("clear", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(
        result.output.contains("\x1b[2J") || result.output.contains("\x1b[J"),
        "{:?}",
        result.output
    );
    assert!(!result.output.ends_with('\n'), "{:?}", result.output);
}

#[test]
fn pty_backend_runs_multiline_commands_before_marker() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("echo paste-one\necho paste-two", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.output.contains("paste-one"), "{:?}", result.output);
    assert!(result.output.contains("paste-two"), "{:?}", result.output);
}

#[test]
fn pty_backend_streams_all_multiline_command_output() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
    let mut displayed = Vec::new();

    let result = backend
        .run_command_streaming_with_wait_callback(
            "printf 'multi-one\\n'\nprintf 'multi-two\\n'",
            Duration::from_secs(5),
            |_| Ok(false),
            |chunk| {
                displayed.extend_from_slice(chunk);
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.output.contains("multi-one"), "{:?}", result.output);
    assert!(result.output.contains("multi-two"), "{:?}", result.output);
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("multi-one"), "{displayed:?}");
    assert!(displayed.contains("multi-two"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_READY__"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_STATUS__"), "{displayed:?}");
}

#[test]
fn pty_backend_captures_failed_command_exit_status() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("false", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.exit_code, 1);
    assert!(result.output.trim().is_empty());
}

#[test]
fn pty_backend_wait_callback_can_interrupt_long_running_commands() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let started = Instant::now();
    let mut interrupted = false;
    let result = backend
        .run_command_with_wait_callback("sleep 30", Duration::from_secs(5), |backend| {
            if !interrupted {
                backend.write_raw("\x03")?;
                interrupted = true;
                return Ok(true);
            }
            Ok(false)
        })
        .unwrap();

    assert!(interrupted);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_ne!(result.exit_code, 0);

    let recovered = backend
        .run_command("printf 'after-interrupt\\n'", Duration::from_secs(5))
        .unwrap();
    assert_eq!(recovered.exit_code, 0);
    assert_eq!(recovered.output.trim(), "after-interrupt");
}

#[test]
fn pty_backend_reports_finish_status_and_cwd() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command("cd /tmp && false", Duration::from_secs(5))
        .unwrap();

    assert_eq!(result.started_command.as_deref(), Some("cd /tmp && false"));
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.cwd.as_deref(), Some("/tmp"));
    assert!(result.output.trim().is_empty());
}

#[test]
fn pty_backend_does_not_confuse_user_output_with_prompt_marker() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command(
            "printf '%s %s %s\\n' before __AISH_STATUS__ after",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output.trim(), "before __AISH_STATUS__ after");
}

#[test]
fn pty_backend_ignores_bogus_writes_to_control_fd() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command(
            "printf 'before-bogus-control\\n'; { printf '__AISH_READY__\\t0\\t/tmp\\n' >&64; } 2>/dev/null || true; sleep 1; printf 'after-bogus-control\\n'",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.output.contains("before-bogus-control"), "{result:?}");
    assert!(result.output.contains("after-bogus-control"), "{result:?}");
}

#[test]
fn pty_backend_does_not_export_control_fd_to_user_commands() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let result = backend
        .run_command(
            "printf 'control-fd=%s\\n' \"${AISH_CONTROL_FD-unset}\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output.trim(), "control-fd=unset");
}

#[test]
fn pty_backend_keeps_user_commands_but_not_aish_internal_markers_in_history() {
    let _guard = pty_test_guard();
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let user_command = "printf 'aish-history-target\\n'";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-history-target");

    let history = backend
        .run_command("history", Duration::from_secs(5))
        .unwrap();

    assert_eq!(history.exit_code, 0);
    assert!(history.output.contains(user_command));
    assert!(!history.output.contains("__aish_status=$?"));
    assert!(!history.output.contains("__AISH_STATUS__"));
    assert!(!history.output.contains("__AISH_READY__"));
}
