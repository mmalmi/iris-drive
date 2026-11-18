use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, HashTree, LinkType, Store, TreeEntry};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::tree::{TrashTree, entry_cid, same_entry};
use super::{DRIVE_ID, TrashItem, TrashRemoval, can_write, valid_component, valid_id};
use crate::config_lock::ConfigMutationLock;
use crate::{
    Daemon, PRIMARY_DRIVE_ID, ProfileState, primary_merged_root_from_view, primary_merged_view,
};

/// List the available, unexpired items from all known account Trash roots.
pub async fn list(config_dir: &Path, now: i64) -> Result<Vec<TrashItem>> {
    let daemon = open(config_dir)?;
    let state = daemon
        .config()
        .profile
        .as_ref()
        .context("No Drive account")?;
    ensure!(
        super::can_open(state, &state.app_key_pubkey),
        "This device needs signed account approval"
    );
    let snapshot = read_heads(&daemon, state).await?;
    Ok(snapshot
        .items
        .into_values()
        .filter(|stored| !stored.item.expired(now))
        .map(|stored| stored.item)
        .collect())
}

/// Move an entry in the personal Drive into this account's private Trash.
pub async fn trash(config_dir: &Path, path: Vec<String>, now: i64) -> Result<TrashItem> {
    let _lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut daemon = open(config_dir)?;
    let state = state(&daemon)?;
    let item = TrashItem {
        v: 1,
        id: Uuid::new_v4().to_string(),
        account: state.profile_id,
        source_scope: state.profile_id,
        tree_name: PRIMARY_DRIVE_ID.into(),
        path,
        deleted_at: now,
    };
    item.validate(state.profile_id, &item.id)?;
    let mut snapshot = read_heads(&daemon, state).await?;
    let root = personal_root(&daemon, &item.path).await?;
    let (name, parent) = split_path(&item.path)?;
    let dir = resolve_parent(daemon.tree(), &root, &parent).await?;
    let content = lookup(daemon.tree(), &dir, name)
        .await?
        .context("Drive item does not exist")?;
    verify_content(daemon.tree(), &content).await?;
    snapshot.expire(daemon.tree(), now).await?;
    snapshot
        .insert(daemon.tree(), item.clone(), content)
        .await?;
    let archive = snapshot.write(daemon.tree()).await?;
    daemon
        .import_visible_root_for_drive(DRIVE_ID, archive)
        .await?;
    // A failure from here leaves a recoverable archive plus the source.
    let updated = daemon.tree().remove_entry(&root, &parent, name).await?;
    daemon
        .import_visible_root_with_tombstone_base(updated, Some(root))
        .await?;
    crate::paths::touch_provider_root_signal_in(config_dir)?;
    Ok(item)
}

/// Restore to the original path, requiring an available parent and an empty
/// destination. A saved intent allows recovery after the destination commits.
pub async fn restore(config_dir: &Path, id: &str, now: i64) -> Result<()> {
    ensure!(valid_id(id), "Invalid Trash item ID");
    ensure!(now >= 0, "Invalid Trash restoration time");
    let _lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut daemon = open(config_dir)?;
    let state = state(&daemon)?;
    let mut snapshot = read_heads(&daemon, state).await?;
    let intent_path = config_dir
        .join("trash-restore")
        .join(state.root_scope_id())
        .join(id);
    if snapshot.was_removed(id) {
        remove_intent(&intent_path)?;
        return Ok(());
    }
    let stored = snapshot
        .items
        .get(id)
        .context("Trash item is unavailable")?;
    let intent = RestoreIntent {
        item: stored.item.clone(),
        content_cid: entry_cid(&stored.content).to_string(),
    };
    let resuming = match std::fs::read(&intent_path) {
        Ok(bytes) => {
            let previous: RestoreIntent = serde_json::from_slice(&bytes)?;
            ensure!(
                previous == intent,
                "Trash item changed after restoration began"
            );
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    ensure!(
        resuming || !stored.item.expired(now),
        "Trash item has expired"
    );
    let root = personal_root(&daemon, &stored.item.path).await?;
    let (name, parent) = split_path(&stored.item.path)?;
    let dir = resolve_parent(daemon.tree(), &root, &parent).await?;
    verify_content(daemon.tree(), &stored.content).await?;
    if let Some(existing) = lookup(daemon.tree(), &dir, name).await? {
        ensure!(
            resuming && same_entry(&existing, &stored.content),
            "A different entry already exists at the restore path"
        );
    } else {
        ensure!(!stored.item.expired(now), "Trash item has expired");
        if !resuming {
            crate::atomic_file::atomic_write_private(&intent_path, &serde_json::to_vec(&intent)?)?;
        }
        let updated = daemon
            .tree()
            .set_entry_with_meta(
                &root,
                &parent,
                name,
                &entry_cid(&stored.content),
                stored.content.size,
                stored.content.link_type,
                stored.content.meta.clone(),
            )
            .await?;
        daemon
            .import_visible_root_with_tombstone_base(updated, Some(root))
            .await?;
        let saved = personal_root(&daemon, &stored.item.path).await?;
        let parent = resolve_parent(daemon.tree(), &saved, &parent).await?;
        let destination = lookup(daemon.tree(), &parent, name).await?;
        ensure!(
            destination.is_some_and(|entry| same_entry(&entry, &stored.content)),
            "The Drive did not retain the restored entry"
        );
    }
    crate::paths::touch_provider_root_signal_in(config_dir)?;
    snapshot
        .mark_removed(
            daemon.tree(),
            &TrashRemoval {
                v: 1,
                id: id.into(),
                removed_at: now,
            },
        )
        .await?;
    snapshot.expire(daemon.tree(), now).await?;
    let updated = snapshot.write(daemon.tree()).await?;
    daemon
        .import_visible_root_for_drive(DRIVE_ID, updated)
        .await?;
    remove_intent(&intent_path)
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct RestoreIntent {
    item: TrashItem,
    content_cid: String,
}

fn remove_intent(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn open(config_dir: &Path) -> Result<Daemon> {
    let config = crate::AppConfig::load_or_default(crate::paths::config_path_in(config_dir))?;
    Ok(Daemon::open_with_config(config_dir, config)?)
}

fn state(daemon: &Daemon) -> Result<&ProfileState> {
    let state = daemon
        .config()
        .profile
        .as_ref()
        .context("No Drive account")?;
    ensure!(
        can_write(state, &state.app_key_pubkey),
        "This device needs signed account approval"
    );
    Ok(state)
}

/// Save an owner's shared file under the request nonce while the caller
/// holds the config lock. Existing removal markers prevent resurrection.
pub(crate) async fn stage_share_return(
    daemon: &mut Daemon,
    item: &TrashItem,
    content: &TreeEntry,
) -> Result<()> {
    let state = state(daemon)?;
    item.validate(state.profile_id, &item.id)?;
    let mut snapshot = read_heads(daemon, state).await?;
    if let Some(stored) = snapshot.items.get(&item.id) {
        ensure!(
            &stored.item == item && same_entry(&stored.content, content),
            "Trash already contains a different payload for this return"
        );
        if let Some(current) = daemon
            .config()
            .drive(DRIVE_ID)
            .and_then(|drive| drive.app_key_roots.get(&state.app_key_pubkey))
            && verify_share_return(
                daemon.tree(),
                &Cid::parse(&current.root_cid)?,
                item,
                content,
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
    } else {
        snapshot
            .insert(daemon.tree(), item.clone(), content.clone())
            .await?;
    }
    let root = snapshot.write(daemon.tree()).await?;
    daemon.import_visible_root_for_drive(DRIVE_ID, root).await?;
    Ok(())
}

pub(crate) async fn verify_share_return<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    item: &TrashItem,
    content: &TreeEntry,
) -> Result<()> {
    let snapshot = TrashTree::read(tree, item.account, std::slice::from_ref(root)).await?;
    let stored = snapshot
        .items
        .get(&item.id)
        .context("Returned file is missing from private Trash")?;
    ensure!(
        &stored.item == item && same_entry(&stored.content, content),
        "Private Trash contains a different return"
    );
    verify_content(tree, &stored.content).await
}

/// Hold the config mutation lock while preparing the causal handoff root.
pub(crate) async fn materialize_for_admission(daemon: &mut Daemon) -> Result<()> {
    if daemon.config().drive(DRIVE_ID).is_none() {
        return Ok(());
    }
    let state = state(daemon)?;
    let snapshot = read_heads(daemon, state).await?;
    let root = snapshot.write(daemon.tree()).await?;
    let current = daemon
        .config()
        .drive(DRIVE_ID)
        .and_then(|drive| drive.app_key_roots.get(&state.app_key_pubkey));
    if let Some(current) = current
        && !current.local_only
        && crate::indexer::filter_ignored_entries_from_root(
            daemon.tree(),
            &Cid::parse(&current.root_cid)?,
        )
        .await?
            == root
    {
        return Ok(());
    }
    daemon.import_visible_root_for_drive(DRIVE_ID, root).await?;
    Ok(())
}

async fn read_heads(daemon: &Daemon, state: &ProfileState) -> Result<TrashTree> {
    let mut heads = Vec::new();
    if let Some(drive) = daemon.config().drive(DRIVE_ID) {
        ensure!(
            drive.root_scope_id == state.root_scope_id(),
            "Trash belongs to a different account"
        );
        let roster = state.profile_projection();
        for (author, root) in &drive.app_key_roots {
            if roster
                .active_facets
                .get(author)
                .is_some_and(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
            {
                heads.push(Cid::parse(&root.root_cid)?);
            }
        }
    }
    TrashTree::read(daemon.tree(), state.profile_id, &heads).await
}

async fn personal_root(daemon: &Daemon, components: &[String]) -> Result<Cid> {
    let config = daemon.config();
    let state = state(daemon)?;
    let path = components.join("/");
    for folder in &config.shared_folders {
        if folder.owner_profile_id == state.profile_id {
            ensure!(
                !overlaps(&path, &folder.source_path),
                "Shared folder items are outside private Trash"
            );
        }
    }
    for shortcut in &config.share_shortcuts {
        ensure!(
            !overlaps(&path, &shortcut.path),
            "Shared folder items are outside private Trash"
        );
    }
    let merged = primary_merged_view(daemon.tree(), config).await?;
    ensure!(
        !merged
            .shared_mounts
            .iter()
            .any(|mount| overlaps(&path, &mount.path)),
        "Shared folder items are outside private Trash"
    );
    Ok(
        primary_merged_root_from_view(daemon.tree(), config, &merged)
            .await?
            .root_cid,
    )
}

fn overlaps(a: &str, b: &str) -> bool {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    a == b
        || a.strip_prefix(&b).is_some_and(|rest| rest.starts_with('/'))
        || b.strip_prefix(&a).is_some_and(|rest| rest.starts_with('/'))
}

fn split_path(path: &[String]) -> Result<(&str, Vec<&str>)> {
    ensure!(
        path.iter().all(|part| valid_component(part)),
        "Invalid Drive path"
    );
    let (name, parent) = path.split_last().context("Missing Drive path")?;
    Ok((name, parent.iter().map(String::as_str).collect()))
}

async fn resolve_parent<S: Store>(tree: &HashTree<S>, root: &Cid, path: &[&str]) -> Result<Cid> {
    let mut dir = root.clone();
    for part in path {
        let entry = lookup(tree, &dir, part)
            .await?
            .context("The original folder is unavailable")?;
        ensure!(
            entry.link_type == LinkType::Dir,
            "The original folder path contains a file"
        );
        dir = entry_cid(&entry);
    }
    Ok(dir)
}

async fn lookup<S: Store>(tree: &HashTree<S>, dir: &Cid, name: &str) -> Result<Option<TreeEntry>> {
    let mut matched = None;
    let folded = name.to_lowercase();
    for entry in tree.list_directory_required(dir).await? {
        if entry.name.to_lowercase() == folded {
            ensure!(
                entry.name == name && matched.is_none(),
                "The Drive path conflicts with a different spelling"
            );
            matched = Some(entry);
        }
    }
    Ok(matched)
}

async fn verify_content<S: Store>(tree: &HashTree<S>, entry: &TreeEntry) -> Result<()> {
    crate::block_sync::verify_reachable_blocks(tree, &entry_cid(entry))
        .await
        .context("Verifying Trash payload blocks")?;
    Ok(())
}
