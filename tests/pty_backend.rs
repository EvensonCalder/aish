use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
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
    assert_eq!(first_pwd.started_command, None);
    assert!(!first_pwd.output.trim().is_empty());
    assert_eq!(backend.initial_cwd(), Some(first_pwd.output.trim()));
    assert!(!first_pwd.output.contains("__AISH_STATUS__"));

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let second_pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(second_pwd.exit_code, 0);
    assert_eq!(second_pwd.output.trim(), "/tmp");
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

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(pwd.exit_code, 0);
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
