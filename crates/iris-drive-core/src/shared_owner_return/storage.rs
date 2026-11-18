use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, HashTree, Store, TreeEntry};
use nostr_sdk::Keys;

use super::{OWNER_META, PendingReturn, RETURN_META, RETURNED_FROM_META, entry_at, entry_cid};
use crate::{AppKeyRootRef, Daemon, PRIMARY_DRIVE_ID};

pub(super) async fn verify_payload<S: Store>(tree: &HashTree<S>, entry: &TreeEntry) -> Result<()> {
    let cid = entry_cid(entry);
    super::revision(&cid)?;
    crate::block_sync::verify_reachable_blocks(tree, &cid).await?;
    Ok(())
}

pub(super) struct PrivateCopy {
    pub root: AppKeyRootRef,
    destination: PrivateDestination,
    pub recipients: Vec<String>,
    pub scope: String,
}

enum PrivateDestination {
    Drive(Vec<String>),
    Trash(crate::private_trash::TrashItem),
}

impl PrivateCopy {
    pub fn drive_id(&self) -> &'static str {
        match self.destination {
            PrivateDestination::Drive(_) => PRIMARY_DRIVE_ID,
            PrivateDestination::Trash(_) => crate::private_trash::DRIVE_ID,
        }
    }

    pub async fn verify<S: Store>(
        &self,
        tree: &HashTree<S>,
        root: &Cid,
        pending: &PendingReturn,
    ) -> Result<()> {
        match &self.destination {
            PrivateDestination::Drive(path) => check_private_copy(tree, root, path, pending).await,
            PrivateDestination::Trash(item) => {
                crate::private_trash::verify_share_return(tree, root, item, &private_entry(pending))
                    .await
            }
        }
    }
}

fn private_entry(pending: &PendingReturn) -> TreeEntry {
    let mut entry = pending.entry.clone();
    if let Some(meta) = &mut entry.meta {
        meta.remove(OWNER_META);
        meta.remove(RETURN_META);
        meta.remove(RETURNED_FROM_META);
        if meta.is_empty() {
            entry.meta = None;
        }
    }
    entry
}

pub(super) async fn stage_private_copy(
    daemon: &mut Daemon,
    pending: &PendingReturn,
    keys: &Keys,
) -> Result<PrivateCopy> {
    verify_payload(daemon.tree(), &pending.entry).await?;
    let config = daemon.config();
    let state = config.profile.as_ref().context("Profile required")?;
    let pubkey = keys.public_key().to_hex();
    ensure!(
        state.profile_id == pending.request.owner
            && state.app_key_pubkey == pubkey
            && crate::private_trash::can_write(state, &pubkey),
        "An approved owner device is required to save the return"
    );

    let (drive_id, destination) = if pending.destination_is_trash {
        let item = crate::private_trash::TrashItem {
            v: 1,
            id: pending.request.nonce.clone(),
            account: state.profile_id,
            source_scope: state.profile_id,
            tree_name: PRIMARY_DRIVE_ID.into(),
            path: vec![pending.entry.name.clone()],
            deleted_at: i64::try_from(pending.event.created_at.as_secs())?,
        };
        ensure!(
            !item.expired(i64::try_from(super::unix_now()?)?),
            "Share return request has expired"
        );
        crate::private_trash::stage_share_return(daemon, &item, &private_entry(pending)).await?;
        (
            crate::private_trash::DRIVE_ID,
            PrivateDestination::Trash(item),
        )
    } else {
        (
            PRIMARY_DRIVE_ID,
            PrivateDestination::Drive(stage_my_drive_copy(daemon, pending).await?),
        )
    };
    let state = daemon
        .config()
        .profile
        .as_ref()
        .context("Profile required")?;
    let drive = daemon
        .config()
        .drive(drive_id)
        .context("Private owner drive is missing")?;
    let root = crate::publishable_root::publishable_app_key_root(daemon.config_dir(), drive, state)
        .await?
        .context("This device has no publishable owner copy")?;
    let copy = PrivateCopy {
        root,
        destination,
        recipients: crate::recovery_wraps::drive_root_key_recipient_pubkeys(state, drive),
        scope: state.root_scope_id(),
    };
    copy.verify(daemon.tree(), &Cid::parse(&copy.root.root_cid)?, pending)
        .await?;
    Ok(copy)
}

async fn stage_my_drive_copy(daemon: &mut Daemon, pending: &PendingReturn) -> Result<Vec<String>> {
    let config = daemon.config();
    let mut personal = config.clone();
    personal.shared_folders.clear();
    personal.share_shortcuts.clear();
    let private_root = crate::primary_merged_root(daemon.tree(), &personal)
        .await?
        .root_cid;
    let receipt = pending.event.id.to_hex();
    if let Some(path) = find_receipt(daemon.tree(), &private_root, &receipt, &pending.entry).await?
    {
        return Ok(path);
    }
    let visible_root = crate::primary_merged_root(daemon.tree(), config)
        .await?
        .root_cid;
    let name = pending
        .request
        .path
        .last()
        .context("Shared file name required")?;
    let path = vec![available_name(daemon.tree(), &visible_root, name).await?];
    let mut entry = private_entry(pending);
    entry
        .meta
        .get_or_insert_with(Default::default)
        .insert(RETURNED_FROM_META.into(), receipt.into());
    let updated = super::replace_entry(daemon.tree(), &visible_root, &path, &entry).await?;
    daemon
        .import_visible_root_with_tombstone_base(updated, Some(visible_root))
        .await?;
    crate::paths::touch_provider_root_signal_in(daemon.config_dir())?;
    Ok(path)
}

pub(super) async fn check_private_copy<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &[String],
    pending: &PendingReturn,
) -> Result<()> {
    let entry = entry_at(tree, root, path)
        .await?
        .context("Owner copy is missing")?;
    ensure!(
        receipt_at(&entry) == Some(pending.event.id.to_hex().as_str())
            && entry_cid(&entry) == entry_cid(&pending.entry)
            && entry.size == pending.entry.size,
        "Owner copy does not match the exact requested file"
    );
    verify_payload(tree, &entry).await
}

async fn find_receipt<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    receipt: &str,
    source: &TreeEntry,
) -> Result<Option<Vec<String>>> {
    for (path, entry) in crate::indexer::required_visible_entries(tree, root)
        .await?
        .files
    {
        if receipt_at(&entry) == Some(receipt) {
            ensure!(
                entry_cid(&entry) == entry_cid(source) && entry.size == source.size,
                "A saved owner receipt refers to a different file"
            );
            return Ok(Some(path.split('/').map(str::to_owned).collect()));
        }
    }
    Ok(None)
}

fn receipt_at(entry: &TreeEntry) -> Option<&str> {
    entry.meta.as_ref()?.get(RETURNED_FROM_META)?.as_str()
}

async fn available_name<S: Store>(tree: &HashTree<S>, root: &Cid, name: &str) -> Result<String> {
    let names: std::collections::BTreeSet<_> = tree
        .list_directory_required(root)
        .await?
        .into_iter()
        .map(|entry| entry.name.to_lowercase())
        .collect();
    if !names.contains(&name.to_lowercase()) {
        return Ok(name.to_owned());
    }
    let (stem, extension) = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or((name, String::new()), |(stem, ext)| {
            (stem, format!(".{ext}"))
        });
    (2..=names.len().saturating_add(2))
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| !names.contains(&candidate.to_lowercase()))
        .context("No free name for the returned file")
}

/// Read encrypted blocks directly from configured storage during proof.
pub(super) struct RemoteRead(pub hashtree_blossom::BlossomClient);

#[async_trait::async_trait]
impl Store for RemoteRead {
    async fn put(
        &self,
        _hash: hashtree_core::Hash,
        _data: Vec<u8>,
    ) -> Result<bool, hashtree_core::StoreError> {
        Err(hashtree_core::StoreError::Other(
            "Owner return verification is read-only".into(),
        ))
    }

    async fn get(
        &self,
        hash: &hashtree_core::Hash,
    ) -> Result<Option<Vec<u8>>, hashtree_core::StoreError> {
        self.0
            .download(&hex::encode(hash))
            .await
            .map(Some)
            .map_err(|error| hashtree_core::StoreError::Other(error.to_string()))
    }

    async fn has(&self, hash: &hashtree_core::Hash) -> Result<bool, hashtree_core::StoreError> {
        Ok(self.get(hash).await?.is_some())
    }

    async fn delete(&self, _hash: &hashtree_core::Hash) -> Result<bool, hashtree_core::StoreError> {
        Err(hashtree_core::StoreError::Other(
            "Owner return verification is read-only".into(),
        ))
    }
}
