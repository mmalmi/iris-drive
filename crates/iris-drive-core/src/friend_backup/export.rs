//! A snapshot allowlist over immutable encrypted blocks; exports copy no file data.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, Hash, Store, StoreError, sha256};
use hashtree_fs::FsBlobStore;

use super::storage::{BackupManifest, FriendBackupError, MAX_BLOB_BYTES};

pub struct PreparedBackup {
    pub manifest: BackupManifest,
    pub manifest_hash: Hash,
    /// This small export store contains only the encoded manifest. File blocks
    /// remain in their original encrypted store and are exposed through `route()`.
    pub store: Arc<FsBlobStore>,
    route: Arc<ExportRoute>,
}

impl PreparedBackup {
    #[must_use]
    pub fn route(&self) -> Arc<dyn BlobRoute> {
        self.route.clone()
    }
}

pub(super) async fn prepare_export(
    source: Arc<FsBlobStore>,
    manifest: BackupManifest,
    export_dir: &Path,
) -> Result<PreparedBackup, FriendBackupError> {
    let bytes = manifest.to_bytes()?;
    let manifest_hash = sha256(&bytes);
    let store = Arc::new(FsBlobStore::new(
        export_dir.join(hex::encode(manifest_hash)),
    )?);
    store.put(manifest_hash, bytes).await?;
    let mut allowed = BTreeMap::new();
    for entry in &manifest.entries {
        let mut hash = [0; 32];
        hex::decode_to_slice(&entry.hash, &mut hash)
            .map_err(|_| FriendBackupError::Invalid("invalid selected hash".into()))?;
        allowed.insert(hash, entry.size);
    }
    let route = Arc::new(ExportRoute {
        source,
        manifest_store: store.clone(),
        manifest_hash,
        allowed,
    });
    Ok(PreparedBackup {
        manifest,
        manifest_hash,
        store,
        route,
    })
}

struct ExportRoute {
    source: Arc<FsBlobStore>,
    manifest_store: Arc<FsBlobStore>,
    manifest_hash: Hash,
    allowed: BTreeMap<Hash, u64>,
}

#[async_trait]
impl BlobRoute for ExportRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        let (store, expected_size) = if request.hash == self.manifest_hash {
            (&self.manifest_store, None)
        } else if let Some(size) = self.allowed.get(&request.hash) {
            (&self.source, Some(*size))
        } else {
            return Ok(BlobReply::NoResult);
        };
        let Some(size) = store.blob_size(&request.hash).await? else {
            return Ok(BlobReply::NoResult);
        };
        if size > MAX_BLOB_BYTES as u64 || expected_size.is_some_and(|expected| expected != size) {
            return Err(StoreError::Other(
                "selected backup block has an invalid size".into(),
            ));
        }
        let Some(bytes) = store.get(&request.hash).await? else {
            return Ok(BlobReply::NoResult);
        };
        if bytes.len() > MAX_BLOB_BYTES
            || bytes.len() as u64 != size
            || sha256(&bytes) != request.hash
        {
            return Err(StoreError::Other(
                "selected backup block has an invalid hash or size".into(),
            ));
        }
        Ok(BlobReply::Data(bytes))
    }
}
