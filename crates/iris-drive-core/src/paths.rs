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

#[must_use] 
pub fn key_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("key")
}

#[must_use]
pub fn config_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("config.toml")
}

/// Owner signing key. Only present on devices with owner authority
/// (create / restore flows). Linked devices never have this file.
#[must_use]
pub fn owner_key_path_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("owner_key")
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
        assert_eq!(config_path_in(base), PathBuf::from("/tmp/x/config.toml"));
        assert_eq!(
            owner_key_path_in(base),
            PathBuf::from("/tmp/x/owner_key")
        );
    }
}
