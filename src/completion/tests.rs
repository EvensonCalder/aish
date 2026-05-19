use super::matching::CompletionMatcher;
use super::*;
use crate::config::CompletionTabAccept;
use crate::display_width::display_width;

mod accept;
mod first_token;
mod indexed_structural;
mod matching_typos;
mod non_first_token;
mod path;
mod private_commands;
mod render;
mod shell_words;
mod templates_history;
mod token;

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
