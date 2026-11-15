//! Where a shared folder sits in a member's own Drive. Moving or deleting
//! the folder itself on disk changes only that place, as with a Google Drive
//! shortcut; an owner's shared folder moves with them and is never deleted
//! while people still share it.
use anyhow::{Context, Result, bail};
use hashtree_core::{Cid, HashTree, LinkType, Store};

use crate::AppConfig;
use crate::projection::SharedMount;

const MAX_DEPTH: usize = 64;

/// Where the folder `target` went, if the edit moved it: a directory with the
/// same content that `edited` has and `base` does not. Unchanged directories
/// are skipped by their content address, so this follows only the edit.
pub(super) async fn moved_dir<S: Store>(
    tree: &HashTree<S>,
    base: &Cid,
    edited: &Cid,
    target: &Cid,
) -> Result<Option<String>> {
    let mut pending = vec![(String::new(), Some(base.clone()), edited.clone(), 0)];
    while let Some((path, base_dir, edited_dir, depth)) = pending.pop() {
        let before = match &base_dir {
            Some(dir) => tree.list_directory(dir).await?,
            None => Vec::new(),
        };
        for entry in tree.list_directory(&edited_dir).await? {
            if entry.link_type != LinkType::Dir || entry.name == crate::merge::META_DIR {
                continue;
            }
            let cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            let was = before
                .iter()
                .find(|other| other.name == entry.name && other.link_type == LinkType::Dir)
                .map(|other| Cid {
                    hash: other.hash,
                    key: other.key,
                });
            if was.as_ref() == Some(&cid) {
                continue;
            }
            let child = if path.is_empty() {
                entry.name.clone()
            } else {
                format!("{path}/{}", entry.name)
            };
            if &cid == target {
                return Ok(Some(child));
            }
            if depth < MAX_DEPTH {
                pending.push((child, was, cid, depth + 1));
            }
        }
    }
    Ok(None)
}

/// Record that the folder mounted at `mount` moved to `to`, or left My Drive.
pub(super) fn apply(
    config: &mut AppConfig,
    mount: &SharedMount,
    to: Option<&str>,
    explicit_base: bool,
) -> Result<()> {
    let profile = config
        .profile
        .as_ref()
        .context("Profile required")?
        .profile_id;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == mount.share_id)
        .context("Share missing")?;
    let owned_here = folder.owner_profile_id == profile && folder.source_path == mount.path;
    let is_shortcut =
        |s: &crate::ShareShortcut| s.share_id == mount.share_id && s.path == mount.path;
    match to {
        Some(to) => {
            if owned_here {
                to.clone_into(&mut folder.source_path);
            }
            for shortcut in config.share_shortcuts.iter_mut().filter(|s| is_shortcut(s)) {
                to.clone_into(&mut shortcut.path);
            }
        }
        None if owned_here => bail!("Stop sharing this folder before deleting it"),
        None if !explicit_base => bail!("Remove shared folders using the sharing controls"),
        None => config.share_shortcuts.retain(|s| !is_shortcut(s)),
    }
    Ok(())
}
