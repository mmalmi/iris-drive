//! Shared folders have their own causal roots. Never publish a personal root,
//! its ancestor history, or a recipient's same-named local folder as a share.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail, ensure};
use hashtree_core::{Cid, HashTree, LinkType, Store};

use crate::shared_owner_return::markers::{self, RemovalMarker};
use crate::{
    AppConfig, AppKeyRootRef, Daemon, PRIMARY_DRIVE_ID, SharedFolder,
    config_lock::ConfigMutationLock,
    paths::config_path_in,
    root_meta::{DriveRootMeta, RootObservation, RootParent},
};

/// Enforce roles before a provider stages or acknowledges a local write.
/// Only writes inside a read-only folder are refused: where its shortcut
/// sits in My Drive is the member's own to move or remove.
pub fn ensure_shared_path_writable(config: &AppConfig, path: &str) -> Result<()> {
    let Some(account) = config.profile.as_ref() else {
        return Ok(());
    };
    for folder in &config.shared_folders {
        let owner = folder.owner_profile_id == account.profile_id;
        let inside = config
            .share_shortcuts
            .iter()
            .filter(|s| s.share_id == folder.share_id)
            .map(|s| s.path.as_str())
            .chain(owner.then_some(folder.source_path.as_str()))
            .any(|mount| path.starts_with(&format!("{mount}/")));
        if inside && !crate::shared_folder_app_key_can_write_roots(folder, &account.app_key_pubkey)
        {
            bail!("This shared folder is read-only");
        }
    }
    Ok(())
}

/// A membership update changes the root's recipient wraps even when its
/// bytes and causal sequence stay unchanged. Relays must retain that update.
#[must_use]
pub fn share_root_for_publish(folder: &SharedFolder, root: &AppKeyRootRef) -> AppKeyRootRef {
    let mut root = root.clone();
    root.published_at = root.published_at.max(folder.access.updated_at);
    root.dck_generation = folder
        .access
        .key_epochs
        .keys()
        .next_back()
        .copied()
        .unwrap_or(root.dck_generation);
    root
}

pub async fn prepare_shared_folder_roots(config_dir: &Path) -> Result<AppConfig> {
    let _lock = ConfigMutationLock::acquire(config_dir).await?;
    let daemon = Daemon::open(config_dir)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if seed_shared_roots(daemon.tree(), &mut config).await? {
        config.save(config_path_in(config_dir))?;
    }
    Ok(config)
}

pub(crate) async fn seed_shared_roots<S: Store>(
    tree: &HashTree<S>,
    config: &mut AppConfig,
) -> Result<bool> {
    let Some(account) = config.profile.as_ref() else {
        return Ok(false);
    };
    let app_key = account.app_key_pubkey.clone();
    let profile_id = account.profile_id;
    let pending = config
        .shared_folders
        .iter()
        .enumerate()
        .filter(|(_, folder)| {
            folder.owner_profile_id == profile_id
                && folder.app_key_roots.is_empty()
                && crate::sharing::shared_folder_created_by(folder, &app_key)
                && crate::shared_folder_app_key_can_write_roots(folder, &app_key)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(false);
    }
    let mut personal = config.clone();
    personal.shared_folders.clear();
    personal.share_shortcuts.clear();
    let visible = crate::primary_merged_root(tree, &personal).await?;
    for index in pending {
        let folder = &mut config.shared_folders[index];
        let root = tree
            .resolve(&visible.root_cid, &folder.source_path)
            .await?
            .with_context(|| format!("Shared folder does not exist: {}", folder.source_path))?;
        if !tree.is_dir(&root).await? {
            bail!("Only folders can be shared");
        }
        let meta = share_root_meta(folder, &app_key);
        let root = crate::indexer::layer_history_and_meta_on_root(
            tree,
            root,
            None,
            meta.created_at,
            Some(&meta),
        )
        .await?;
        folder.app_key_roots.insert(
            app_key.clone(),
            AppKeyRootRef::from_meta(root.to_string(), meta.created_at, &meta),
        );
    }
    Ok(true)
}

pub(crate) fn share_root_meta(folder: &SharedFolder, app_key: &str) -> DriveRootMeta {
    let previous = folder.app_key_roots.get(app_key);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    let writers = crate::shared_folder_authorized_writer_pubkeys(folder);
    DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: app_key.into(),
        app_key_seq: previous.map_or(1, |root| root.app_key_seq.saturating_add(1)),
        dck_generation: folder
            .access
            .key_epochs
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0),
        local_only: false,
        parents: previous
            .map(|root| RootParent {
                app_key_pubkey: app_key.into(),
                app_key_seq: root.app_key_seq,
                root_cid: root.root_cid.clone(),
            })
            .into_iter()
            .collect(),
        observed: folder
            .app_key_roots
            .iter()
            .filter(|(key, _)| writers.contains(key))
            .map(|(key, root)| {
                (
                    key.clone(),
                    RootObservation {
                        app_key_seq: root.app_key_seq,
                        root_cid: root.root_cid.clone(),
                    },
                )
            })
            .collect(),
        created_at: previous.map_or(now, |root| now.max(root.published_at.saturating_add(1))),
    }
}

/// Apply a provider edit to the folder's contribution using the same delta,
/// deletion and conflict metadata as personal Drive writes.
pub async fn write_shared_folder<S: Store>(
    tree: &HashTree<S>,
    folder: &mut SharedFolder,
    keys: &nostr_sdk::Keys,
    edited: Cid,
    base: &Cid,
) -> Result<()> {
    let app_key = keys.public_key().to_hex();
    if !crate::shared_folder_app_key_can_write_roots(folder, &app_key) {
        bail!("This shared folder is read-only");
    }
    let current = crate::projection::shared_folder_merged_root(tree, folder).await?;
    let prepared = crate::shared_owner_return::prepare_edit(
        tree,
        folder,
        keys,
        edited,
        base,
        &current.root_cid,
        false,
    )
    .await?;
    write_shared_folder_delta(
        tree,
        folder,
        &app_key,
        prepared.root,
        base,
        &current.root_cid,
        &prepared.allowed_removals,
    )
    .await
}

pub(crate) async fn write_shared_folder_delta<S: Store>(
    tree: &HashTree<S>,
    folder: &mut SharedFolder,
    app_key: &str,
    edited: Cid,
    base: &Cid,
    current: &Cid,
    allowed_removals: &std::collections::BTreeSet<String>,
) -> Result<()> {
    write_shared_folder_change(
        tree,
        folder,
        app_key,
        SharedFolderChange {
            edited,
            base,
            current,
            allowed_removals,
            marker: None,
        },
    )
    .await
}

pub(crate) struct SharedFolderChange<'a> {
    pub edited: Cid,
    pub base: &'a Cid,
    pub current: &'a Cid,
    pub allowed_removals: &'a std::collections::BTreeSet<String>,
    pub marker: Option<&'a RemovalMarker>,
}

pub(crate) async fn write_shared_folder_change<S: Store>(
    tree: &HashTree<S>,
    folder: &mut SharedFolder,
    app_key: &str,
    change: SharedFolderChange<'_>,
) -> Result<()> {
    let SharedFolderChange {
        edited,
        base,
        current,
        allowed_removals,
        marker,
    } = change;
    ensure!(
        crate::shared_folder_app_key_can_write_roots(folder, app_key),
        "This shared folder is read-only"
    );
    let previous = folder
        .app_key_roots
        .get(app_key)
        .map(|r| Cid::parse(&r.root_cid))
        .transpose()?;
    let delta = crate::indexer::local_visible_root_for_mount_import(
        tree,
        &edited,
        previous.as_ref(),
        base,
        Some(current),
        None,
    )
    .await?;
    ensure!(
        delta.tombstone_paths.is_subset(allowed_removals),
        "The shared file stays in the folder until its owner confirms it is saved"
    );
    let meta = share_root_meta(folder, app_key);
    let mut tombstones = delta.tombstone_paths;
    if let Some(marker) = marker {
        tombstones.remove(
            &crate::shared_owner_return::read_request(&marker.request)?
                .path
                .join("/"),
        );
    }
    let root =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements(
            tree,
            delta.root,
            previous.as_ref(),
            Some(base),
            meta.created_at,
            Some(&meta),
            Some(&tombstones),
            Some(&delta.kind_replacement_paths),
        )
        .await?;
    let root = markers::attach(tree, &root, folder, marker).await?;
    folder.app_key_roots.insert(
        app_key.into(),
        AppKeyRootRef::from_meta(root.to_string(), meta.created_at, &meta),
    );
    Ok(())
}

/// A source import replaces the owner's visible folder content. Recipient
/// mounts are independent of the imported personal directory.
pub(crate) async fn import_owned_shared_folders<S: Store>(
    tree: &HashTree<S>,
    config_dir: &Path,
    config: &mut AppConfig,
    imported: &Cid,
    previous_import: Option<&Cid>,
) -> Result<()> {
    let Some(profile) = config.profile.as_ref() else {
        return Ok(());
    };
    let owner = profile.profile_id;
    if !config
        .shared_folders
        .iter()
        .any(|folder| folder.owner_profile_id == owner)
    {
        return Ok(());
    }
    let device = crate::AppKey::load(crate::paths::key_path_in(config_dir))?;
    seed_shared_roots(tree, config).await?;
    for folder in config
        .shared_folders
        .iter_mut()
        .filter(|folder| folder.owner_profile_id == owner)
    {
        let root = tree.resolve(imported, &folder.source_path).await?.context(
            "An imported shared folder is missing. Stop sharing it before removing the folder.",
        )?;
        anyhow::ensure!(
            tree.is_dir(&root).await?,
            "A shared folder must remain a folder"
        );
        let imported_base = if let Some(previous) = previous_import {
            tree.resolve(previous, &folder.source_path).await?
        } else {
            None
        };
        let base = match imported_base {
            Some(base) => base,
            None => {
                crate::projection::shared_folder_merged_root(tree, folder)
                    .await?
                    .root_cid
            }
        };
        if root != base {
            write_shared_folder(tree, folder, device.keys(), root, &base).await?;
        }
    }
    Ok(())
}

pub(crate) async fn route_shared_edits<S: Store>(
    tree: &HashTree<S>,
    config_dir: &Path,
    persisted: &mut AppConfig,
    mut edited: Cid,
    base: Option<Cid>,
    local_only: bool,
) -> Result<(Cid, Option<Cid>)> {
    if persisted.shared_folders.is_empty() {
        return Ok((edited, base));
    }
    let mut transaction = persisted.clone();
    let config = &mut transaction;
    seed_shared_roots(tree, config).await?;
    let projection = crate::primary_merged_view(tree, config).await?;
    let current = crate::primary_merged_root_from_view(tree, config, &projection)
        .await?
        .root_cid;
    let app_key = config
        .profile
        .as_ref()
        .context("Profile required")?
        .app_key_pubkey
        .clone();
    let explicit_base = base.is_some();
    let mut base = base.unwrap_or(current);
    let mut changes = BTreeMap::new();
    let mut moved = Vec::new();
    for mount in &projection.shared_mounts {
        let before = tree.resolve(&base, &mount.path).await?;
        let after = tree.resolve(&edited, &mount.path).await?;
        if !local_only && before != after {
            let Some(after) = after else {
                // The mounted folder itself is gone from where it was: its
                // place in My Drive changed, never everyone's files.
                let to = match &before {
                    Some(before) if explicit_base => {
                        placement::moved_dir(tree, &base, &edited, before).await?
                    }
                    _ => None,
                };
                placement::apply(config, mount, to.as_deref(), explicit_base)?;
                moved.extend(to);
                continue;
            };
            if !tree.is_dir(&after).await? {
                bail!("A shared folder must remain a folder");
            }
            let folder = config
                .shared_folder(mount.share_id)
                .context("Share missing")?;
            let full = crate::projection::shared_folder_merged_root(tree, folder)
                .await?
                .root_cid;
            let edited_root = if mount.target_path.is_empty() {
                after
            } else {
                let parts: Vec<_> = mount.target_path.split('/').collect();
                let (name, parent) = parts.split_last().context("Invalid shortcut target")?;
                tree.set_entry(&full, parent, name, &after, 0, LinkType::Dir)
                    .await?
            };
            if let Some((existing, _)) = changes.get(&mount.share_id)
                && existing != &edited_root
            {
                bail!("Conflicting edits through shared folder shortcuts");
            }
            let before = if mount.target_path.is_empty() {
                before.unwrap_or(full)
            } else {
                full
            };
            changes.insert(mount.share_id, (edited_root, before));
        }
    }
    // Validate every permission before committing any shared contribution.
    for id in changes.keys() {
        if !crate::shared_folder_app_key_can_write_roots(
            config.shared_folder(*id).context("Share missing")?,
            &app_key,
        ) {
            bail!("This shared folder is read-only");
        }
    }
    if !changes.is_empty() {
        let device = crate::AppKey::load(crate::paths::key_path_in(config_dir))?;
        let mut updated = Vec::new();
        let mut requests = Vec::new();
        for (id, (changed, before)) in changes {
            let mut folder = config.shared_folder(id).context("Share missing")?.clone();
            let live = crate::projection::shared_folder_merged_root(tree, &folder)
                .await?
                .root_cid;
            let prepared = crate::shared_owner_return::prepare_edit(
                tree,
                &folder,
                device.keys(),
                changed,
                &before,
                &live,
                true,
            )
            .await?;
            write_shared_folder_delta(
                tree,
                &mut folder,
                &app_key,
                prepared.root,
                &before,
                &live,
                &prepared.allowed_removals,
            )
            .await?;
            requests.extend(prepared.requests);
            updated.push(folder);
        }
        crate::shared_owner_return::remember_requests(config_dir, requests)?;
        for folder in updated {
            config.upsert_shared_folder(folder);
        }
    }
    for path in projection
        .shared_mounts
        .iter()
        .map(|mount| &mount.path)
        .chain(&moved)
    {
        edited = remove_mount(tree, edited, path).await?;
        base = remove_mount(tree, base, path).await?;
    }
    *persisted = transaction;
    Ok((edited, Some(base)))
}

async fn remove_mount<S: Store>(tree: &HashTree<S>, root: Cid, path: &str) -> Result<Cid> {
    if tree.resolve(&root, path).await?.is_none() {
        return Ok(root);
    }
    let parts: Vec<_> = path.split('/').collect();
    let (name, parent) = parts.split_last().context("Invalid shared folder path")?;
    Ok(tree.remove_entry(&root, parent, name).await?)
}

#[path = "shared_content/placement.rs"]
mod placement;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "shared_content/placement_tests.rs"]
mod placement_tests;
