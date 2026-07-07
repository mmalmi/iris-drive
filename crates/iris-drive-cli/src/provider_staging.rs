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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("staged.json.tmp");
    std::fs::write(&tmp_path, serde_json::to_vec_pretty(staged)?)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("moving {} to {}", tmp_path.display(), path.display()))?;
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
