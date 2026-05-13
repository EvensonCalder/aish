use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::log::{DEFAULT_MAX_EVENTS, EventLevel, append_event};

const GITIGNORE_BEGIN: &str = "# BEGIN AISH MANAGED";
const GITIGNORE_END: &str = "# END AISH MANAGED";
const MANAGED_GITIGNORE_LINES: &[&str] = &["cache/", "logs/", "secrets/", "*.tmp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedManagedFilesWarning {
    pub paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFailureKind {
    Conflict,
    Failure,
}

#[derive(Debug)]
pub struct SyncLock {
    path: PathBuf,
    held: bool,
}

impl SyncLock {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create sync lock directory {}", parent.display())
            })?;
        }

        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => {
                write_lock_metadata(file)?;
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    held: true,
                }))
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(err) => {
                Err(err).with_context(|| format!("failed to create sync lock {}", path.display()))
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        if self.held {
            let _ = fs::remove_file(&self.path);
            self.held = false;
        }
    }
}

fn write_lock_metadata(mut file: File) -> Result<()> {
    use std::io::Write;

    writeln!(file, "pid={}", std::process::id()).context("failed to write sync lock metadata")
}

pub fn maintain_managed_gitignore(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let existing = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read gitignore {}", path.display()))?
        }
    };
    let next = replace_managed_gitignore_section(&existing);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create gitignore directory {}", parent.display())
        })?;
    }
    fs::write(path, next).with_context(|| format!("failed to write gitignore {}", path.display()))
}

fn replace_managed_gitignore_section(existing: &str) -> String {
    let managed = managed_gitignore_section();
    let lines: Vec<&str> = existing.lines().collect();
    let start = lines.iter().position(|line| line.trim() == GITIGNORE_BEGIN);
    let end = lines.iter().position(|line| line.trim() == GITIGNORE_END);

    if let (Some(start), Some(end)) = (start, end)
        && start <= end
    {
        let mut output = String::new();
        for line in &lines[..start] {
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(&managed);
        for line in &lines[end + 1..] {
            output.push_str(line);
            output.push('\n');
        }
        return output;
    }

    let mut output = existing.trim_end_matches('\n').to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(&managed);
    output
}

fn managed_gitignore_section() -> String {
    let mut output = String::new();
    output.push_str(GITIGNORE_BEGIN);
    output.push('\n');
    for line in MANAGED_GITIGNORE_LINES {
        output.push_str(line);
        output.push('\n');
    }
    output.push_str(GITIGNORE_END);
    output.push('\n');
    output
}

pub fn tracked_managed_files_warning<I, S>(tracked_paths: I) -> Option<TrackedManagedFilesWarning>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut paths: Vec<String> = tracked_paths
        .into_iter()
        .filter_map(|path| managed_tracked_path(path.as_ref()))
        .collect();
    paths.sort();
    paths.dedup();

    if paths.is_empty() {
        return None;
    }

    Some(TrackedManagedFilesWarning {
        message: format!(
            "warning: {} Aish-managed path(s) may already be tracked; not running git rm --cached automatically",
            paths.len()
        ),
        paths,
    })
}

fn managed_tracked_path(path: &str) -> Option<String> {
    let normalized = path.trim_start_matches("./");
    if normalized.starts_with("cache/")
        || normalized.starts_with("logs/")
        || normalized.starts_with("secrets/")
        || normalized.ends_with(".tmp")
    {
        Some(normalized.to_string())
    } else {
        None
    }
}

pub fn log_sync_failure(
    log_path: impl AsRef<Path>,
    t: i64,
    kind: SyncFailureKind,
    detail: &str,
) -> Result<()> {
    let label = match kind {
        SyncFailureKind::Conflict => "sync conflict",
        SyncFailureKind::Failure => "sync failed",
    };
    append_event(
        log_path.as_ref(),
        t,
        EventLevel::Error,
        &format!("{label}: {detail}"),
        DEFAULT_MAX_EVENTS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_lock_allows_single_holder_and_removes_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime/sync.lock");

        let lock = SyncLock::acquire(&path)
            .unwrap()
            .expect("first lock acquired");
        assert_eq!(lock.path(), path.as_path());
        assert!(path.exists());
        assert!(SyncLock::acquire(&path).unwrap().is_none());

        drop(lock);

        assert!(!path.exists());
        assert!(SyncLock::acquire(&path).unwrap().is_some());
    }

    #[test]
    fn sync_lock_creates_parent_directory() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/locks/sync.lock");

        let _lock = SyncLock::acquire(&path).unwrap().expect("lock acquired");

        assert!(path.exists());
    }

    #[test]
    fn managed_gitignore_preserves_user_content_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".gitignore");
        fs::write(&path, "user-file\n").unwrap();

        maintain_managed_gitignore(&path).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        maintain_managed_gitignore(&path).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("user-file\n"));
        assert!(first.contains(GITIGNORE_BEGIN));
        assert!(first.contains("cache/\n"));
        assert!(first.contains("logs/\n"));
        assert!(first.contains("secrets/\n"));
        assert!(first.contains(GITIGNORE_END));
    }

    #[test]
    fn managed_gitignore_replaces_existing_managed_section() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".gitignore");
        fs::write(
            &path,
            "before\n# BEGIN AISH MANAGED\nold\n# END AISH MANAGED\nafter\n",
        )
        .unwrap();

        maintain_managed_gitignore(&path).unwrap();
        let updated = fs::read_to_string(&path).unwrap();

        assert!(updated.contains("before\n"));
        assert!(updated.contains("after\n"));
        assert!(!updated.contains("old\n"));
        assert_eq!(updated.matches(GITIGNORE_BEGIN).count(), 1);
        assert_eq!(updated.matches(GITIGNORE_END).count(), 1);
    }

    #[test]
    fn tracked_managed_files_warning_lists_managed_tracked_paths() {
        let warning = tracked_managed_files_warning([
            "README.md",
            "cache/model.json",
            "./logs/events.jsonl",
            "secrets/key.json.gpg",
            "notes.tmp",
            "cache/model.json",
        ])
        .expect("tracked managed paths are warned");

        assert_eq!(
            warning.paths,
            vec![
                "cache/model.json",
                "logs/events.jsonl",
                "notes.tmp",
                "secrets/key.json.gpg"
            ]
        );
        assert!(warning.message.contains("4 Aish-managed path(s)"));
        assert!(warning.message.contains("not running git rm --cached"));
    }

    #[test]
    fn tracked_managed_files_warning_ignores_unmanaged_paths() {
        assert!(tracked_managed_files_warning(["README.md", "src/main.rs", "tmp/notes"]).is_none());
    }

    #[test]
    fn log_sync_failure_records_error_event() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("logs/events.jsonl");

        log_sync_failure(&path, 7, SyncFailureKind::Failure, "git push exited 1").unwrap();

        let loaded = crate::log::load_events(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].t, 7);
        assert_eq!(loaded.items[0].level, EventLevel::Error);
        assert_eq!(loaded.items[0].msg, "sync failed: git push exited 1");
    }

    #[test]
    fn log_sync_conflict_redacts_secret_like_detail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("logs/events.jsonl");

        log_sync_failure(
            &path,
            8,
            SyncFailureKind::Conflict,
            "merge conflict near sk-secret-token",
        )
        .unwrap();

        let loaded = crate::log::load_events(&path).unwrap();
        assert_eq!(loaded.items[0].level, EventLevel::Error);
        assert_eq!(
            loaded.items[0].msg,
            "sync conflict: merge conflict near [redacted]"
        );
    }
}
