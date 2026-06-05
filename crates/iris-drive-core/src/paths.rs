use std::path::PathBuf;

/// Resolve the platform config dir for iris-drive, honouring `IRIS_DRIVE_CONFIG_DIR`
/// as an override (mainly for tests).
#[must_use]
pub fn default_config_dir() -> Option<PathBuf> {
    if let Ok(override_dir) = std::env::var("IRIS_DRIVE_CONFIG_DIR") {
        return Some(PathBuf::from(override_dir));
    }
    dirs::config_dir().map(|p| p.join("iris-drive"))
}

/// Resolve the default mountpoint for the primary drive.
#[must_use]
pub fn default_mountpoint_in(config_dir: &std::path::Path) -> PathBuf {
    if config_dir.file_name().and_then(|s| s.to_str()) == Some("Config")
        && let Some(parent) = config_dir.parent()
    {
        return parent.join("Drive");
    }
    config_dir.join("Drive")
}

#[must_use]
pub fn key_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("key")
}

#[must_use]
pub fn recovery_phrase_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("recovery_phrase")
}

#[must_use]
pub fn config_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("config.toml")
}

#[must_use]
pub fn sync_cache_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("sync-cache.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `IRIS_DRIVE_CONFIG_DIR` override is exercised end-to-end by the
    // idrive CLI tests; we don't unit-test it here because mutating
    // process env in 2024-edition Rust requires `unsafe`, which is
    // forbidden workspace-wide.

    #[test]
    fn key_and_config_paths_are_inside_dir() {
        let base = std::path::Path::new("/tmp/x");
        assert_eq!(key_path_in(base), PathBuf::from("/tmp/x/key"));
        assert_eq!(
            recovery_phrase_path_in(base),
            PathBuf::from("/tmp/x/recovery_phrase")
        );
        assert_eq!(config_path_in(base), PathBuf::from("/tmp/x/config.toml"));
        assert_eq!(
            sync_cache_path_in(base),
            PathBuf::from("/tmp/x/sync-cache.json")
        );
    }

    #[test]
    fn default_mountpoint_uses_config_sibling_for_native_layout() {
        assert_eq!(
            default_mountpoint_in(std::path::Path::new("/tmp/IrisDrive/Config")),
            PathBuf::from("/tmp/IrisDrive/Drive")
        );
    }

    #[test]
    fn default_mountpoint_uses_child_for_cli_layout() {
        assert_eq!(
            default_mountpoint_in(std::path::Path::new("/tmp/iris-drive")),
            PathBuf::from("/tmp/iris-drive/Drive")
        );
    }
}
