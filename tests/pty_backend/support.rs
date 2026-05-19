use std::sync::{Mutex, MutexGuard};

pub(crate) use crate::env_support::EnvVarGuard;
pub(crate) use crate::shell_support::{command_available, find_shell, fish_backend_tests_enabled};

static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn pty_test_guard() -> MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn field_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(prefix))
}

pub(crate) fn stty_has_flag(output: &str, flag: &str) -> bool {
    output
        .split(|ch: char| ch.is_whitespace() || ch == ';')
        .any(|token| token == flag)
}
