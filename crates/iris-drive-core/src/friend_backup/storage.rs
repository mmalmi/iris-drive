//! Opaque, quota-accounted friend backups. Keys and filenames never enter the manifest.
//!
//! A committed manifest names a complete snapshot. Failed transfers keep the preceding
//! snapshot and charge partial blobs to the same quota. Historical/orphaned bytes are
//! deliberately retained; removing a contact is not permission to erase their backup.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, Cid, Hash, HashTree, Store, sha256};
use hashtree_fs::FsBlobStore;
use nostr_sdk::{PublicKey, ToBech32};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

pub use super::export::PreparedBackup;
use super::export::prepare_export;
use super::selection::collect_backup_hashes;
use crate::config_lock::ConfigMutationLock;

pub const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_MANIFEST_ENTRIES: usize = 65_536;
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub const BLOB_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum FriendBackupError {
    #[error("invalid friend backup: {0}")]
    Invalid(String),
    #[error(
        "friend backup needs {required} bytes including retained snapshots; allocation is {quota} bytes"
    )]
    Quota { required: u64, quota: u64 },
    #[error("friend backup block is missing: {0}")]
    Missing(String),
    #[error("friend backup block has an incorrect hash or size: {0}")]
    InvalidBlob(String),
    #[error("friend backup retrieval timed out: {0}")]
    Timeout(String),
    #[error("friend backup access: {0}")]
    Policy(String),
    #[error("friend backup storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("friend backup blob store: {0}")]
    Store(#[from] hashtree_core::StoreError),
    #[error("friend backup tree: {0}")]
    Tree(#[from] hashtree_core::HashTreeError),
    #[error("friend backup manifest: {0}")]
    Json(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, FriendBackupError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BackupEntry {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub version: u8,
    pub root_hash: String,
    /// NIP-44 v2 encrypted *to the owner's backup identity*, never a plaintext CID.
    pub encrypted_root_cid: String,
    pub entries: Vec<BackupEntry>,
}

impl BackupManifest {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(invalid("manifest exceeds 8 MiB"));
        }
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.entries.is_empty() || self.entries.len() > MAX_MANIFEST_ENTRIES
        {
            return Err(invalid("unsupported version or invalid entry count"));
        }
        parse_hash(&self.root_hash)?;
        if self.encrypted_root_cid.len() > 16 * 1024 {
            return Err(invalid("encrypted recovery capsule exceeds limit"));
        }
        let capsule = base64::engine::general_purpose::STANDARD
            .decode(&self.encrypted_root_cid)
            .map_err(|_| invalid("recovery CID must be NIP-44 encrypted to its owner"))?;
        if capsule.first() != Some(&2) || capsule.len() < 99 {
            return Err(invalid(
                "recovery CID must be NIP-44 encrypted to its owner",
            ));
        }
        let mut hashes = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            let hash = parse_hash(&entry.hash)?;
            if entry.size > MAX_BLOB_BYTES as u64 || !hashes.insert(hash) {
                return Err(invalid("duplicate hash or blob exceeds 16 MiB"));
            }
        }
        if !hashes.contains(&parse_hash(&self.root_hash)?) {
            return Err(invalid("manifest does not contain its root"));
        }
        if serde_json::to_vec(self)?.len() > MAX_MANIFEST_BYTES {
            return Err(invalid("manifest exceeds 8 MiB"));
        }
        Ok(())
    }

    /// Deterministic selection with caller-provided unpredictable seed for audits.
    #[must_use]
    pub fn audit_entries(&self, seed: &[u8; 32], count: usize) -> Vec<&BackupEntry> {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| {
            let mut input = seed.to_vec();
            input.extend_from_slice(entry.hash.as_bytes());
            sha256(&input)
        });
        entries.truncate(count.min(entries.len()));
        entries
    }
}

/// Prepare a private snapshot. The caller must encrypt the root CID to its own
/// recoverable backup identity; the host receives only that capsule and hashes.
pub async fn prepare_backup(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
    encrypted_root_cid: String,
    export_dir: impl AsRef<Path>,
) -> Result<PreparedBackup> {
    if !root.is_encrypted() || !tree.is_encrypted() {
        return Err(invalid("friend backups require an encrypted root"));
    }
    let hashes = collect_backup_hashes(tree, root).await?;
    let source = tree.get_store();
    let mut entries = Vec::with_capacity(hashes.len());
    for hash in &hashes {
        let size = source
            .blob_size(hash)
            .await?
            .ok_or_else(|| FriendBackupError::Missing(hex::encode(hash)))?;
        if size > MAX_BLOB_BYTES as u64 {
            return Err(invalid("snapshot contains a blob exceeding 16 MiB"));
        }
        entries.push(BackupEntry {
            hash: hex::encode(hash),
            size,
        });
    }
    let manifest = BackupManifest {
        version: 1,
        root_hash: hex::encode(root.hash),
        encrypted_root_cid,
        entries,
    };
    prepare_export(source, manifest, export_dir.as_ref()).await
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetainReport {
    pub downloaded: usize,
    pub already_present: usize,
    pub retained_bytes: u64,
}

#[derive(Clone)]
pub struct FriendBackupStore {
    base_dir: PathBuf,
    mutation: Arc<Mutex<()>>,
}

enum Admission<'a> {
    Fixed(u64),
    Authorized(&'a Path),
}

struct Reservation {
    required_bytes: u64,
    accounted_bytes: BTreeMap<String, u64>,
    write_manifest: bool,
}

impl FriendBackupStore {
    pub fn open(base_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            mutation: Arc::new(Mutex::new(())),
        })
    }

    pub fn usage_bytes(&self, npub: &str) -> Result<u64> {
        directory_bytes(&self.peer_dir(npub)?)
    }

    /// Includes all retained history, interrupted writes and uncommitted blobs.
    pub fn all_usage_bytes(&self) -> Result<BTreeMap<String, u64>> {
        let mut usage = BTreeMap::new();
        if !self.base_dir.exists() {
            return Ok(usage);
        }
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid("unexpected entry in friend backup directory"));
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let key =
                PublicKey::parse(&name).map_err(|_| invalid("invalid friend backup directory"))?;
            if key.to_hex() != name {
                return Err(invalid("invalid friend backup directory"));
            }
            let npub = key.to_bech32().map_err(|e| invalid(e.to_string()))?;
            usage.insert(npub, directory_bytes(&entry.path())?);
        }
        Ok(usage)
    }

    pub fn current_manifest(&self, npub: &str) -> Result<Option<BackupManifest>> {
        Self::manifest_bytes(&self.peer_dir(npub)?)?
            .map(|bytes| BackupManifest::from_bytes(&bytes))
            .transpose()
    }

    /// The endpoint must authorize the requester before exposing its own route.
    pub fn route_for(&self, npub: &str) -> Result<Arc<dyn BlobRoute>> {
        self.peer_dir(npub)?;
        Ok(Arc::new(RetainedRoute {
            backups: self.clone(),
            npub: npub.to_string(),
        }))
    }

    /// Low-level entry point for an already-authorized allocation. Production
    /// configuration-backed callers should use `retain_authorized` instead.
    pub async fn retain(
        &self,
        npub: &str,
        quota_bytes: u64,
        manifest: &BackupManifest,
        source: &dyn BlobRoute,
    ) -> Result<RetainReport> {
        self.retain_inner(npub, manifest, source, Admission::Fixed(quota_bytes))
            .await
    }

    pub async fn retain_authorized(
        &self,
        config_dir: &Path,
        npub: &str,
        manifest: &BackupManifest,
        source: &dyn BlobRoute,
    ) -> Result<RetainReport> {
        self.retain_inner(npub, manifest, source, Admission::Authorized(config_dir))
            .await
    }

    async fn retain_inner(
        &self,
        npub: &str,
        manifest: &BackupManifest,
        source: &dyn BlobRoute,
        policy: Admission<'_>,
    ) -> Result<RetainReport> {
        let _serial = self.mutation.lock().await;
        let bytes = manifest.to_bytes()?;
        let peer_dir = self.peer_dir(npub)?;
        // Admission happens before any writes or network requests. Never keep the
        // configuration lock during retrieval: the owner can revoke access promptly.
        let reservation = self.reserve(npub, manifest, &bytes, &policy).await?;
        let mut report = RetainReport::default();
        for entry in &manifest.entries {
            let hash = parse_hash(&entry.hash)?;
            if let Some(data) = existing_blob(&peer_dir, &hash)? {
                verify_blob(entry, &data)?;
                report.already_present += 1;
                continue;
            }
            let response = tokio::time::timeout(
                BLOB_FETCH_TIMEOUT,
                source.route(BlobRequest { hash, htl: 0 }),
            )
            .await
            .map_err(|_| FriendBackupError::Timeout(entry.hash.clone()))??;
            let BlobReply::Data(data) = response else {
                return Err(FriendBackupError::Missing(entry.hash.clone()));
            };
            verify_blob(entry, &data)?;
            let _config = Self::recheck_reservation(npub, &reservation, &policy).await?;
            create_private_directory(&self.base_dir)?;
            create_private_directory(&peer_dir)?;
            let blocks = peer_dir.join("blocks");
            let store = FsBlobStore::new(&blocks)?;
            if store.put_sync(hash, &data)? {
                sync_blob(&blocks, &entry.hash)?;
                report.downloaded += 1;
            } else {
                let existing = store
                    .get_sync(&hash)?
                    .ok_or_else(|| FriendBackupError::Missing(entry.hash.clone()))?;
                verify_blob(entry, &existing)?;
                report.already_present += 1;
            }
        }
        let _config = Self::recheck_reservation(npub, &reservation, &policy).await?;
        if reservation.write_manifest {
            create_private_directory(&self.base_dir)?;
            create_private_directory(&peer_dir)?;
            let mut temporary = tempfile::NamedTempFile::new_in(&peer_dir)?;
            temporary.write_all(&bytes)?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(peer_dir.join("manifest.json"))
                .map_err(|e| e.error)?;
            sync_directory(&peer_dir)?;
            sync_directory(&self.base_dir)?;
            if let Some(parent) = self.base_dir.parent() {
                sync_directory(parent)?;
            }
        }
        report.retained_bytes = directory_bytes(&peer_dir)?;
        Ok(report)
    }

    async fn reserve(
        &self,
        npub: &str,
        manifest: &BackupManifest,
        bytes: &[u8],
        policy: &Admission<'_>,
    ) -> Result<Reservation> {
        let (_lock, config, quota) = Self::policy_snapshot(npub, policy).await?;
        let peer_dir = self.peer_dir(npub)?;
        let mut accounted_bytes = if config.is_some() {
            self.all_usage_bytes()?
        } else {
            BTreeMap::from([(npub.to_string(), directory_bytes(&peer_dir)?)])
        };
        let mut required = accounted_bytes.get(npub).copied().unwrap_or(0);
        for entry in &manifest.entries {
            let hash = parse_hash(&entry.hash)?;
            if blob_size(&peer_dir, &hash)?.is_none() {
                required = required
                    .checked_add(entry.size)
                    .ok_or_else(|| invalid("allocation overflow"))?;
            }
        }
        let write_manifest = Self::manifest_bytes(&peer_dir)?.as_deref() != Some(bytes);
        if write_manifest {
            required = required
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("allocation overflow"))?;
        }
        if required > quota {
            return Err(FriendBackupError::Quota { required, quota });
        }
        accounted_bytes.insert(npub.to_string(), required);
        if let Some(config) = config {
            config
                .validate_retained(&accounted_bytes)
                .map_err(|error| FriendBackupError::Policy(error.to_string()))?;
        }
        Ok(Reservation {
            required_bytes: required,
            accounted_bytes,
            write_manifest,
        })
    }

    /// The service's OS lease and this store's mutation mutex serialize writers.
    /// A single physical inventory reserves peak usage (old head, new head and
    /// all missing blocks). Rechecking at each write only reloads the small
    /// settings file, so a backup with N blocks does O(N) filesystem work.
    /// Settings mutations independently verify fresh on-disk usage under the
    /// same config lock; a quota reduction can interrupt this reservation.
    async fn recheck_reservation(
        npub: &str,
        reservation: &Reservation,
        policy: &Admission<'_>,
    ) -> Result<Option<ConfigMutationLock>> {
        let (lock, config, quota) = Self::policy_snapshot(npub, policy).await?;
        if reservation.required_bytes > quota {
            return Err(FriendBackupError::Quota {
                required: reservation.required_bytes,
                quota,
            });
        }
        if let Some(config) = config {
            config
                .validate_retained(&reservation.accounted_bytes)
                .map_err(|error| FriendBackupError::Policy(error.to_string()))?;
        }
        Ok(lock)
    }

    async fn policy_snapshot(
        npub: &str,
        policy: &Admission<'_>,
    ) -> Result<(
        Option<ConfigMutationLock>,
        Option<super::FriendBackupConfig>,
        u64,
    )> {
        match policy {
            Admission::Fixed(quota) => Ok((None, None, *quota)),
            Admission::Authorized(config_dir) => {
                let lock = ConfigMutationLock::acquire(config_dir)
                    .await
                    .map_err(|error| FriendBackupError::Policy(error.to_string()))?;
                let config = super::FriendBackupConfig::load(config_dir)
                    .map_err(|error| FriendBackupError::Policy(error.to_string()))?;
                let quota = config
                    .friends
                    .iter()
                    .find(|friend| friend.npub == npub && friend.enabled)
                    .ok_or_else(|| FriendBackupError::Policy("friend is not authorized".into()))?
                    .quota_bytes;
                Ok((Some(lock), Some(config), quota))
            }
        }
    }

    fn peer_dir(&self, npub: &str) -> Result<PathBuf> {
        let public = PublicKey::parse(npub).map_err(|_| invalid("invalid friend npub"))?;
        if public.to_bech32().map_err(|e| invalid(e.to_string()))? != npub {
            return Err(invalid("friend must use its canonical npub"));
        }
        Ok(self.base_dir.join(public.to_hex()))
    }

    fn manifest_bytes(peer_dir: &Path) -> Result<Option<Vec<u8>>> {
        let path = peer_dir.join("manifest.json");
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
            Ok(metadata) if metadata.len() > MAX_MANIFEST_BYTES as u64 => {
                Err(invalid("stored manifest exceeds 8 MiB"))
            }
            Ok(_) => Ok(Some(std::fs::read(path)?)),
        }
    }
}

struct RetainedRoute {
    backups: FriendBackupStore,
    npub: String,
}

#[async_trait]
impl BlobRoute for RetainedRoute {
    async fn route(
        &self,
        request: BlobRequest,
    ) -> std::result::Result<BlobReply, hashtree_core::StoreError> {
        let get = || -> Result<BlobReply> {
            let peer_dir = self.backups.peer_dir(&self.npub)?;
            if let Some(bytes) = FriendBackupStore::manifest_bytes(&peer_dir)?
                && sha256(&bytes) == request.hash
            {
                return Ok(BlobReply::Data(bytes));
            }
            match existing_blob(&peer_dir, &request.hash)? {
                Some(bytes) if sha256(&bytes) != request.hash => {
                    Err(FriendBackupError::InvalidBlob(hex::encode(request.hash)))
                }
                Some(bytes) => Ok(BlobReply::Data(bytes)),
                None => Ok(BlobReply::NoResult),
            }
        };
        get().map_err(|error| hashtree_core::StoreError::Other(error.to_string()))
    }
}

fn invalid(message: impl Into<String>) -> FriendBackupError {
    FriendBackupError::Invalid(message.into())
}

fn parse_hash(value: &str) -> Result<Hash> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "blob hash must be 64 lowercase hexadecimal characters",
        ));
    }
    let mut hash = [0; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| invalid("invalid blob hash"))?;
    Ok(hash)
}

fn verify_blob(entry: &BackupEntry, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_BLOB_BYTES
        || bytes.len() as u64 != entry.size
        || sha256(bytes) != parse_hash(&entry.hash)?
    {
        return Err(FriendBackupError::InvalidBlob(entry.hash.clone()));
    }
    Ok(())
}

fn existing_blob(peer_dir: &Path, hash: &Hash) -> Result<Option<Vec<u8>>> {
    let blocks = peer_dir.join("blocks");
    if !blocks.exists() {
        return Ok(None);
    }
    let store = FsBlobStore::new(blocks)?;
    if store
        .blob_size_sync(hash)?
        .is_some_and(|size| size > MAX_BLOB_BYTES as u64)
    {
        return Err(FriendBackupError::InvalidBlob(hex::encode(hash)));
    }
    Ok(store.get_sync(hash)?)
}

fn blob_size(peer_dir: &Path, hash: &Hash) -> Result<Option<u64>> {
    let blocks = peer_dir.join("blocks");
    if !blocks.exists() {
        return Ok(None);
    }
    Ok(FsBlobStore::new(blocks)?.blob_size_sync(hash)?)
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)?;
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    let mut directories = vec![path.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(invalid("symlinks are not allowed in friend backup storage"));
            }
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                total = total
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| invalid("storage accounting overflow"))?;
            } else {
                return Err(invalid("unexpected file in friend backup storage"));
            }
        }
    }
    Ok(total)
}

fn sync_blob(blocks: &Path, hash: &str) -> Result<()> {
    let directory = blocks.join(&hash[..2]).join(&hash[2..4]);
    std::fs::File::open(directory.join(&hash[4..]))?.sync_all()?;
    sync_directory(&directory)?;
    if let Some(parent) = directory.parent() {
        sync_directory(parent)?;
    }
    sync_directory(blocks)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
