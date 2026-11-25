//! Listing a provider while an applied root's blocks are on their way.

use std::sync::Arc;

use anyhow::{Context, Result};
use hashtree_core::{HashTree, HashTreeConfig, HashTreeError};

use crate::blossom_sync::{BlossomSyncError, WriteBackBlossomStore};
use crate::daemon::DaemonError;
use crate::indexer::IndexError;
use crate::paths::key_path_in;
use crate::projection::{PrimaryMergedRoot, PrimaryMergedView, ProjectionError};
use crate::{AppKey, Daemon};

/// The provider view of [`Daemon::provider_projection`], downloading the
/// blocks it reads that are not stored on this device yet.
///
/// Sync records a root announced over the relay before its blocks arrive (the
/// author uploads them to the file service first), and on a phone only the
/// app downloads them. Files, the share sheet and Android's documents provider
/// list in that window, often while the app is suspended. Listing then fetches
/// the folder blocks it reads from the configured file service, as Drive does;
/// a file's own bytes download when it is opened. When the download fails,
/// the missing-chunk error stands with the reason.
pub async fn provider_projection(
    daemon: &Daemon,
) -> Result<(PrimaryMergedView, PrimaryMergedRoot, bool)> {
    let missing = match daemon.provider_projection().await {
        Err(error) if is_missing_chunk(&error) => error,
        result => return Ok(result?),
    };
    if let Err(error) = download_projection_blocks(daemon).await {
        return Err(anyhow::Error::new(missing).context(format!(
            "Drive folders are not all stored on this device and could not be downloaded \
             ({error:#})"
        )));
    }
    daemon
        .provider_projection()
        .await
        .context("building the provider view after downloading its folders")
}

fn is_missing_chunk(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::Projection(
            ProjectionError::Tree(HashTreeError::MissingChunk(_))
                | ProjectionError::Index(IndexError::Tree(HashTreeError::MissingChunk(_)))
        )
    )
}

/// Build the merged view over a store that fetches each block it lacks from
/// the file service and keeps it. Only what the view reads is fetched.
async fn download_projection_blocks(daemon: &Daemon) -> Result<()> {
    let config = daemon.config();
    if config.blossom_servers.is_empty() {
        return Err(BlossomSyncError::NoServers.into());
    }
    let device = AppKey::load(key_path_in(daemon.config_dir())).context("loading app key")?;
    let client = crate::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let store = Arc::new(WriteBackBlossomStore::new(
        daemon.tree().get_store().clone(),
        client,
    ));
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));
    let built = async {
        let view = crate::primary_merged_view(&tree, config).await?;
        crate::primary_merged_root_from_view(&tree, config, &view).await
    }
    .await;
    if store.missing() > 0 {
        let detail = store
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", store.missing()));
        return Err(BlossomSyncError::MissingOnBlossom(detail).into());
    }
    built.context("reading the Drive's folders")?;
    Ok(())
}

#[cfg(test)]
mod tests;
