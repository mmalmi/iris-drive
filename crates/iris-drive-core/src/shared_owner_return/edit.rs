use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, HashTree, Store, TreeEntry};
use nostr_sdk::{Event, Keys};

use super::{
    OWNER_META, RETURN_META, RETURNED_FROM_META, entry_cid, put_signed_meta, replace_entry,
};
use crate::sharing::SharedFolder;

pub(crate) struct PreparedEdit {
    pub root: Cid,
    pub allowed_removals: BTreeSet<String>,
    pub requests: Vec<Event>,
}

/// Bind new files to their author and carry the same claim through normal edits.
/// Only a move with an exact surviving entry may tombstone the old path here.
pub(crate) async fn prepare_edit<S: Store>(
    tree: &HashTree<S>,
    folder: &SharedFolder,
    keys: &Keys,
    edited: Cid,
    base: &Cid,
    current: &Cid,
    queue_removals: bool,
) -> Result<PreparedEdit> {
    let old = crate::indexer::required_visible_entries(tree, base).await?;
    let current_entries = if current == base {
        None
    } else {
        Some(crate::indexer::required_visible_entries(tree, current).await?)
    };
    let live = &current_entries.as_ref().unwrap_or(&old).files;
    let edited_entries = crate::indexer::required_visible_entries(tree, &edited).await?;
    let moves = confirmed_moves(folder, &old.files, live, &edited_entries.files);
    let moves_by_destination: BTreeMap<_, _> = moves
        .iter()
        .map(|(from, to)| (to.as_str(), from.as_str()))
        .collect();

    let mut result = edited;
    let mut requests = Vec::new();
    for (path, before) in &old.files {
        if edited_entries.files.contains_key(path) || moves.contains_key(path) {
            continue;
        }
        ensure!(
            queue_removals,
            "The shared file stays in the folder until its owner confirms it is saved"
        );
        ensure!(
            !edited_entries.dirs.contains_key(path)
                && path
                    .rsplit_once('/')
                    .is_none_or(|(parent, _)| edited_entries.dirs.contains_key(parent)),
            "Request removal of each shared file before removing its containing directory"
        );
        let entry = live
            .get(path)
            .context("The shared file changed. Refresh the folder and retry")?;
        ensure!(
            entry_cid(entry) == entry_cid(before) && owner_meta(entry) == owner_meta(before),
            "The shared file changed. Refresh the folder and retry"
        );
        let parts: Vec<String> = path.split('/').map(str::to_owned).collect();
        let (entry, request) =
            super::mark_pending(tree, folder, current, entry.clone(), parts.clone(), keys).await?;
        if request.pubkey == keys.public_key() {
            requests.push(request);
        }
        result = replace_entry(tree, &result, &parts, &entry).await?;
    }

    let owner = super::active_profile(folder, &keys.public_key().to_hex());
    for (path, mut entry) in edited_entries.files {
        let original = old.files.get(&path).or_else(|| {
            moves_by_destination
                .get(path.as_str())
                .and_then(|from| old.files.get(*from))
        });
        if original.is_some_and(|prior| same_link(prior, &entry))
            && !moves_by_destination.contains_key(path.as_str())
        {
            continue;
        }
        let current_entry = live.get(&path).or_else(|| {
            moves_by_destination
                .get(path.as_str())
                .and_then(|from| live.get(*from))
        });
        if let Some(original) = original {
            ensure!(
                current_entry.is_some_and(|current| owner_meta(current) == owner_meta(original)),
                "The shared file changed identity. Refresh the folder and retry"
            );
        } else {
            ensure!(
                current_entry.is_none(),
                "The shared destination already exists. Refresh the folder and retry"
            );
        }
        let before = entry.meta.clone();
        let meta = entry.meta.get_or_insert_with(Default::default);
        meta.remove(OWNER_META);
        meta.remove(RETURN_META);
        meta.remove(RETURNED_FROM_META);
        if let Some(source) = original {
            if let Some(claim) = owner_meta(source) {
                meta.insert(OWNER_META.into(), claim.clone());
            }
            if current_entry.is_some_and(|current| entry_cid(current) == entry_cid(&entry))
                && !moves_by_destination.contains_key(path.as_str())
                && let Some(request) = current_entry
                    .and_then(|current| current.meta.as_ref())
                    .and_then(|meta| meta.get(RETURN_META))
            {
                entry
                    .meta
                    .as_mut()
                    .context("Shared metadata disappeared")?
                    .insert(RETURN_META.into(), request.clone());
            }
        } else if let Some(owner) = owner {
            let claim = super::wire::signed_entry_owner(
                keys,
                folder.share_id,
                owner,
                &entry,
                super::unix_now()?,
            )?;
            put_signed_meta(&mut entry, OWNER_META, &claim);
        } else {
            anyhow::bail!(
                "Link this writing device to a verified share member before creating a file"
            );
        }
        if before != entry.meta {
            result = replace_entry(
                tree,
                &result,
                &path.split('/').map(str::to_owned).collect::<Vec<_>>(),
                &entry,
            )
            .await?;
        }
    }
    let mut allowed_removals: BTreeSet<_> = moves.into_keys().collect();
    allowed_removals.extend(
        old.dirs
            .into_keys()
            .filter(|path| !edited_entries.dirs.contains_key(path)),
    );
    Ok(PreparedEdit {
        root: result,
        allowed_removals,
        requests,
    })
}

fn confirmed_moves(
    folder: &SharedFolder,
    old: &BTreeMap<String, TreeEntry>,
    live: &BTreeMap<String, TreeEntry>,
    edited: &BTreeMap<String, TreeEntry>,
) -> BTreeMap<String, String> {
    let mut destinations = BTreeMap::<String, Vec<&str>>::new();
    for (path, entry) in edited {
        if !old.contains_key(path) && !live.contains_key(path) {
            destinations
                .entry(move_identity(entry))
                .or_default()
                .push(path);
        }
    }
    let mut matched = BTreeMap::new();
    for (path, source) in old {
        if edited.contains_key(path)
            || !live.get(path).is_some_and(|entry| same_link(entry, source))
        {
            continue;
        }
        let Some(group) = destinations.get_mut(&move_identity(source)) else {
            continue;
        };
        // Native providers can combine a rename with a write. In that
        // case the bytes and modified time change, but a verified claim
        // still identifies an unambiguous move within this share.
        let found = if group.len() == 1
            && super::validated_owner(folder, source).is_ok_and(|claim| claim.is_some())
        {
            Some(0)
        } else {
            group
                .iter()
                .position(|candidate| same_link(&edited[*candidate], source))
        };
        if let Some(index) = found {
            matched.insert(path.clone(), group.swap_remove(index).to_owned());
        }
    }
    matched
}

fn move_identity(entry: &TreeEntry) -> String {
    owner_meta(entry)
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| entry_cid(entry).to_string(), str::to_owned)
}

fn same_link(left: &TreeEntry, right: &TreeEntry) -> bool {
    left.hash == right.hash
        && left.key == right.key
        && left.size == right.size
        && left.link_type == right.link_type
        && left.meta == right.meta
}

fn owner_meta(entry: &TreeEntry) -> Option<&serde_json::Value> {
    entry.meta.as_ref()?.get(OWNER_META)
}
