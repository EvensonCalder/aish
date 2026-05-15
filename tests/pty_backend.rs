use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{env, ffi::OsString};

use aish::pty::PtyBackend;

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
    assert!(result.output.contains("\x1b[2J"), "{:?}", result.output);
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

fn find_shell(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|candidate| Path::new(candidate).exists())
}

fn fish_backend_tests_enabled() -> bool {
    env::var_os("AISH_TEST_FISH").is_some()
}
