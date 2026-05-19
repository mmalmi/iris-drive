use std::path::PathBuf;

pub fn default_config_dir() -> Option<PathBuf> {
    dirs_config().map(|p| p.join("hashdrive"))
}

fn dirs_config() -> Option<PathBuf> {
    dirs::config_dir()
}
