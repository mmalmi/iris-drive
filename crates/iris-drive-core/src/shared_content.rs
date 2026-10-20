//! Shared folders have their own causal roots. Never publish a personal root,
//! its ancestor history, or a recipient's same-named local folder as a share.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail};
use hashtree_core::{Cid, HashTree, LinkType, Store};

use crate::{
    AppConfig, AppKeyRootRef, Daemon, PRIMARY_DRIVE_ID, SharedFolder,
    config_lock::ConfigMutationLock,
    paths::config_path_in,
    root_meta::{DriveRootMeta, RootObservation, RootParent},
};

/// Enforce roles before a provider stages or acknowledges a local write.
pub fn ensure_shared_path_writable(config: &AppConfig, path: &str) -> Result<()> {
    let Some(account) = config.profile.as_ref() else {
        return Ok(());
    };
    for folder in &config.shared_folders {
        let mut paths = vec![crate::sharing::shared_with_me_mount_path(
            folder,
            &config.shared_folders,
        )];
        if folder.owner_profile_id == account.profile_id {
            paths.push(folder.source_path.clone());
        }
        paths.extend(
            config
                .share_shortcuts
                .iter()
                .filter(|s| s.share_id == folder.share_id)
                .map(|s| s.path.clone()),
        );
        if paths.iter().any(|mount| {
            path == mount
                || path.starts_with(&format!("{mount}/"))
                || mount.starts_with(&format!("{path}/"))
        }) && !crate::shared_folder_app_key_can_write_roots(folder, &account.app_key_pubkey)
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
    app_key: &str,
    edited: Cid,
    base: &Cid,
) -> Result<()> {
    if !crate::shared_folder_app_key_can_write_roots(folder, app_key) {
        bail!("This shared folder is read-only");
    }
    let previous = folder
        .app_key_roots
        .get(app_key)
        .map(|r| Cid::parse(&r.root_cid))
        .transpose()?;
    let current = crate::projection::shared_folder_merged_root(tree, folder).await?;
    let delta = crate::indexer::local_visible_root_for_mount_import(
        tree,
        &edited,
        previous.as_ref(),
        base,
        Some(&current.root_cid),
        None,
    )
    .await?;
    let meta = share_root_meta(folder, app_key);
    let root =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements(
            tree,
            delta.root,
            previous.as_ref(),
            Some(base),
            meta.created_at,
            Some(&meta),
            Some(&delta.tombstone_paths),
            Some(&delta.kind_replacement_paths),
        )
        .await?;
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
    config: &mut AppConfig,
    imported: &Cid,
    previous_import: Option<&Cid>,
) -> Result<()> {
    let Some(profile) = config.profile.as_ref() else {
        return Ok(());
    };
    let owner = profile.profile_id;
    let app_key = profile.app_key_pubkey.clone();
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
            write_shared_folder(tree, folder, &app_key, root, &base).await?;
        }
    }
    Ok(())
}

pub(crate) async fn route_shared_edits<S: Store>(
    tree: &HashTree<S>,
    config: &mut AppConfig,
    mut edited: Cid,
    base: Option<Cid>,
    local_only: bool,
) -> Result<(Cid, Option<Cid>)> {
    if config.shared_folders.is_empty() {
        return Ok((edited, base));
    }
    seed_shared_roots(tree, config).await?;
    let projection = crate::primary_merged_view(tree, config).await?;
    let current = crate::primary_merged_root_from_view(tree, config, &projection)
        .await?
        .root_cid;
    let mut base = base.unwrap_or(current);
    let app_key = config
        .profile
        .as_ref()
        .context("Profile required")?
        .app_key_pubkey
        .clone();
    let mut changes = BTreeMap::new();
    for mount in &projection.shared_mounts {
        let before = tree.resolve(&base, &mount.path).await?;
        let after = tree.resolve(&edited, &mount.path).await?;
        if !local_only && before != after {
            // Removing/renaming the mounted folder itself needs an explicit
            // shortcut action; it must not silently delete everyone's files.
            let after = after.context("Remove shared folders using the sharing controls")?;
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
    for (id, (changed, before)) in changes {
        let folder = config
            .shared_folders
            .iter_mut()
            .find(|folder| folder.share_id == id)
            .context("Share missing")?;
        write_shared_folder(tree, folder, &app_key, changed, &before).await?;
    }
    for mount in &projection.shared_mounts {
        edited = remove_mount(tree, edited, &mount.path).await?;
        base = remove_mount(tree, base, &mount.path).await?;
    }
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

#[cfg(test)]
mod tests;
