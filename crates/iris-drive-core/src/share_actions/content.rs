use super::*;
use crate::{AppKeyRootRef, Daemon};

/// Production share actions prepare folder content and return short,
/// remotely retrievable invite links. The synchronous reducer remains usable
/// for pure membership operations and deterministic protocol tests.
pub async fn dispatch_share_action_with_content(
    config_dir: &Path,
    mut action: ShareAction,
    now_seconds: i64,
) -> Result<ShareActionResult> {
    if let ShareAction::AcceptShareInvite { invite } = &mut action {
        *invite = crate::share_links::resolve_share_invite(config_dir, invite).await?;
    }
    let transaction = crate::config_lock::ConfigMutationLock::acquire(config_dir).await?;
    let daemon = Daemon::open(config_dir)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if let ShareAction::CreateShare { source_path, .. } = &action {
        let profile_id = config
            .profile
            .as_ref()
            .context("Profile required")?
            .profile_id;
        anyhow::ensure!(
            !config.shared_folders.iter().any(|folder| {
                folder.owner_profile_id == profile_id
                    && (source_path == &folder.source_path
                        || source_path.starts_with(&format!("{}/", folder.source_path))
                        || folder.source_path.starts_with(&format!("{source_path}/")))
            }),
            "This folder overlaps an existing share. Manage that share's members instead."
        );
        let visible = crate::primary_merged_root(daemon.tree(), &config).await?;
        let root = daemon
            .tree()
            .resolve(&visible.root_cid, source_path)
            .await?
            .context("Choose an existing folder to share")?;
        anyhow::ensure!(
            daemon.tree().is_dir(&root).await?,
            "Only folders can be shared"
        );
    }
    let mut changed = crate::shared_content::seed_shared_roots(daemon.tree(), &mut config).await?;
    if let ShareAction::DeleteShare { share_id } = action {
        detach_share(config_dir, daemon.tree(), config, share_id).await?;
        let mut result = share_action_state(config_dir)?;
        result.share_id = Some(share_id);
        return Ok(result);
    }
    let checkpoint = match &action {
        ShareAction::RevokeShareMember { share_id, .. }
        | ShareAction::SetShareMemberRole { share_id, .. } => Some(*share_id),
        _ => None,
    };
    if let Some(id) = checkpoint {
        let app_key = config
            .profile
            .as_ref()
            .context("Profile required")?
            .app_key_pubkey
            .clone();
        let folder = config
            .shared_folders
            .iter_mut()
            .find(|folder| folder.share_id == id)
            .context("Share missing")?;
        anyhow::ensure!(
            crate::shared_folder_app_key_can_admin(folder, &app_key),
            "Only admins can change sharing permissions"
        );
        // Re-author the complete merged content before withdrawing another
        // writer. Future projection must not depend on a revoked writer root.
        let visible = crate::projection::shared_folder_merged_root(daemon.tree(), folder).await?;
        let meta = crate::shared_content::share_root_meta(folder, &app_key);
        let previous = folder
            .app_key_roots
            .get(&app_key)
            .map(|r| hashtree_core::Cid::parse(&r.root_cid))
            .transpose()?;
        let root = crate::indexer::layer_history_and_meta_on_root(
            daemon.tree(),
            visible.root_cid,
            previous.as_ref(),
            meta.created_at,
            Some(&meta),
        )
        .await?;
        folder.app_key_roots.insert(
            app_key,
            AppKeyRootRef::from_meta(root.to_string(), meta.created_at, &meta),
        );
        changed = true;
    }
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    let mut result = dispatch_share_action(config_dir, action, now_seconds)?;
    let mut saved = AppConfig::load_or_default(config_path_in(config_dir))?;
    if repair_shortcut_collisions(daemon.tree(), &mut saved).await? {
        saved.save(config_path_in(config_dir))?;
        result.shares = shared_folder_views(
            &saved.shared_folders,
            &saved.share_shortcuts,
            &saved
                .profile
                .as_ref()
                .context("Profile required")?
                .app_key_pubkey,
        );
        if let Some(shortcut) = result.shortcut.as_mut() {
            if let Some(updated) = saved
                .share_shortcuts
                .iter()
                .find(|item| item.share_id == shortcut.share_id)
            {
                *shortcut = updated.clone();
            }
        }
    }
    drop(transaction);
    if let Some(invite) = &result.last_share_invite {
        publish_share_content(config_dir, result.share_id.context("Share missing")?).await?;
        result.last_share_invite =
            Some(crate::share_links::publish_compact_invite(config_dir, invite).await?);
    }
    Ok(result)
}

pub fn dispatch_share_action_with_content_blocking(
    config_dir: &Path,
    action: ShareAction,
    now_seconds: i64,
) -> Result<ShareActionResult> {
    let config_dir = config_dir.to_path_buf();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(dispatch_share_action_with_content(
                &config_dir,
                action,
                now_seconds,
            ))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Share action failed"))?
}

async fn publish_share_content(config_dir: &Path, share_id: NostrIdentityId) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let folder = config.shared_folder(share_id).context("Share missing")?;
    let app_key = crate::AppKey::load(key_path_in(config_dir))?;
    let pubkey = app_key.keys().public_key().to_hex();
    let daemon = Daemon::open(config_dir)?;
    let blossom = crate::blossom_sync_client(app_key.keys().clone(), &config.blossom_servers);
    let root = folder
        .app_key_roots
        .get(&pubkey)
        .context("Shared folder is not ready")?;
    crate::blossom_sync::upload_tree(
        daemon.tree(),
        &hashtree_core::Cid::parse(&root.root_cid)?,
        &blossom,
    )
    .await?;
    let client = crate::relay_sync::connect(&config.relays).await?;
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let snapshot =
            crate::sign_share_access_snapshot(app_key.keys(), folder, folder.access.updated_at)?;
        crate::relay_sync::publish_share_access_snapshot(&client, &snapshot).await?;
        crate::relay_sync::publish_drive_root(
            &client,
            app_key.keys(),
            &share_id.to_string(),
            crate::PRIMARY_DRIVE_ID,
            &crate::shared_content::share_root_for_publish(folder, root),
            &crate::shared_folder_key_recipient_pubkeys(folder),
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    crate::relay_sync::shutdown_client(&client).await;
    outcome.context("Publishing shared folder timed out")?
}

async fn repair_shortcut_collisions<S: hashtree_core::Store>(
    tree: &hashtree_core::HashTree<S>,
    config: &mut AppConfig,
) -> Result<bool> {
    if config.share_shortcuts.is_empty() {
        return Ok(false);
    }
    let mut personal = config.clone();
    personal.shared_folders.clear();
    personal.share_shortcuts.clear();
    let root = crate::primary_merged_root(tree, &personal).await?.root_cid;
    let profile = config
        .profile
        .as_ref()
        .context("Profile required")?
        .profile_id;
    let mut changed = false;
    for index in 0..config.share_shortcuts.len() {
        let shortcut = &config.share_shortcuts[index];
        let Some(folder) = config.shared_folder(shortcut.share_id) else {
            continue;
        };
        if folder.owner_profile_id == profile && shortcut.path == folder.source_path {
            continue;
        }
        if tree.resolve(&root, &shortcut.path).await?.is_none() {
            continue;
        }
        let stem = format!("{} (shared)", shortcut.path);
        let mut candidate = stem.clone();
        for suffix in 2.. {
            if tree.resolve(&root, &candidate).await?.is_none()
                && !config
                    .share_shortcuts
                    .iter()
                    .any(|item| item.path == candidate)
            {
                break;
            }
            candidate = format!("{stem} {suffix}");
        }
        config.share_shortcuts[index].path = candidate;
        changed = true;
    }
    Ok(changed)
}

async fn detach_share<S: hashtree_core::Store>(
    config_dir: &Path,
    tree: &hashtree_core::HashTree<S>,
    mut config: AppConfig,
    share_id: NostrIdentityId,
) -> Result<()> {
    let folder = config
        .shared_folder(share_id)
        .context("Share missing")?
        .clone();
    let owner = config
        .profile
        .as_ref()
        .context("Profile required")?
        .profile_id
        == folder.owner_profile_id;
    let projection = crate::primary_merged_view(tree, &config).await?;
    let mut root = crate::primary_merged_root_from_view(tree, &config, &projection)
        .await?
        .root_cid;
    for mount in projection
        .shared_mounts
        .iter()
        .filter(|mount| mount.share_id == share_id)
    {
        if owner && mount.path == folder.source_path {
            continue;
        }
        let parts: Vec<_> = mount.path.split('/').collect();
        let (name, parent) = parts.split_last().context("Invalid shared folder path")?;
        root = tree.remove_entry(&root, parent, name).await?;
    }
    config.remove_shared_folder(share_id);
    config.remove_share_shortcuts_for_share(share_id);
    // Import with the share detached so its last merged content becomes an
    // ordinary owner folder. No previously accepted editor work disappears.
    Daemon::open_with_config(config_dir.to_path_buf(), config)?
        .import_visible_root(root)
        .await?;
    Ok(())
}
