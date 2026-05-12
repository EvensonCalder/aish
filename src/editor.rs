use std::env;
use std::path::{Path, PathBuf};

use crate::config::EditorConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub argv: Vec<String>,
}

pub fn resolve_editor_command(config: &EditorConfig) -> Option<EditorCommand> {
    if !config.command.is_empty() {
        return Some(EditorCommand {
            argv: config.command.clone(),
        });
    }

    for var in ["VISUAL", "EDITOR"] {
        if let Ok(value) = env::var(var)
            && let Some(argv) = split_editor_command(&value)
        {
            return Some(EditorCommand { argv });
        }
    }

    ["nvim", "vim", "vi"]
        .into_iter()
        .find(|name| command_exists_on_path(name))
        .map(|name| EditorCommand {
            argv: vec![name.to_string()],
        })
}

fn split_editor_command(value: &str) -> Option<Vec<String>> {
    let argv: Vec<_> = value
        .split_whitespace()
        .filter(|part| !part.trim().is_empty())
        .map(str::to_string)
        .collect();
    (!argv.is_empty()).then_some(argv)
}

fn command_exists_on_path(name: &str) -> bool {
    let command = Path::new(name);
    if command.components().count() > 1 {
        return command.is_file();
    }

    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| candidate_path(dir, name))
        .any(|path| path.is_file())
}

fn candidate_path(dir: PathBuf, name: &str) -> PathBuf {
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};

    static EDITOR_ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn editor_env_guard() -> MutexGuard<'static, ()> {
        EDITOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn resolve_editor_prefers_config_command() {
        let config = EditorConfig {
            command: vec!["nvim".to_string(), "--clean".to_string()],
            execute_after_save: false,
        };

        let command = resolve_editor_command(&config).unwrap();

        assert_eq!(command.argv, ["nvim", "--clean"]);
    }

    #[test]
    fn resolve_editor_uses_visual_before_editor() {
        let _guard = editor_env_guard();
        let old_visual = env::var_os("VISUAL");
        let old_editor = env::var_os("EDITOR");
        unsafe {
            env::set_var("VISUAL", "code --wait");
            env::set_var("EDITOR", "vim");
        }

        let command = resolve_editor_command(&EditorConfig::default()).unwrap();

        restore_env("VISUAL", old_visual);
        restore_env("EDITOR", old_editor);
        assert_eq!(command.argv, ["code", "--wait"]);
    }

    #[test]
    fn resolve_editor_falls_back_to_path_candidates() {
        let _guard = editor_env_guard();
        let temp = tempfile::tempdir().unwrap();
        let fake_vi = temp.path().join("vi");
        fs::write(&fake_vi, "#!/bin/sh\n").unwrap();
        let old_visual = env::var_os("VISUAL");
        let old_editor = env::var_os("EDITOR");
        let old_path = env::var_os("PATH");
        unsafe {
            env::remove_var("VISUAL");
            env::remove_var("EDITOR");
            env::set_var("PATH", temp.path());
        }

        let command = resolve_editor_command(&EditorConfig::default()).unwrap();

        restore_env("VISUAL", old_visual);
        restore_env("EDITOR", old_editor);
        restore_env("PATH", old_path);
        assert_eq!(command.argv, ["vi"]);
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        unsafe {
            match value {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }
}
