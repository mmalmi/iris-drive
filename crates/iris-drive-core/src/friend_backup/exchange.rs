//! Backup offers reveal only ciphertext hashes and an owner-encrypted root.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use fips_core::PeerIdentity;
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, Hash, MemoryStore, to_hex};
use hashtree_fips_transport::TcpBlobTransport;
use nostr_sdk::Keys;
use serde::{Deserialize, Serialize};

use super::FriendBackupConfig;
use super::routes::LocalBackupRoute;
use super::status::{PeerRuntimeStatus, write_runtime_status};
use super::storage::{BackupManifest, FriendBackupStore, PreparedBackup};
use crate::fips_sync::control_runtime::DriveControlRuntime;

pub(crate) const BACKUP_TOPIC: &str = "iris.drive.friend-backup/1";
const OFFER_INTERVAL: Duration = Duration::from_mins(1);
const AUDIT_INTERVAL_SECS: i64 = 6 * 60 * 60;
const AUDIT_SAMPLES: usize = 4;

#[path = "exchange_snapshot.rs"]
mod snapshot;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BackupMessage {
    Offer {
        manifest_hash: String,
    },
    RequestHead,
    Head {
        manifest_hash: String,
    },
    Stored {
        manifest_hash: String,
        root_hash: String,
    },
    Rejected {
        reason: String,
    },
}

pub(crate) struct ExportedBackup {
    root: String,
    source_fingerprint: Hash,
    backup: PreparedBackup,
    _directory: tempfile::TempDir,
}

pub(crate) struct Exchange {
    pub config_dir: PathBuf,
    pub keys: Keys,
    pub store: FriendBackupStore,
    pub local_route: Arc<LocalBackupRoute>,
    pub prepared: Option<ExportedBackup>,
    pub last_offer: Option<Instant>,
    pub status: BTreeMap<String, PeerRuntimeStatus>,
}

impl Exchange {
    pub async fn refresh(
        &mut self,
        config: &FriendBackupConfig,
        control: &DriveControlRuntime,
        transport: &Arc<TcpBlobTransport<MemoryStore>>,
    ) -> Result<()> {
        self.status
            .retain(|npub, _| config.friends.iter().any(|peer| &peer.npub == npub));
        self.refresh_snapshot().await?;
        self.refresh_local_routes(config)?;
        if self
            .last_offer
            .is_none_or(|last| last.elapsed() >= OFFER_INTERVAL)
        {
            if let Some(exported) = self.prepared.as_ref() {
                let offer = serde_json::to_vec(&BackupMessage::Offer {
                    manifest_hash: to_hex(&exported.backup.manifest_hash),
                })?;
                for peer in config.friends.iter().filter(|peer| peer.enabled) {
                    if let Err(error) = control
                        .send(peer.npub.clone(), BACKUP_TOPIC.into(), offer.clone())
                        .await
                    {
                        self.status.entry(peer.npub.clone()).or_default().error =
                            Some(error.to_string());
                    }
                }
            }
            self.last_offer = Some(Instant::now());
        }
        let now = unix_now();
        for peer in config.friends.iter().filter(|peer| peer.enabled) {
            let saved = self.status.get(&peer.npub).cloned().unwrap_or_default();
            if saved.last_synced_at.is_some() && audit_due(&saved, now) {
                self.status
                    .entry(peer.npub.clone())
                    .or_default()
                    .last_audit_attempt_at = Some(now);
                match self.audit(&peer.npub, transport).await {
                    Ok(()) => {
                        let status = self.status.entry(peer.npub.clone()).or_default();
                        status.last_checked_at = Some(now);
                        status.error = None;
                    }
                    Err(error) => {
                        self.status.entry(peer.npub.clone()).or_default().error =
                            Some(error.to_string());
                    }
                }
            }
        }
        write_runtime_status(&self.config_dir, &self.status)
    }

    fn refresh_local_routes(&self, config: &FriendBackupConfig) -> Result<()> {
        let mut routes = Vec::new();
        if let Some(exported) = self.prepared.as_ref() {
            routes.push(exported.backup.route());
        }
        for peer in config.friends.iter().filter(|peer| peer.enabled) {
            routes.push(self.store.route_for(&peer.npub)?);
        }
        self.local_route.set_routes(routes);
        Ok(())
    }

    pub async fn handle(
        &mut self,
        peer: &str,
        data: &[u8],
        control: &DriveControlRuntime,
        transport: &Arc<TcpBlobTransport<MemoryStore>>,
    ) -> Result<()> {
        ensure!(data.len() <= 4096, "backup control message too large");
        let config = FriendBackupConfig::load(&self.config_dir)?;
        let friend = config
            .friends
            .iter()
            .find(|friend| friend.npub == peer && friend.enabled)
            .context("friend is not authorized")?;
        let message: BackupMessage = serde_json::from_slice(data)?;
        match message {
            BackupMessage::RequestHead => {
                if let Some(manifest) = self.store.current_manifest(peer)? {
                    let hash = hashtree_core::sha256(&manifest.to_bytes()?);
                    let reply = BackupMessage::Head {
                        manifest_hash: to_hex(&hash),
                    };
                    control
                        .send(
                            peer.into(),
                            BACKUP_TOPIC.into(),
                            serde_json::to_vec(&reply)?,
                        )
                        .await?;
                } else {
                    self.reject(
                        control,
                        peer,
                        "No completed backup is stored for this identity",
                    )
                    .await?;
                }
            }
            BackupMessage::Head { .. } => {}
            BackupMessage::Offer { manifest_hash } => {
                if friend.quota_bytes == 0 {
                    return self
                        .reject(
                            control,
                            peer,
                            "No backup space has been offered on this device",
                        )
                        .await;
                }
                let result = self.receive_backup(peer, &manifest_hash, transport).await;
                match result {
                    Ok(manifest) => {
                        self.refresh_local_routes(&config)?;
                        self.status.entry(peer.to_string()).or_default().hosted_at =
                            Some(unix_now());
                        let reply = BackupMessage::Stored {
                            manifest_hash,
                            root_hash: manifest.root_hash,
                        };
                        control
                            .send(
                                peer.into(),
                                BACKUP_TOPIC.into(),
                                serde_json::to_vec(&reply)?,
                            )
                            .await?;
                    }
                    Err(error) => {
                        self.reject(control, peer, "Backup could not be stored; check the friend's offered space and connection").await?;
                        return Err(error);
                    }
                }
            }
            BackupMessage::Stored {
                manifest_hash,
                root_hash,
            } => {
                let Some(prepared) = self.prepared.as_ref() else {
                    bail!("no backup is being offered");
                };
                ensure!(
                    manifest_hash == to_hex(&prepared.backup.manifest_hash)
                        && root_hash == prepared.backup.manifest.root_hash,
                    "backup acknowledgement refers to a different snapshot"
                );
                let status = self.status.entry(peer.into()).or_default();
                if status.root_hash.as_deref() != Some(&root_hash) {
                    status.last_checked_at = None;
                    status.last_audit_attempt_at = None;
                }
                status.root_hash = Some(root_hash);
                status.last_synced_at = Some(unix_now());
                // A receipt is not a proof. Independently retrieve fresh samples
                // for changed snapshots and when the periodic audit is due.
                if !audit_due(status, unix_now()) {
                    return write_runtime_status(&self.config_dir, &self.status);
                }
                status.last_audit_attempt_at = Some(unix_now());
                if let Err(error) = self.audit(peer, transport).await {
                    self.status.entry(peer.into()).or_default().error = Some(error.to_string());
                } else {
                    let status = self.status.entry(peer.into()).or_default();
                    status.last_checked_at = Some(unix_now());
                    status.error = None;
                }
            }
            BackupMessage::Rejected { reason } => {
                ensure!(reason.len() <= 512, "backup error is too long");
                self.status.entry(peer.into()).or_default().error = Some(reason);
            }
        }
        write_runtime_status(&self.config_dir, &self.status)
    }

    async fn receive_backup(
        &self,
        peer: &str,
        manifest_hash: &str,
        transport: &Arc<TcpBlobTransport<MemoryStore>>,
    ) -> Result<BackupManifest> {
        let source = transport.route_to(PeerIdentity::from_npub(peer)?);
        let hash = parse_hash(manifest_hash)?;
        let bytes = fetch(&source, hash).await?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "backup manifest exceeds its size limit"
        );
        let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
        self.store
            .retain_authorized(&self.config_dir, peer, &manifest, &source)
            .await?;
        Ok(manifest)
    }

    async fn reject(&self, control: &DriveControlRuntime, peer: &str, reason: &str) -> Result<()> {
        control
            .send(
                peer.into(),
                BACKUP_TOPIC.into(),
                serde_json::to_vec(&BackupMessage::Rejected {
                    reason: reason.into(),
                })?,
            )
            .await?;
        Ok(())
    }

    pub fn record_error(&mut self, peer: &str, error: &str) -> Result<()> {
        if self.status.len() < super::MAX_FRIENDS || self.status.contains_key(peer) {
            self.status.entry(peer.into()).or_default().error =
                Some(error.chars().take(512).collect());
        }
        write_runtime_status(&self.config_dir, &self.status)
    }

    pub(crate) async fn audit(
        &self,
        peer: &str,
        transport: &Arc<TcpBlobTransport<MemoryStore>>,
    ) -> Result<()> {
        let prepared = self
            .prepared
            .as_ref()
            .context("no current backup to check")?;
        let entries = &prepared.backup.manifest.entries;
        ensure!(!entries.is_empty(), "backup has no blocks");
        let source = transport.route_to(PeerIdentity::from_npub(peer)?);
        for index in random_sample_indices(entries.len(), AUDIT_SAMPLES) {
            let entry = &entries[index];
            let bytes = fetch(&source, parse_hash(&entry.hash)?).await?;
            ensure!(
                u64::try_from(bytes.len())? == entry.size,
                "friend returned a backup block with the wrong size"
            );
        }
        Ok(())
    }
}

pub(crate) async fn fetch(source: &dyn BlobRoute, hash: Hash) -> Result<Vec<u8>> {
    let reply = tokio::time::timeout(
        Duration::from_secs(12),
        source.route(BlobRequest { hash, htl: 0 }),
    )
    .await
    .context("friend backup request timed out (unverified)")??;
    match reply {
        BlobReply::Data(bytes) => {
            ensure!(
                hashtree_core::sha256(&bytes) == hash,
                "friend returned corrupt backup data"
            );
            Ok(bytes)
        }
        BlobReply::NoResult => bail!("friend reports a missing backup block"),
    }
}

pub(crate) fn parse_hash(value: &str) -> Result<Hash> {
    let bytes = hex::decode(value)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid backup hash"))
}

fn random_sample_indices(length: usize, requested: usize) -> BTreeSet<usize> {
    let count = requested.min(length);
    let mut indices = BTreeSet::new();
    while indices.len() < count {
        let random = u128::from_le_bytes(*uuid::Uuid::new_v4().as_bytes());
        #[allow(clippy::cast_possible_truncation)]
        let index = (random % length as u128) as usize;
        indices.insert(index);
    }
    indices
}

fn audit_due(status: &PeerRuntimeStatus, now: i64) -> bool {
    status
        .last_checked_at
        .is_none_or(|checked| now.saturating_sub(checked) >= AUDIT_INTERVAL_SECS)
        && status
            .last_audit_attempt_at
            .is_none_or(|attempt| now.saturating_sub(attempt) >= 15 * 60)
}

fn unix_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_receipts_do_not_repeat_recent_audits() {
        let mut status = PeerRuntimeStatus::default();
        assert!(audit_due(&status, 100));
        status.last_audit_attempt_at = Some(100);
        assert!(!audit_due(&status, 160));
        assert!(audit_due(&status, 1000));
        status.last_checked_at = Some(1000);
        assert!(!audit_due(&status, 2000));
        assert!(audit_due(&status, 1000 + AUDIT_INTERVAL_SECS));
    }

    #[test]
    fn audit_samples_are_unique_and_cover_small_backups() {
        assert_eq!(random_sample_indices(3, 4), BTreeSet::from([0, 1, 2]));
        let samples = random_sample_indices(1000, 4);
        assert_eq!(samples.len(), 4);
        assert!(samples.iter().all(|index| *index < 1000));
    }
}
