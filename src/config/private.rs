use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

pub fn create_private_dir_all(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;
    set_private_dir_permissions(dir)
}

#[cfg(unix)]
pub fn set_private_dir_permissions(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).with_context(|| {
        format!(
            "failed to set private directory permissions {}",
            dir.display()
        )
    })
}

#[cfg(not(unix))]
pub fn set_private_dir_permissions(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set private file permissions {}", path.display()))
}

#[cfg(unix)]
pub fn set_private_file_handle_permissions(file: &File, path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| {
            format!(
                "failed to set private file permissions for {}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
pub fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub fn set_private_file_handle_permissions(_file: &File, _path: &Path) -> Result<()> {
    Ok(())
}

pub fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)?;
    }
    write_private_file_inner(path, bytes)
}

#[cfg(unix)]
fn write_private_file_inner(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to write private file {}", path.display()))?;
    set_private_file_handle_permissions(&file, path)?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write private file {}", path.display()))
}

#[cfg(not(unix))]
fn write_private_file_inner(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)
        .with_context(|| format!("failed to write private file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn write_private_file_refuses_symlink_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        fs::write(&target, "original").unwrap();
        symlink(&target, &link).unwrap();

        let result = write_private_file(&link, b"secret");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    }
}
