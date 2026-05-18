use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{env, ffi::OsString};

use aish::pty::{PtyBackend, PtyCommandEvent};

static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

fn pty_test_guard() -> std::sync::MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvVarGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: impl Into<OsString>) -> Self {
        let previous = env::var_os(name);
        unsafe {
            env::set_var(name, value.into());
        }
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe {
                env::set_var(self.name, value);
            },
            None => unsafe {
                env::remove_var(self.name);
            },
        }
    }
}

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
fn fish_pty_backend_streams_output_before_command_completion_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend streaming test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let mut backend = PtyBackend::spawn(fish).unwrap();
    let started = Instant::now();
    let mut first_chunk_at = None;
    let mut displayed = Vec::new();

    let result = backend
        .run_command_streaming_with_wait_callback(
            "printf 'fish-stream-first\\n'; sleep 2; printf 'fish-stream-second\\n'",
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
        "first streamed fish chunk arrived after command completion: {first_chunk_at:?}"
    );
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("fish-stream-first"), "{displayed:?}");
    assert!(displayed.contains("fish-stream-second"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_READY__"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_START__"), "{displayed:?}");
    assert!(!displayed.contains('\u{23ce}'), "{displayed:?}");
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

#[test]
fn zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available() {
    let _guard = pty_test_guard();
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let mut backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let first = backend
        .run_command("printf 'zsh-ok\\n'", Duration::from_secs(5))
        .unwrap();
    assert_eq!(first.exit_code, 0);
    assert_eq!(first.started_command.as_deref(), Some("printf 'zsh-ok\\n'"));
    assert_eq!(first.output.trim(), "zsh-ok");
    assert_eq!(first.cwd.as_deref(), Some(backend.initial_cwd().unwrap()));

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(pwd.exit_code, 0);
    assert_eq!(pwd.started_command.as_deref(), Some("pwd"));
    assert_eq!(pwd.cwd.as_deref(), Some("/tmp"));
    assert_eq!(pwd.output.trim(), "/tmp");
}

#[test]
fn zsh_pty_backend_keeps_user_commands_but_not_aish_internal_markers_in_history() {
    let _guard = pty_test_guard();
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let user_command = "printf 'aish-zsh-history-target\\n'";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-zsh-history-target");

    let history = backend
        .run_command("fc -l 1", Duration::from_secs(5))
        .unwrap();

    assert_eq!(history.exit_code, 0);
    assert!(history.output.contains(user_command));
    assert!(!history.output.contains("__aish_status=$?"));
    assert!(!history.output.contains("__AISH_STATUS__"));
    assert!(!history.output.contains("__AISH_READY__"));
}

#[test]
fn zsh_pty_backend_loads_user_zshrc_when_available() {
    let _guard = pty_test_guard();
    let Some(zsh) = find_shell(&["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("from-aish-zshrc-path"),
        "#!/bin/sh\nprintf 'path-from-zshrc\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(bin_dir.join("from-aish-zshrc-path"))
        .status()
        .unwrap();
    fs::write(
        home.path().join(".zshrc"),
        format!(
            "alias aish_alias_from_zshrc='printf alias-from-zshrc\\\\n'\n\
             aish_function_from_zshrc() {{ printf 'function-from-zshrc\\n'; }}\n\
             function aish_user_preexec() {{ export AISH_ZSH_USER_PREEXEC=\"$1\"; }}\n\
             function aish_user_precmd() {{ export AISH_ZSH_USER_PRECMD=ran; }}\n\
             autoload -Uz add-zsh-hook\n\
             add-zsh-hook preexec aish_user_preexec\n\
             add-zsh-hook precmd aish_user_precmd\n\
             export AISH_ZSH_RC_ENV=env-from-zshrc\n\
             export PATH=\"{}:$PATH\"\n\
             PROMPT='zshrc-prompt> '\n",
            bin_dir.display()
        ),
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(zsh).unwrap();

    let alias = backend
        .run_command("aish_alias_from_zshrc", Duration::from_secs(5))
        .unwrap();
    let function = backend
        .run_command("aish_function_from_zshrc", Duration::from_secs(5))
        .unwrap();
    let env = backend
        .run_command(
            "printf '%s\\n' \"$AISH_ZSH_RC_ENV\"",
            Duration::from_secs(5),
        )
        .unwrap();
    let path = backend
        .run_command("from-aish-zshrc-path", Duration::from_secs(5))
        .unwrap();
    let hooks = backend
        .run_command(
            "printf '%s|%s\\n' \"$AISH_ZSH_USER_PRECMD\" \"$AISH_ZSH_USER_PREEXEC\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(alias.exit_code, 0);
    assert_eq!(alias.output.trim(), "alias-from-zshrc");
    assert_eq!(function.exit_code, 0);
    assert_eq!(function.output.trim(), "function-from-zshrc");
    assert_eq!(env.exit_code, 0);
    assert_eq!(env.output.trim(), "env-from-zshrc");
    assert_eq!(path.exit_code, 0);
    assert_eq!(path.output.trim(), "path-from-zshrc");
    assert_eq!(hooks.exit_code, 0);
    assert!(hooks.output.trim().starts_with("ran|printf "), "{hooks:?}");
    for result in [&alias, &function, &env, &path, &hooks] {
        assert!(!result.output.contains("zshrc-prompt"), "{result:?}");
        assert!(!result.output.contains("__AISH_READY__"), "{result:?}");
    }
}

#[test]
fn fish_pty_backend_runs_commands_and_reports_cwd_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let mut backend = PtyBackend::spawn(fish).unwrap();
    let first = backend
        .run_command("printf 'fish-ok\\n'", Duration::from_secs(5))
        .unwrap();

    assert_eq!(first.exit_code, 0);
    assert_eq!(
        first.started_command.as_deref(),
        Some("printf 'fish-ok\\n'")
    );
    assert_eq!(first.output.trim(), "fish-ok");
    assert_eq!(first.cwd.as_deref(), Some(backend.initial_cwd().unwrap()));

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(pwd.exit_code, 0);
    assert_eq!(pwd.started_command.as_deref(), Some("pwd"));
    assert_eq!(pwd.cwd.as_deref(), Some("/tmp"));
    assert_eq!(pwd.output.trim(), "/tmp");
}

#[test]
fn fish_pty_backend_preserves_output_that_matches_command_tokens_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let temp = tempfile::Builder::new()
        .prefix("aish-fish-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut backend = PtyBackend::spawn(fish).unwrap();
    let cd = backend
        .run_command(
            &format!("cd {}", temp.path().display()),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let setup = backend
        .run_command(
            "mkdir -p c; printf 'alpha\\nbeta\\n' > c/i",
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(setup.exit_code, 0);

    let grep = backend
        .run_command("cat c/i | grep beta", Duration::from_secs(5))
        .unwrap();
    assert_eq!(grep.exit_code, 0);
    assert_eq!(grep.output.trim(), "beta", "{grep:?}");

    let file_test = backend
        .run_command("test -f c/i; and echo file-exists", Duration::from_secs(5))
        .unwrap();
    assert_eq!(file_test.exit_code, 0);
    assert_eq!(file_test.output.trim(), "file-exists", "{file_test:?}");

    let echo = backend
        .run_command("echo after-failure", Duration::from_secs(5))
        .unwrap();
    assert_eq!(echo.exit_code, 0);
    assert_eq!(echo.output.trim(), "after-failure", "{echo:?}");
}

#[test]
fn fish_pty_backend_loads_user_config_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend config test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    let fish_config_dir = home.path().join(".config/fish");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&fish_config_dir).unwrap();
    fs::write(
        bin_dir.join("from-aish-fish-config-path"),
        "#!/bin/sh\nprintf 'path-from-fish-config\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(bin_dir.join("from-aish-fish-config-path"))
        .status()
        .unwrap();
    fs::write(
        fish_config_dir.join("config.fish"),
        format!(
            "function aish_function_from_fish_config\n\
                 printf 'function-from-fish-config\\n'\n\
             end\n\
             set -gx AISH_FISH_RC_ENV env-from-fish-config\n\
             set -gx PATH {} $PATH\n\
             function aish_user_fish_preexec --on-event fish_preexec\n\
                 set -gx AISH_FISH_USER_PREEXEC $argv[1]\n\
             end\n\
             function aish_user_fish_postexec --on-event fish_postexec\n\
                 set -gx AISH_FISH_USER_POSTEXEC ran\n\
             end\n\
             function fish_prompt\n\
                 printf 'fish-config-prompt> '\n\
             end\n",
            bin_dir.display()
        ),
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(fish).unwrap();

    let function = backend
        .run_command("aish_function_from_fish_config", Duration::from_secs(5))
        .unwrap();
    let env = backend
        .run_command("printf '%s\\n' $AISH_FISH_RC_ENV", Duration::from_secs(5))
        .unwrap();
    let path = backend
        .run_command("from-aish-fish-config-path", Duration::from_secs(5))
        .unwrap();
    let events = backend
        .run_command(
            "printf '%s|%s\\n' $AISH_FISH_USER_POSTEXEC $AISH_FISH_USER_PREEXEC",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(function.exit_code, 0);
    assert_eq!(function.output.trim(), "function-from-fish-config");
    assert_eq!(env.exit_code, 0);
    assert_eq!(env.output.trim(), "env-from-fish-config");
    assert_eq!(path.exit_code, 0);
    assert_eq!(path.output.trim(), "path-from-fish-config");
    assert_eq!(events.exit_code, 0);
    assert!(
        events.output.trim().starts_with("ran|printf "),
        "{events:?}"
    );
    for result in [&function, &env, &path, &events] {
        assert!(!result.output.contains("fish-config-prompt"), "{result:?}");
        assert!(!result.output.contains("__AISH_READY__"), "{result:?}");
    }
}

fn find_shell(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|candidate| Path::new(candidate).exists())
}

fn fish_backend_tests_enabled() -> bool {
    env::var_os("AISH_TEST_FISH").is_some()
}
