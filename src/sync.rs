use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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
}
