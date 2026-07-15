//! Block replication via Blossom HTTP blob servers.
//!
//! The Nostr `relay_sync` layer ships **metadata** (`AppKeys` + drive-root
//! references) between app installs. The actual htree blocks live in each
//! app install's `FsBlobStore` and have to move separately. This module
//! handles that movement over the Blossom protocol — a NIP-98-signed
//! HTTP blob store sized for content-addressed data.
//!
//! Two flows:
//!
//! - **Upload** (after a local publish): walk the live-sync tree from a
//!   root CID, collect every current hash, push each blob to the
//!   configured write servers via `BlossomClient::upload_if_missing`.
//! - **Download** (after a remote drive-root event): walk the same
//!   tree, but through a [`WriteBackBlossomStore`] that reads from
//!   Blossom on local-store miss and persists the fetched bytes back
//!   to local. The live sync block walker skips old `.hashtree/prev` history
//!   targets so current sync is not blocked by stale historical bytes.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hashtree_blossom::{BlobAvailability, BlossomClient};
use hashtree_core::{
    Cid, Hash, HashTree, HashTreeConfig, HashTreeError, Store, StoreError, to_hex,
};
use thiserror::Error;

use crate::block_sync::collect_live_sync_hashes;

#[derive(Debug, Error)]
pub enum BlossomSyncError {
    #[error("no blossom servers configured")]
    NoServers,
    #[error("blossom: {0}")]
    Blossom(String),
    #[error("tree walk: {0}")]
    Tree(#[from] HashTreeError),
    #[error("local store: {0}")]
    LocalStore(String),
    #[error("missing block locally: {0}")]
    MissingLocally(String),
    #[error("missing block on blossom: {0}")]
    MissingOnBlossom(String),
}

/// Counts of work done by [`upload_tree`].
#[derive(Debug, Default, Clone, Copy)]
pub struct UploadReport {
    pub total_hashes: usize,
    pub uploaded: usize,
    pub already_present: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackupCheckReport {
    pub total_hashes: usize,
    pub sample_size: usize,
    pub sampled_hashes: usize,
    pub present: usize,
    pub missing: usize,
    pub unknown: usize,
    pub latency_ms: Option<u64>,
    pub download_bytes: Option<usize>,
    pub download_ms: Option<u64>,
    pub download_bytes_per_second: Option<u64>,
    pub error: Option<String>,
}

impl BackupCheckReport {
    #[must_use]
    pub fn state(&self) -> &'static str {
        if self.missing > 0 {
            "missing"
        } else if self.unknown > 0 || self.error.is_some() {
            "unknown"
        } else if self.sampled_hashes > 0 {
            "verified"
        } else {
            "unknown"
        }
    }
}

/// Walk the local tree rooted at `root` and upload every live-sync block to the
/// configured Blossom write servers. Skips blobs the server already
/// has via `upload_if_missing`.
pub async fn upload_tree<S>(
    tree: &HashTree<S>,
    root: &Cid,
    client: &BlossomClient,
) -> Result<UploadReport, BlossomSyncError>
where
    S: Store + Send + Sync + 'static,
{
    if client.write_servers().is_empty() {
        return Err(BlossomSyncError::NoServers);
    }
    let hashes: HashSet<Hash> = collect_live_sync_hashes(tree, root, 4).await?;
    let mut report = UploadReport {
        total_hashes: hashes.len(),
        ..Default::default()
    };
    let store = tree.get_store();
    for hash in hashes {
        let bytes = store
            .get(&hash)
            .await
            .map_err(|e| BlossomSyncError::LocalStore(e.to_string()))?
            .ok_or_else(|| BlossomSyncError::MissingLocally(to_hex(&hash)))?;
        let (_, uploaded) = client
            .upload_if_missing(&bytes)
            .await
            .map_err(|e| BlossomSyncError::Blossom(e.to_string()))?;
        if uploaded {
            report.uploaded += 1;
        } else {
            report.already_present += 1;
        }
    }
    Ok(report)
}

/// Check one Blossom server for a deterministic sample of live-sync blocks.
///
/// The storage check uses sampled `HEAD` requests for coverage. If a sampled
/// block is present, the function downloads one present block to measure the
/// real read path and estimate transfer bandwidth.
pub async fn check_tree_on_server<S>(
    tree: &HashTree<S>,
    root: &Cid,
    client: &BlossomClient,
    server: &str,
    sample_size: usize,
) -> Result<BackupCheckReport, BlossomSyncError>
where
    S: Store + Send + Sync + 'static,
{
    let hashes: HashSet<Hash> = collect_live_sync_hashes(tree, root, 4).await?;
    let sample_size = sample_size.max(1);
    let mut sorted = hashes.into_iter().collect::<Vec<_>>();
    sorted.sort_unstable();
    let samples = spread_hash_samples(&sorted, sample_size);
    let mut report = BackupCheckReport {
        total_hashes: sorted.len(),
        sample_size,
        sampled_hashes: samples.len(),
        ..BackupCheckReport::default()
    };
    let mut present_probe: Option<Hash> = None;
    let mut latency_total = Duration::ZERO;
    let mut latency_count = 0u64;
    let checks = samples.into_iter().map(|hash| async move {
        let hex = to_hex(&hash);
        let started = Instant::now();
        let availability = client.check_on_server(&hex, server).await;
        (hash, availability, started.elapsed())
    });

    for (hash, availability, latency) in futures::future::join_all(checks).await {
        latency_total += latency;
        latency_count = latency_count.saturating_add(1);
        match availability {
            BlobAvailability::Present => {
                report.present += 1;
                present_probe.get_or_insert(hash);
            }
            BlobAvailability::Missing => {
                report.missing += 1;
            }
            BlobAvailability::Unknown => {
                report.unknown += 1;
            }
        }
    }

    if latency_count > 0 {
        let average = latency_total / u32::try_from(latency_count).unwrap_or(u32::MAX);
        report.latency_ms = Some(duration_millis_u64(average));
    }

    if let Some(hash) = present_probe {
        measure_download_probe(client, &hash, &mut report).await;
    }

    Ok(report)
}

fn spread_hash_samples(hashes: &[Hash], sample_size: usize) -> Vec<Hash> {
    if hashes.is_empty() {
        return Vec::new();
    }
    let wanted = sample_size.min(hashes.len());
    let step = (hashes.len() / wanted).max(1);
    hashes.iter().step_by(step).take(wanted).copied().collect()
}

async fn measure_download_probe(
    client: &BlossomClient,
    hash: &Hash,
    report: &mut BackupCheckReport,
) {
    let hex = to_hex(hash);
    let started = Instant::now();
    match client.download(&hex).await {
        Ok(bytes) => {
            let elapsed = started.elapsed();
            let elapsed_ms = duration_millis_u64(elapsed).max(1);
            report.download_bytes = Some(bytes.len());
            report.download_ms = Some(elapsed_ms);
            report.download_bytes_per_second = Some(bytes_per_second(bytes.len(), elapsed_ms));
        }
        Err(error) => {
            report.error = Some(format!("download probe failed: {error}"));
        }
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn bytes_per_second(bytes: usize, elapsed_ms: u64) -> u64 {
    let bytes = u128::try_from(bytes).unwrap_or(u128::MAX);
    let elapsed = u128::from(elapsed_ms.max(1));
    u64::try_from(bytes.saturating_mul(1_000) / elapsed).unwrap_or(u64::MAX)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DownloadReport {
    pub total_hashes: usize,
    pub fetched: usize,
    pub already_local: usize,
}

/// A `Store` adapter that reads-from-local-or-falls-back-to-Blossom,
/// persisting any fetched blob back to local on the way through.
/// Mutations are local-only (Blossom is read-only from this side; the
/// owner uploads explicitly).
pub struct WriteBackBlossomStore<L: Store + Send + Sync + 'static> {
    local: Arc<L>,
    client: BlossomClient,
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L: Store + Send + Sync + 'static> WriteBackBlossomStore<L> {
    pub fn new(local: Arc<L>, client: BlossomClient) -> Self {
        Self {
            local,
            client,
            fetched: std::sync::atomic::AtomicUsize::new(0),
            already_local: std::sync::atomic::AtomicUsize::new(0),
            missing: std::sync::atomic::AtomicUsize::new(0),
            first_missing: Mutex::new(None),
        }
    }

    pub fn fetched(&self) -> usize {
        self.fetched.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn already_local(&self) -> usize {
        self.already_local
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn missing(&self) -> usize {
        self.missing.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn first_missing(&self) -> Option<String> {
        self.first_missing
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

#[async_trait]
impl<L: Store + Send + Sync + 'static> Store for WriteBackBlossomStore<L> {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get(hash).await? {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(bytes));
        }
        let hex = to_hex(hash);
        let Some(bytes) = self.client.try_download(&hex).await else {
            if self
                .missing
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0
                && let Ok(mut first_missing) = self.first_missing.lock()
            {
                *first_missing = Some(hex);
            }
            return Ok(None);
        };
        // Write-back to local so subsequent ops hit cache and don't
        // re-fetch.
        self.local.put(*hash, bytes.clone()).await?;
        self.fetched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(bytes))
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        if self.local.has(hash).await? {
            return Ok(true);
        }
        Ok(self.client.exists(&to_hex(hash)).await)
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}

/// Walk the live-sync tree rooted at `root` through a writeback store. Every
/// block not already in `local_store` gets fetched from Blossom and
/// persisted. Returns counts of fetched vs. already-present blocks.
///
/// Caller's responsibility: only invoke this for roots produced by
/// authorized `AppKeys`. Iris-drive's relay-sync apply step has already
/// done the authorization check by the time this is called.
pub async fn download_tree<L>(
    local_store: Arc<L>,
    root: &Cid,
    client: BlossomClient,
) -> Result<DownloadReport, BlossomSyncError>
where
    L: Store + Send + Sync + 'static,
{
    if client.read_servers().is_empty() {
        return Err(BlossomSyncError::NoServers);
    }
    let writeback = Arc::new(WriteBackBlossomStore::new(local_store, client));
    let tree = HashTree::new(HashTreeConfig::new(writeback.clone()));
    let hashes = collect_live_sync_hashes(&tree, root, 4).await?;
    if writeback.missing() > 0 {
        let detail = writeback
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", writeback.missing()));
        return Err(BlossomSyncError::MissingOnBlossom(detail));
    }

    Ok(DownloadReport {
        total_hashes: hashes.len(),
        fetched: writeback.fetched(),
        already_local: writeback.already_local(),
    })
}

/// Retry [`download_tree`] across a bounded set of delays when Blossom
/// metadata is visible before the just-uploaded blocks are readable from
/// the backing CDN.
pub async fn download_tree_with_retry<L>(
    local_store: Arc<L>,
    root: &Cid,
    client: BlossomClient,
    retry_delays_secs: &[u64],
) -> Result<DownloadReport, BlossomSyncError>
where
    L: Store + Send + Sync + 'static,
{
    let mut attempt = 0usize;
    loop {
        match download_tree(local_store.clone(), root, client.clone()).await {
            Ok(report) => return Ok(report),
            Err(BlossomSyncError::MissingOnBlossom(_)) if attempt < retry_delays_secs.len() => {
                let delay = retry_delays_secs[attempt];
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            Err(err) => return Err(err),
        }
    }
}
