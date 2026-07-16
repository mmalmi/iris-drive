//! Measured block retrieval through the configured Hashtree resolver.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hashtree_core::{Cid, Hash, HashTree, HashTreeConfig, Store, StoreError, to_hex};

use crate::block_sync::collect_live_sync_hashes;
use crate::blossom_sync::DownloadReport;

use super::FipsSyncError;

pub(super) async fn download_tree_with_resolver<L, R>(
    local_store: Arc<L>,
    root: &Cid,
    resolver: Arc<R>,
) -> Result<DownloadReport, FipsSyncError>
where
    L: Store + Send + Sync + 'static,
    R: Store + ?Sized + 'static,
{
    let writeback = Arc::new(MeasuredResolverStore::new(local_store, resolver));
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

struct MeasuredResolverStore<L: Store + Send + Sync + 'static, R: Store + ?Sized + 'static> {
    local: Arc<L>,
    resolver: Arc<R>,
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L, R> MeasuredResolverStore<L, R>
where
    L: Store + Send + Sync + 'static,
    R: Store + ?Sized + 'static,
{
    fn new(local: Arc<L>, resolver: Arc<R>) -> Self {
        Self {
            local,
            resolver,
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
impl<L, R> Store for MeasuredResolverStore<L, R>
where
    L: Store + Send + Sync + 'static,
    R: Store + ?Sized + 'static,
{
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get(hash).await? {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(bytes));
        }

        if let Some(bytes) = self.resolver.get(hash).await? {
            self.fetched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        if self.local.has(hash).await? {
            return Ok(true);
        }
        self.resolver.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}
