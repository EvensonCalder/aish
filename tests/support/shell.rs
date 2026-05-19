use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn find_shell(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|candidate| Path::new(candidate).exists())
}

pub(crate) fn fish_backend_tests_enabled() -> bool {
    env::var_os("AISH_TEST_FISH").is_some()
}
