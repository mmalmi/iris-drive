//! Explicit, portable recovery without exposing the normal app identity.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use fips_core::PeerIdentity;
use hashtree_core::{BlobRoute, Cid, DirEntry, LinkType, Store};
use hashtree_fips_transport::{FipsPeerConfig, set_fips_peer_configs};
use nostr_sdk::{
    Keys,
    nips::{nip19::ToBech32, nip44},
};
use serde::{Deserialize, Serialize};

use super::exchange::{BACKUP_TOPIC, BackupMessage, fetch, parse_hash};
use super::routes::FriendReadRoute;
use super::runtime::BackupService;
use super::storage::BackupManifest;
use super::{FriendBackupConfig, backup_keys, parse_backup_contact};
use crate::config_lock::ConfigMutationLock;
use crate::{AppConfig, AppKey, Daemon};

/// Never serialized into status, logs, invitations, or network messages.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryFile {
    version: u8,
    backup_secret: String,
    friends: Vec<String>,
}

pub fn export_recovery(config_dir: &Path, output: &Path) -> Result<()> {
    let device = AppKey::load(crate::paths::key_path_in(config_dir))?;
    let keys = backup_keys(&device)?;
    let config = FriendBackupConfig::load(config_dir)?;
    let recovery = RecoveryFile {
        version: 1,
        backup_secret: keys.secret_key().to_bech32()?,
        friends: config
            .friends
            .into_iter()
            .map(|friend| friend.npub)
            .collect(),
    };
    ensure!(output.file_name().is_some(), "choose a recovery file name");
    let bytes = serde_json::to_vec_pretty(&recovery)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Write only the exact user-selected file, including under native sandbox grants.
    let mut file = options
        .open(output)
        .context("choose a new recovery file; an existing file will not be overwritten")?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(output);
        return Err(error).context("saving backup recovery file");
    }
    Ok(())
}

fn read_recovery(path: &Path) -> Result<(Keys, Vec<String>)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(32 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 32 * 1024, "recovery file is too large");
    let file: RecoveryFile =
        serde_json::from_slice(&bytes).context("invalid backup recovery file")?;
    ensure!(
        file.version == 1 && file.friends.len() <= super::MAX_FRIENDS,
        "unsupported backup recovery file"
    );
    let keys = Keys::parse(&file.backup_secret)
        .map_err(|_| anyhow::anyhow!("invalid backup recovery key"))?;
    let friends = file
        .friends
        .into_iter()
        .map(|friend| parse_backup_contact(&friend))
        .collect::<Result<Vec<_>>>()?;
    Ok((keys, friends))
}

/// Recover into a new folder in the current Drive, preserving existing files.
pub async fn restore_backup(
    config_dir: &Path,
    recovery_file: &Path,
    friend: &str,
) -> Result<String> {
    // Saved contacts are recovery hints; an explicitly supplied friend may have
    // started hosting after this key was exported.
    let (keys, _friends) = read_recovery(recovery_file)?;
    let friend = parse_backup_contact(friend)?;
    let device = AppKey::load(crate::paths::key_path_in(config_dir))?;
    let _lease = if backup_keys(&device)?.public_key() == keys.public_key() {
        Some(super::service_lease::try_acquire(config_dir)?.context(
            "pause syncing on this device before restoring with its active backup identity",
        )?)
    } else {
        None
    };
    let config = AppConfig::load_or_default(crate::paths::config_path_in(config_dir))?;
    ensure!(
        config.profile.is_some(),
        "create or restore your Drive profile before recovering files"
    );
    let mut service = BackupService::start(
        config_dir,
        keys.clone(),
        &config,
        Arc::new(FriendReadRoute::default()),
    )
    .await?;
    let result = async {
        set_fips_peer_configs(&service.endpoint, vec![FipsPeerConfig::new(friend.clone())]).await?;
        service
            .control
            .set_policy(BTreeSet::from([friend.clone()]), BTreeSet::new())
            .await?;
        let mut messages = service.control.subscribe();
        service
            .control
            .send(
                friend.clone(),
                BACKUP_TOPIC.into(),
                serde_json::to_vec(&BackupMessage::RequestHead)?,
            )
            .await?;
        let hash = tokio::time::timeout(Duration::from_mins(1), async {
            loop {
                let message = messages.recv().await?;
                if message.peer_id != friend || message.topic != BACKUP_TOPIC {
                    continue;
                }
                match serde_json::from_slice::<BackupMessage>(&message.data)? {
                    BackupMessage::Head { manifest_hash } => return parse_hash(&manifest_hash),
                    BackupMessage::Rejected { reason } => bail!(
                        "friend could not supply the backup: {}",
                        reason.chars().take(512).collect::<String>()
                    ),
                    _ => {}
                }
            }
        })
        .await
        .context("friend is unavailable; backup recovery was not verified")??;
        let source = service
            .transport
            .route_to(PeerIdentity::from_npub(&friend)?);
        let manifest = BackupManifest::from_bytes(&fetch(&source, hash).await?)?;
        restore_snapshot(config_dir, &keys, &manifest, &source).await
    }
    .await;
    service.shutdown().await;
    result
}

pub(crate) async fn restore_snapshot(
    config_dir: &Path,
    keys: &Keys,
    manifest: &BackupManifest,
    source: &dyn BlobRoute,
) -> Result<String> {
    manifest.validate()?;
    let root_text = nip44::decrypt(
        keys.secret_key(),
        &keys.public_key(),
        &manifest.encrypted_root_cid,
    )
    .context("this recovery key cannot open the backup")?;
    let root = Cid::parse(&root_text)?;
    ensure!(
        root.is_encrypted() && root.hash == parse_hash(&manifest.root_hash)?,
        "recovery capsule does not match the snapshot"
    );
    let daemon = Daemon::open(config_dir)?;
    for entry in &manifest.entries {
        let hash = parse_hash(&entry.hash)?;
        let bytes = fetch(source, hash).await?;
        ensure!(
            u64::try_from(bytes.len())? == entry.size,
            "backup block has the wrong size"
        );
        daemon.tree().get_store().put(hash, bytes).await?;
    }
    // A valid root capsule does not prove the host listed every file block.
    // Validate the complete live DAG before publishing a recovered folder.
    super::selection::collect_backup_hashes(daemon.tree(), &root)
        .await
        .context("backup is incomplete or contains invalid encrypted content")?;
    // Network I/O ends before locking the current Drive mutation.
    let _lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut daemon = Daemon::open(config_dir)?;
    let recovered = crate::indexer::filter_ignored_entries_from_root(daemon.tree(), &root).await?;
    let merged = crate::primary_merged_root(daemon.tree(), daemon.config()).await?;
    let mut entries = daemon
        .tree()
        .list_directory_required(&merged.root_cid)
        .await?
        .into_iter()
        .map(|entry| DirEntry {
            name: entry.name,
            hash: entry.hash,
            key: entry.key,
            link_type: entry.link_type,
            size: entry.size,
            meta: entry.meta,
        })
        .collect::<Vec<_>>();
    let name = format!(
        "Recovered backup {}",
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    ensure!(
        !entries.iter().any(|entry| entry.name == name),
        "recovery folder already exists"
    );
    entries.push(DirEntry {
        name: name.clone(),
        hash: recovered.hash,
        key: recovered.key,
        link_type: LinkType::Dir,
        size: 0,
        meta: None,
    });
    let visible = daemon.tree().put_directory(entries).await?;
    daemon.import_visible_root(visible).await?;
    crate::paths::touch_provider_root_signal_in(config_dir)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn incomplete_backup_cannot_publish_a_recovered_folder() {
        use hashtree_core::{HashTree, HashTreeConfig};
        use hashtree_fs::FsBlobStore;

        let temp = tempfile::tempdir().unwrap();
        let source = HashTree::new(HashTreeConfig::new(Arc::new(
            FsBlobStore::new(temp.path().join("source")).unwrap(),
        )));
        let (file, size) = source
            .put(b"saved file absent from the replacement install")
            .await
            .unwrap();
        let root = source
            .put_directory(vec![
                DirEntry::from_cid("saved.txt", &file)
                    .with_link_type(LinkType::File)
                    .with_size(size),
            ])
            .await
            .unwrap();
        let keys = Keys::generate();
        let capsule = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            root.to_string(),
            nip44::Version::V2,
        )
        .unwrap();
        let mut prepared = super::super::storage::prepare_backup(
            &source,
            &root,
            capsule,
            temp.path().join("export"),
        )
        .await
        .unwrap();
        prepared
            .manifest
            .entries
            .retain(|entry| entry.hash != hex::encode(file.hash));
        prepared.manifest.validate().unwrap();

        let destination = temp.path().join("replacement");
        let profile = crate::Profile::create(&destination, None).unwrap();
        let mut config = AppConfig {
            profile: Some(profile.state.clone()),
            relays: Vec::new(),
            ..AppConfig::default()
        };
        config.upsert_drive(crate::config::Drive::primary(profile.state.root_scope_id()));
        let config_path = crate::paths::config_path_in(&destination);
        config.save(&config_path).unwrap();
        let existing_source = tempfile::tempdir().unwrap();
        std::fs::write(existing_source.path().join("keep.txt"), b"keep this file").unwrap();
        let mut existing = Daemon::open(&destination).unwrap();
        existing
            .import_source_dir(existing_source.path())
            .await
            .unwrap();
        let before = std::fs::read(&config_path).unwrap();

        let result = restore_snapshot(
            &destination,
            &keys,
            &prepared.manifest,
            prepared.route().as_ref(),
        )
        .await;
        assert!(
            result.is_err(),
            "incomplete backup was reported as restored"
        );
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        let existing = Daemon::open(&destination).unwrap();
        let current = Cid::parse(existing.primary_root().unwrap()).unwrap();
        let kept = existing
            .tree()
            .resolve(&current, "keep.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            existing.tree().get(&kept, None).await.unwrap().unwrap(),
            b"keep this file"
        );
        assert!(
            !existing
                .tree()
                .list_directory_required(&current)
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.name.starts_with("Recovered backup "))
        );
    }

    #[tokio::test]
    async fn recovery_accepts_an_explicit_friend_added_after_export() {
        let original = tempfile::tempdir().unwrap();
        AppKey::generate(crate::paths::key_path_in(original.path()))
            .save()
            .unwrap();
        let output = original.path().join("recovery.json");
        export_recovery(original.path(), &output).unwrap();
        let restored = tempfile::tempdir().unwrap();
        AppKey::generate(crate::paths::key_path_in(restored.path()))
            .save()
            .unwrap();
        let friend = Keys::generate().public_key().to_bech32().unwrap();
        let error = restore_backup(restored.path(), &output, &friend)
            .await
            .unwrap_err();
        // Contact validation succeeds and reaches the next real prerequisite.
        assert!(
            error
                .to_string()
                .contains("create or restore your Drive profile")
        );
    }

    #[test]
    fn recovery_export_is_private_and_restores_only_backup_identity() {
        let dir = tempfile::tempdir().unwrap();
        let key = AppKey::generate(crate::paths::key_path_in(dir.path()));
        key.save().unwrap();
        let output = dir.path().join("recovery.json");
        export_recovery(dir.path(), &output).unwrap();
        let before = std::fs::read(&output).unwrap();
        assert!(export_recovery(dir.path(), &output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), before);
        let (recovered, _) = read_recovery(&output).unwrap();
        assert_eq!(
            recovered.public_key(),
            backup_keys(&key).unwrap().public_key()
        );
        assert_ne!(recovered.public_key(), key.keys().public_key());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
