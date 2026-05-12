use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use aish::pty::PtyBackend;

static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

fn pty_test_guard() -> std::sync::MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn pty_backend_runs_commands_and_preserves_shell_state() {
    let _guard = pty_test_guard();
    let mut backend = PtyBackend::spawn("/bin/bash").unwrap();

    let first_pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(first_pwd.exit_code, 0);
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
#[ignore = "zsh PTY support needs shell-specific prompt/echo integration after bash v0.1"]
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
    assert_eq!(first.output.trim(), "zsh-ok");

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(pwd.exit_code, 0);
    assert_eq!(pwd.output.trim(), "/tmp");
}
