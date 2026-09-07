use std::path::Path;

use anyhow::{Context, Result};
use hashtree_fs::FsBlobStore;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use iris_drive_core::provider::{ProviderListEntry, provider_cache_destination};

#[derive(Default)]
pub(super) struct ProviderCacheReport {
    pub(super) file_count: usize,
    pub(super) directory_count: usize,
    pub(super) written: usize,
    pub(super) updated: usize,
    pub(super) unchanged: usize,
    pub(super) skipped: usize,
}

pub(super) async fn hydrate_provider_cache(
    provider: &HashTreeProviderFs<FsBlobStore>,
    entries: &[ProviderListEntry],
    target_dir: &Path,
) -> Result<ProviderCacheReport> {
    reject_cache_symlinks(target_dir, target_dir)?;
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("creating {}", target_dir.display()))?;
    let target_dir = std::fs::canonicalize(target_dir)
        .with_context(|| format!("resolving {}", target_dir.display()))?;
    let mut report = ProviderCacheReport::default();
    for entry in entries {
        let Some(destination) = provider_cache_destination(&target_dir, &entry.path) else {
            report.skipped += 1;
            continue;
        };
        reject_cache_symlinks(&target_dir, &destination)?;
        if entry.kind == "directory" {
            report.directory_count += 1;
            if destination.is_dir() {
                report.unchanged += 1;
                continue;
            }
            let existed = destination.exists();
            remove_provider_cache_destination(&destination)?;
            std::fs::create_dir_all(&destination)
                .with_context(|| format!("creating {}", destination.display()))?;
            if existed {
                report.updated += 1;
            } else {
                report.written += 1;
            }
            continue;
        }

        report.file_count += 1;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let bytes = provider.read(&entry.path, 0, entry.size).await?;
        // Fetching remote bytes can yield; check the destination again before
        // accessing local files. The selected directory must remain trusted.
        reject_cache_symlinks(&target_dir, &destination)?;
        if destination.is_file() {
            let existing = std::fs::read(&destination)
                .with_context(|| format!("reading {}", destination.display()))?;
            if existing == bytes {
                report.unchanged += 1;
                continue;
            }
            iris_drive_core::atomic_write(&destination, &bytes)
                .with_context(|| format!("writing {}", destination.display()))?;
            report.updated += 1;
            continue;
        }

        let existed = destination.exists();
        remove_provider_cache_destination(&destination)?;
        iris_drive_core::atomic_write(&destination, &bytes)
            .with_context(|| format!("writing {}", destination.display()))?;
        if existed {
            report.updated += 1;
        } else {
            report.written += 1;
        }
    }
    Ok(report)
}

fn reject_cache_symlinks(target_dir: &Path, destination: &Path) -> Result<()> {
    let relative = destination
        .strip_prefix(target_dir)
        .context("cache destination is outside the selected directory")?;
    let mut current = target_dir.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component);
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("cache path is a symbolic link: {}", current.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", current.display()));
            }
        }
    }
    Ok(())
}

fn remove_provider_cache_destination(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    } else if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}
