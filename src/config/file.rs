use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Config, DirectoryLayout, StorageConfig, normalize_config, write_private_file};

pub fn init_default_layout(root: impl Into<PathBuf>) -> Result<(DirectoryLayout, Config)> {
    let layout = DirectoryLayout::new(root);
    layout.create_dirs()?;
    let config = load_or_create_config(&layout.config, &layout.root)?;
    Ok((layout, config))
}

pub fn load_or_create_config(path: &Path, root: &Path) -> Result<Config> {
    if path.exists() {
        return load_config(path);
    }

    let config = Config {
        storage: StorageConfig {
            home: root.to_path_buf(),
        },
        ..Config::default()
    };
    save_config(path, &config)?;
    Ok(config)
}

pub fn load_config(path: &Path) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let mut config: Config =
        toml::from_str(&raw).with_context(|| format!("invalid config {}", path.display()))?;
    normalize_config(&mut config);
    Ok(config)
}

pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let raw = toml::to_string_pretty(config).context("failed to serialize config")?;
    write_private_file(path, raw.as_bytes())
        .with_context(|| format!("failed to write config {}", path.display()))?;
    Ok(())
}
