use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::EditorConfig;
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedEditorSession {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRunResult {
    pub exit_code: Option<i32>,
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

pub fn prepare_editor_file(root: &Path, initial_text: &str) -> Result<PreparedEditorSession> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create editor temp directory {}", root.display()))?;

    for attempt in 0..100_u32 {
        let path = root.join(format!("aish-edit-{}-{attempt}.sh", unique_editor_id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(initial_text.as_bytes()).with_context(|| {
                    format!("failed to write editor temp file {}", path.display())
                })?;
                return Ok(PreparedEditorSession { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create editor temp file {}", path.display())
                });
            }
        }
    }

    bail!(
        "failed to allocate unique editor temp file in {}",
        root.display()
    )
}

pub fn run_editor_command(
    command: &EditorCommand,
    session: &PreparedEditorSession,
) -> Result<EditorRunResult> {
    let Some(program) = command.argv.first() else {
        bail!("editor command is empty");
    };

    let status = Command::new(program)
        .args(&command.argv[1..])
        .arg(&session.path)
        .status()
        .with_context(|| format!("failed to run editor command `{program}`"))?;

    Ok(EditorRunResult {
        exit_code: status.code(),
    })
}

pub fn read_editor_file(session: &PreparedEditorSession) -> Result<String> {
    fs::read_to_string(&session.path)
        .with_context(|| format!("failed to read editor temp file {}", session.path.display()))
}

fn unique_editor_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", process::id())
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

    #[test]
    fn prepare_editor_file_writes_initial_text_to_secure_temp_file() {
        let temp = tempfile::tempdir().unwrap();

        let session = prepare_editor_file(temp.path(), "git status\n# raw editor content").unwrap();

        assert!(session.path.starts_with(temp.path()));
        assert_eq!(
            fs::read_to_string(&session.path).unwrap(),
            "git status\n# raw editor content"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&session.path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn run_editor_command_appends_session_path_and_waits() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        let output = temp.path().join("editor-output.txt");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s|%s|%s' \"$1\" \"$2\" \"$3\" > '{}'\n",
                output.display()
            ),
        )
        .unwrap();
        make_executable(&script);
        let session = prepare_editor_file(temp.path(), "draft").unwrap();
        let command = EditorCommand {
            argv: vec![
                script.display().to_string(),
                "--flag".to_string(),
                "value".to_string(),
            ],
        };

        let result = run_editor_command(&command, &session).unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            fs::read_to_string(output).unwrap(),
            format!("--flag|value|{}", session.path.display())
        );
        assert_eq!(fs::read_to_string(session.path).unwrap(), "draft");
    }

    #[test]
    fn run_editor_command_returns_nonzero_status_without_reading_file() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        fs::write(&script, "#!/bin/sh\nprintf changed > \"$1\"\nexit 7\n").unwrap();
        make_executable(&script);
        let session = prepare_editor_file(temp.path(), "draft").unwrap();
        let command = EditorCommand {
            argv: vec![script.display().to_string()],
        };

        let result = run_editor_command(&command, &session).unwrap();

        assert_eq!(result.exit_code, Some(7));
        assert_eq!(fs::read_to_string(session.path).unwrap(), "changed");
    }

    #[test]
    fn run_editor_command_rejects_empty_argv() {
        let temp = tempfile::tempdir().unwrap();
        let session = prepare_editor_file(temp.path(), "draft").unwrap();
        let command = EditorCommand { argv: Vec::new() };

        let error = run_editor_command(&command, &session).unwrap_err();

        assert!(error.to_string().contains("editor command is empty"));
    }

    #[test]
    fn read_editor_file_returns_saved_content() {
        let temp = tempfile::tempdir().unwrap();
        let session = prepare_editor_file(temp.path(), "initial").unwrap();
        fs::write(&session.path, "echo one\n# raw shell content").unwrap();

        let content = read_editor_file(&session).unwrap();

        assert_eq!(content, "echo one\n# raw shell content");
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        unsafe {
            match value {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
}
