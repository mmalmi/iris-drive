use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use hashtree_core::Cid;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProviderStagedRoot {
    pub(crate) root_cid: String,
    pub(crate) tombstone_base_root_cid: Option<String>,
    pub(crate) tombstone_paths: BTreeSet<String>,
    pub(crate) updated_at: u64,
}

impl ProviderStagedRoot {
    pub(crate) fn root(&self) -> Result<Cid> {
        Cid::parse(&self.root_cid).context("parsing staged provider root cid")
    }

    pub(crate) fn tombstone_base_root(&self) -> Result<Option<Cid>> {
        self.tombstone_base_root_cid
            .as_deref()
            .map(Cid::parse)
            .transpose()
            .context("parsing staged provider tombstone base root cid")
    }
}

pub(crate) fn read_provider_staging(config_dir: &Path) -> Result<Option<ProviderStagedRoot>> {
    let path = iris_drive_core::paths::provider_root_staging_path_in(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading provider staging {}", path.display()));
        }
    };
    serde_json::from_str(&raw)
        .with_context(|| format!("parsing provider staging {}", path.display()))
}

pub(crate) fn write_provider_staging(config_dir: &Path, staged: &ProviderStagedRoot) -> Result<()> {
    let path = iris_drive_core::paths::provider_root_staging_path_in(config_dir);
    iris_drive_core::atomic_write(&path, &serde_json::to_vec_pretty(staged)?)
        .with_context(|| format!("writing provider staging {}", path.display()))?;
    Ok(())
}

pub(crate) fn clear_provider_staging(config_dir: &Path) -> Result<()> {
    let path = iris_drive_core::paths::provider_root_staging_path_in(config_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

pub(crate) fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn staging_write_does_not_follow_predictable_temporary_symlink() {
        let config = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external_file = outside.path().join("private.txt");
        std::fs::write(&external_file, b"keep private data").unwrap();
        let path = iris_drive_core::paths::provider_root_staging_path_in(config.path());
        let old_tmp_path = path.with_extension("staged.json.tmp");
        std::os::unix::fs::symlink(&external_file, old_tmp_path).unwrap();
        let staged = ProviderStagedRoot {
            root_cid: "root".into(),
            tombstone_base_root_cid: None,
            tombstone_paths: BTreeSet::new(),
            updated_at: 1,
        };

        write_provider_staging(config.path(), &staged).unwrap();

        assert_eq!(std::fs::read(&external_file).unwrap(), b"keep private data");
        assert_eq!(read_provider_staging(config.path()).unwrap(), Some(staged));
    }
}
