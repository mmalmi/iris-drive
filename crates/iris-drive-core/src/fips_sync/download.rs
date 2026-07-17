//! Measured block retrieval through Drive's configured Hashtree blob router.

use std::collections::HashSet;
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
    classified: Mutex<HashSet<Hash>>,
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
            classified: Mutex::new(HashSet::new()),
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

    fn record_found(&self, hash: &Hash, was_local: bool) {
        let first_observation = self
            .classified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(*hash);
        if !first_observation {
            return;
        }
        if was_local {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.fetched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
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
        let was_local = self.local.has(hash).await?;
        if let Some(bytes) = self.router.get(hash, Some(&self.preferred)).await? {
            self.record_found(hash, was_local);
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
        let was_local = self.local.has(hash).await?;
        let found = self
            .router
            .get(hash, Some(&self.preferred))
            .await?
            .is_some();
        if found {
            self.record_found(hash, was_local);
        }
        Ok(found)
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{MemoryStore, StoreBlobRoute, sha256};
    use hashtree_network::{BlobRouteEntry, BlobRouterConfig};

    #[tokio::test]
    async fn route_probe_cache_fill_is_reported_as_fetched_once() {
        let bytes = b"route probe cache fill".to_vec();
        let hash = sha256(&bytes);
        let source = Arc::new(MemoryStore::new());
        source.put(hash, bytes.clone()).await.unwrap();
        let local = Arc::new(MemoryStore::new());
        let cache: Arc<dyn Store> = local.clone();
        let router = Arc::new(
            BlobRouter::new(
                vec![
                    BlobRouteEntry::new(
                        LOCAL_ROUTE_ID,
                        Arc::new(StoreBlobRoute::new(local.clone())),
                    ),
                    BlobRouteEntry::new("test.source", Arc::new(StoreBlobRoute::new(source))),
                ],
                Some(cache),
                BlobRouterConfig::default(),
            )
            .unwrap(),
        );
        let measured = MeasuredRouterStore::new(local, router);

        assert!(measured.has(&hash).await.unwrap());
        assert_eq!(measured.get(&hash).await.unwrap(), Some(bytes));
        assert_eq!(measured.fetched(), 1);
        assert_eq!(measured.already_local(), 0);
    }
}
