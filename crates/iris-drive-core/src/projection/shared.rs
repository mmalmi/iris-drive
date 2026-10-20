use super::{
    AppConfig, BTreeMap, Cid, Drive, HashTree, HashTreeError, MergedEntry, PRIMARY_DRIVE_ID,
    PrimaryMergedRoot, PrimaryMergedView, ProjectionError, Store, drive_merged_root_from_view,
    drive_merged_view, merge_app_key_pubkeys, walk_app_key_tree,
};
use crate::{NostrIdentityId, SharedFolder};

#[derive(Debug, Clone, PartialEq)]
pub struct SharedMount {
    pub share_id: NostrIdentityId,
    pub path: String,
    pub target_path: String,
    pub root_cid: Cid,
}

pub(super) fn under(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub async fn shared_folder_merged_root<S: Store>(
    tree: &HashTree<S>,
    folder: &SharedFolder,
) -> Result<PrimaryMergedRoot, ProjectionError> {
    let mut drive = Drive::primary(folder.share_id.to_string());
    drive.app_key_roots = folder.app_key_roots.clone();
    let writers = crate::shared_folder_authorized_writer_pubkeys(folder);
    let merged = drive_merged_view(tree, &drive, &writers).await?;
    drive_merged_root_from_view(tree, &drive, &writers, &merged).await
}

pub(super) async fn add_shared_mounts<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
    merged: &mut PrimaryMergedView,
) -> Result<(), ProjectionError> {
    let account = config.profile.as_ref().ok_or(ProjectionError::NoAccount)?;
    // Capture the personal projection before overlaying any shares. A new
    // owner's seed must never include other shares or private ancestor history.
    let personal = merged.clone();
    for folder in &config.shared_folders {
        if folder
            .active_member_for_app_key(&account.app_key_pubkey)
            .is_none()
        {
            continue;
        }
        let owner = folder.owner_profile_id == account.profile_id;
        let root = if folder.app_key_roots.is_empty() {
            if !owner {
                continue;
            }
            let drive = config
                .drive(PRIMARY_DRIVE_ID)
                .ok_or(ProjectionError::PrimaryDriveMissing)?;
            let keys = merge_app_key_pubkeys(account, drive);
            let root = drive_merged_root_from_view(tree, drive, &keys, &personal).await?;
            let Some(root) = tree.resolve(&root.root_cid, &folder.source_path).await? else {
                continue;
            };
            root
        } else {
            shared_folder_merged_root(tree, folder).await?.root_cid
        };
        let mut paths = BTreeMap::new();
        if !owner {
            paths.insert(
                crate::sharing::shared_with_me_mount_path(folder, &config.shared_folders),
                String::new(),
            );
        }
        if owner {
            paths.insert(folder.source_path.clone(), String::new());
        }
        for shortcut in config
            .share_shortcuts
            .iter()
            .filter(|s| s.share_id == folder.share_id)
        {
            paths.insert(shortcut.path.clone(), shortcut.target_path.clone());
        }
        for (path, target_path) in paths {
            let mounted = if target_path.is_empty() {
                root.clone()
            } else {
                let Some(subdir) = tree.resolve(&root, &target_path).await? else {
                    continue;
                };
                subdir
            };
            if !tree.is_dir(&mounted).await? {
                return Err(HashTreeError::PathNotFound(path).into());
            }
            let (files, _) = walk_app_key_tree(tree, &mounted).await?;
            merged.view.files.retain(|entry| !under(&entry.path, &path));
            for file in files {
                merged.view.files.push(MergedEntry {
                    path: format!("{path}/{}", file.path),
                    source_path: Some(format!("{}/{}", folder.source_path, file.path)),
                    hash: file.hash,
                    size: file.size,
                    whole_file_hash: file.whole_file_hash,
                    modified_at: file.modified_at,
                    source_app_key_pubkey: account.app_key_pubkey.clone(),
                    published_at: folder.access.updated_at,
                });
            }
            merged.shared_mounts.push(SharedMount {
                share_id: folder.share_id,
                path,
                target_path,
                root_cid: mounted,
            });
        }
    }
    merged.view.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(())
}
