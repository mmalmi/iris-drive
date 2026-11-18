//! Verified ownership and return requests within encrypted shared Drive roots.
use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, HashTree, LinkType, Store, TreeEntry};
use nostr_sdk::{Event, JsonUtil, Keys};
use uuid::Uuid;

use crate::NostrIdentityId;
use crate::sharing::{SharedFolder, shared_folder_app_key_can_write_roots};

mod wire;
pub use wire::{
    FileOwner, FileReturn, NOTE_LABEL, OWNER_META, RETURN_META, RETURNED_FROM_META, ReturnNote,
    ReturnPhase, build_return_note, read_owner, read_request, read_return_note, return_note_filter,
    revision, sign_return_request,
};

mod edit;
pub(crate) use edit::prepare_edit;
pub(crate) mod markers;
mod storage;
mod sync;
pub(crate) use sync::remember_requests;
pub use sync::{queue_shared_file_return, sync_shared_file_returns};

#[cfg(test)]
mod tests;

fn active_profile(folder: &SharedFolder, app_key: &str) -> Option<NostrIdentityId> {
    let projection = folder.projection();
    let facet = projection.active_facets.get(app_key)?;
    let profile = facet.profile_id?;
    (facet.is_app_key() && folder.active_member_for_app_key(app_key)?.profile_id == profile)
        .then_some(profile)
}

fn validated_owner(folder: &SharedFolder, entry: &TreeEntry) -> Result<Option<(Event, FileOwner)>> {
    let Some(event) = wire::signed_meta_at(entry, OWNER_META)? else {
        return Ok(None);
    };
    let claim = read_owner(&event)?;
    ensure!(
        claim.share == folder.share_id
            && active_profile(folder, &event.pubkey.to_hex()) == Some(claim.owner),
        "Shared file ownership cannot be verified"
    );
    Ok(Some((event, claim)))
}

struct PendingReturn {
    claim: Event,
    destination_is_trash: bool,
    event: Event,
    request: FileReturn,
    entry: TreeEntry,
}

async fn current_pending<S: Store>(
    tree: &HashTree<S>,
    folder: &SharedFolder,
    root: &Cid,
    request_event: &Event,
) -> Result<PendingReturn> {
    let request = read_request(request_event)?;
    ensure!(
        request.share == folder.share_id,
        "Return request belongs to another share"
    );
    let editor = request_event.pubkey.to_hex();
    ensure!(
        shared_folder_app_key_can_write_roots(folder, &editor),
        "Shared file return request requires an active editor"
    );
    let editor_profile = active_profile(folder, &editor)
        .context("Shared file return request requires a verified editor")?;
    let author_root = folder
        .app_key_roots
        .get(&editor)
        .context("The editor's signed request root is unavailable")?;
    ensure!(
        request.seen.iter().all(|(writer, seq)| {
            if writer == &editor {
                author_root.app_key_seq > *seq
            } else {
                author_root
                    .observed
                    .get(writer)
                    .is_some_and(|observed| observed.app_key_seq >= *seq)
            }
        }),
        "The signed share root does not cover the request's observed revisions"
    );
    let entry = entry_at(tree, root, &request.path)
        .await?
        .context("Shared file return request is obsolete")?;
    let (claim_event, claim) =
        validated_owner(folder, &entry)?.context("Shared file ownership cannot be verified")?;
    let pending = wire::signed_meta_at(&entry, RETURN_META)?
        .context("Shared file return request is no longer pending")?;
    ensure!(
        pending == *request_event
            && claim.item == request.item
            && claim.owner == request.owner
            && claim_event.id.to_hex() == request.claim
            && wire::entry_revision(&entry)? == request.revision,
        "Shared file return request does not name the current pending revision"
    );
    Ok(PendingReturn {
        claim: claim_event,
        destination_is_trash: editor_profile == request.owner,
        event: pending,
        request,
        entry,
    })
}

fn build_request(
    folder: &SharedFolder,
    entry: &TreeEntry,
    path: Vec<String>,
    keys: &Keys,
    now: u64,
) -> Result<Event> {
    let (claim_event, claim) = validated_owner(folder, entry)?
        .context("Shared file ownership cannot be verified. The file is still in the share.")?;
    let editor = keys.public_key().to_hex();
    ensure!(
        shared_folder_app_key_can_write_roots(folder, &editor)
            && active_profile(folder, &editor).is_some(),
        "Shared file return requires a verified active editor"
    );
    sign_return_request(
        keys,
        &FileReturn {
            v: 1,
            share: folder.share_id,
            item: claim.item,
            owner: claim.owner,
            claim: claim_event.id.to_hex(),
            revision: wire::entry_revision(entry)?,
            path,
            nonce: Uuid::new_v4().to_string(),
            seen: crate::shared_folder_authorized_writer_pubkeys(folder)
                .into_iter()
                .filter_map(|writer| {
                    let seq = folder.app_key_roots.get(&writer)?.app_key_seq;
                    (1..=wire::MAX_SAFE_SEQUENCE)
                        .contains(&seq)
                        .then_some((writer, seq))
                })
                .collect(),
        },
        now,
    )
}

async fn mark_pending<S: Store>(
    tree: &HashTree<S>,
    folder: &SharedFolder,
    root: &Cid,
    mut entry: TreeEntry,
    path: Vec<String>,
    keys: &Keys,
) -> Result<(TreeEntry, Event)> {
    wire::entry_revision(&entry)?;
    if let Some(event) = wire::signed_meta_at(&entry, RETURN_META)?
        && current_pending(tree, folder, root, &event).await.is_ok()
    {
        return Ok((entry, event));
    }
    let event = build_request(folder, &entry, path, keys, unix_now()?)?;
    put_signed_meta(&mut entry, RETURN_META, &event);
    Ok((entry, event))
}

async fn entry_at<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &[String],
) -> Result<Option<TreeEntry>> {
    let (name, parents) = path.split_last().context("Shared path is empty")?;
    let mut dir = root.clone();
    for part in parents {
        let Some(entry) = tree
            .list_directory_required(&dir)
            .await?
            .into_iter()
            .find(|entry| entry.name == *part)
        else {
            return Ok(None);
        };
        if entry.link_type != LinkType::Dir {
            return Ok(None);
        }
        dir = entry_cid(&entry);
    }
    Ok(tree
        .list_directory_required(&dir)
        .await?
        .into_iter()
        .find(|entry| entry.name == *name))
}

fn entry_cid(entry: &TreeEntry) -> Cid {
    Cid {
        hash: entry.hash,
        key: entry.key,
    }
}

async fn replace_entry<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &[String],
    entry: &TreeEntry,
) -> Result<Cid> {
    let (name, parent) = path.split_last().context("Shared path is empty")?;
    let parent: Vec<&str> = parent.iter().map(String::as_str).collect();
    Ok(tree
        .set_entry_with_meta(
            root,
            &parent,
            name,
            &entry_cid(entry),
            entry.size,
            entry.link_type,
            entry.meta.clone(),
        )
        .await?)
}

fn put_signed_meta(entry: &mut TreeEntry, field: &str, event: &Event) {
    entry
        .meta
        .get_or_insert_with(Default::default)
        .insert(field.into(), event.as_json().into());
}

fn unix_now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}
