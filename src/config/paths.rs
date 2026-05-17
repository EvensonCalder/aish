use std::path::PathBuf;

use anyhow::{Result, bail};

pub fn default_aish_dir() -> PathBuf {
    if let Ok(home) = std::env::var("AISH_HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aish")
}

pub fn runtime_aish_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("AISH_HOME") {
        let home = home.trim();
        if !home.is_empty() {
            let home = PathBuf::from(home);
            if !home.is_absolute() {
                bail!("AISH_HOME must be set to an absolute path");
            }
            return Ok(home);
        }
    }

    let Some(home) = std::env::var_os("HOME") else {
        bail!("AISH_HOME or HOME must be set to an absolute path");
    };
    let home = PathBuf::from(home);
    if home.as_os_str().is_empty() || !home.is_absolute() {
        bail!("AISH_HOME or HOME must be set to an absolute path");
    }
    Ok(home.join(".aish"))
}
