//! Measured block retrieval through Drive's configured Hashtree blob router.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hashtree_core::{Cid, Hash, HashTree, HashTreeConfig, Store, StoreError, to_hex};
use hashtree_network::{BlobRouteIdentity, BlobRouter};

use crate::block_sync::collect_live_sync_hashes;
use crate::blossom_sync::DownloadReport;

use super::FipsSyncError;

use super::blob_runtime::LOCAL_ROUTE_ID;

pub(super) async fn download_tree_with_router<L>(
    local_store: Arc<L>,
    root: &Cid,
    router: Arc<BlobRouter>,
) -> Result<DownloadReport, FipsSyncError>
where
    L: Store + Send + Sync + 'static,
{
    let writeback = Arc::new(MeasuredRouterStore::new(local_store, router));
    let tree = HashTree::new(HashTreeConfig::new(writeback.clone()));
    let hashes = collect_live_sync_hashes(&tree, root, 4).await?;
    if writeback.missing() > 0 {
        let detail = writeback
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", writeback.missing()));
        return Err(FipsSyncError::MissingOnFips(detail));
    }

    Ok(DownloadReport {
        total_hashes: hashes.len(),
        fetched: writeback.fetched(),
        already_local: writeback.already_local(),
    })
}

struct MeasuredRouterStore<L: Store + Send + Sync + 'static> {
    local: Arc<L>,
    router: Arc<BlobRouter>,
    preferred: [BlobRouteIdentity; 1],
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L> MeasuredRouterStore<L>
where
    L: Store + Send + Sync + 'static,
{
    fn new(local: Arc<L>, router: Arc<BlobRouter>) -> Self {
        Self {
            local,
            router,
            preferred: [BlobRouteIdentity::from(LOCAL_ROUTE_ID)],
            fetched: std::sync::atomic::AtomicUsize::new(0),
            already_local: std::sync::atomic::AtomicUsize::new(0),
            missing: std::sync::atomic::AtomicUsize::new(0),
            first_missing: Mutex::new(None),
        }
    }

    fn fetched(&self) -> usize {
        self.fetched.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn already_local(&self) -> usize {
        self.already_local
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn missing(&self) -> usize {
        self.missing.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn first_missing(&self) -> Option<String> {
        self.first_missing
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

#[async_trait]
impl<L> Store for MeasuredRouterStore<L>
where
    L: Store + Send + Sync + 'static,
{
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let was_local = self.local.has(hash).await.unwrap_or(false);
        if let Some(bytes) = self.router.get(hash, Some(&self.preferred)).await? {
            if was_local {
                self.already_local
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            } else {
                self.fetched
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(Some(bytes))
        } else {
            let hex = to_hex(hash);
            if self
                .missing
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0
                && let Ok(mut first_missing) = self.first_missing.lock()
            {
                *first_missing = Some(hex);
            }
            Ok(None)
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self
            .router
            .get(hash, Some(&self.preferred))
            .await?
            .is_some())
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}
