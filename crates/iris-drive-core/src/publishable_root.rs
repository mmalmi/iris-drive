//! The latest root of this `AppKey` that may be published.
//!
//! Background convergence records merged views as local-only roots: another
//! `AppKey` keeps authorship of its own contribution. The root this device
//! last published is then the nearest ancestor that is not local-only.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use hashtree_core::Cid;

use crate::{AppKeyRootRef, Daemon, Drive, ProfileState};

/// This `AppKey`'s current root if publishable, else its nearest published
/// ancestor.
pub async fn publishable_app_key_root(
    config_dir: &Path,
    drive: &Drive,
    state: &ProfileState,
) -> Result<Option<AppKeyRootRef>> {
    let Some(root) = drive.app_key_roots.get(&state.app_key_pubkey).cloned() else {
        return Ok(None);
    };
    if !root.local_only {
        return Ok(Some(root));
    }
    publishable_parent_root(config_dir, state, root).await
}

/// The nearest ancestor of `root` by this `AppKey` that is not local-only.
pub async fn publishable_parent_root(
    config_dir: &Path,
    state: &ProfileState,
    mut root: AppKeyRootRef,
) -> Result<Option<AppKeyRootRef>> {
    let daemon = Daemon::open(config_dir).context("opening daemon for publishable root lookup")?;
    let mut seen = BTreeSet::new();
    for _ in 0..32 {
        if !seen.insert(root.root_cid.clone()) {
            return Ok(None);
        }
        let cid = Cid::parse(&root.root_cid)
            .with_context(|| format!("parsing root cid {}", root.root_cid))?;
        let Some(meta) = crate::indexer::read_root_meta(daemon.tree(), &cid)
            .await
            .with_context(|| format!("reading root metadata for {}", root.root_cid))?
        else {
            return Ok(None);
        };
        let Some(parent) = meta
            .parents
            .iter()
            .find(|parent| parent.app_key_pubkey == state.app_key_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid = Cid::parse(&parent.root_cid)
            .with_context(|| format!("parsing parent root cid {}", parent.root_cid))?;
        let parent_root = match crate::indexer::read_root_meta(daemon.tree(), &parent_cid)
            .await
            .with_context(|| format!("reading parent root metadata for {}", parent.root_cid))?
        {
            Some(parent_meta) => AppKeyRootRef::from_meta(
                parent.root_cid.clone(),
                parent_meta.created_at,
                &parent_meta,
            ),
            None => AppKeyRootRef::legacy(
                parent.root_cid.clone(),
                root.published_at,
                root.dck_generation,
            ),
        };
        if !parent_root.local_only {
            return Ok(Some(parent_root));
        }
        root = parent_root;
    }
    Ok(None)
}
