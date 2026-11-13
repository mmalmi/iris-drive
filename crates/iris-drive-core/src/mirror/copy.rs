//! Prepare file data outside the shared writer lock, then overlay it on the latest root.
use super::{
    Checkpoint, Mirror, Report, ensure_private, find, owner_dir, read, save,
    source::{self, Scan, Verified},
};
use crate::{
    Daemon,
    config_lock::ConfigMutationLock,
    paths::{provider_root_staging_path_in, touch_provider_root_signal_in},
};
use anyhow::{Result, ensure};
use hashtree_core::{Cid, DirEntry, HashTree, LinkType, Store};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Default)]
struct Overlay {
    dirs: BTreeMap<String, Overlay>,
    files: BTreeMap<String, (String, Verified)>,
}

impl Overlay {
    fn directory(&mut self, path: &str) -> &mut Self {
        let mut dir = self;
        for segment in path.split('/').filter(|part| !part.is_empty()) {
            dir = dir.dirs.entry(segment.to_owned()).or_default();
        }
        dir
    }

    fn from_scan(destination: &str, scan: Scan) -> Self {
        let mut root = Self::default();
        let dest = root.directory(destination);
        for dir in scan.dirs {
            dest.directory(&dir);
        }
        for (path, file) in scan.files {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
            dest.directory(parent)
                .files
                .insert(name.to_owned(), (path, file));
        }
        root
    }
}

pub(super) async fn run(
    config: &Path,
    owner: &Path,
    dir: &Path,
    mirror: &Mirror,
    report: &mut Report,
) -> Result<()> {
    let mut checkpoint: Checkpoint = read(&dir.join("files.json"))?.unwrap_or_default();
    let daemon = Daemon::open(config)?;
    source::validate_location(config, &mirror.source)?;
    let scan = source::scan(daemon.tree(), &mirror.source, &checkpoint, report).await?;
    let overlay = Overlay::from_scan(&mirror.destination, scan);
    let config_lock = ConfigMutationLock::acquire(config).await?;
    ensure!(
        owner_dir(config)? == owner && find(owner, &mirror.destination)?.id == mirror.id,
        "Mirror registration changed"
    );
    ensure!(
        !provider_root_staging_path_in(config).try_exists()?,
        "A Finder change is pending. Retry after Iris has saved it"
    );
    let mut daemon = Daemon::open(config)?;
    ensure_private(daemon.config(), &mirror.destination)?;
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config()).await?;
    // A failed root commit cannot advance the checkpoint or count proposed writes as copied.
    let mut copied = 0;
    let mut checkpoint_changed = false;
    let root = apply(
        daemon.tree(),
        Some(&visible.root_cid),
        overlay,
        "",
        &mut checkpoint,
        report,
        &mut copied,
        &mut checkpoint_changed,
    )
    .await?;
    if root != visible.root_cid {
        let deletions = BTreeSet::new();
        daemon
            .import_visible_root_with_tombstone_base_and_paths(
                root,
                Some(visible.root_cid),
                Some(&deletions),
            )
            .await?;
        touch_provider_root_signal_in(config)?;
    }
    drop(config_lock);
    report.copied = copied;
    if checkpoint_changed {
        save(&dir.join("files.json"), &checkpoint)?;
    }
    Ok(())
}

fn apply<'a, S: Store>(
    tree: &'a HashTree<S>,
    base: Option<&'a Cid>,
    overlay: Overlay,
    path: &'a str,
    checkpoint: &'a mut Checkpoint,
    report: &'a mut Report,
    copied: &'a mut u64,
    checkpoint_changed: &'a mut bool,
) -> futures::future::BoxFuture<'a, Result<Cid>> {
    Box::pin(async move {
        let mut entries = BTreeMap::new();
        if let Some(base) = base {
            for e in tree.list_directory_required(base).await? {
                entries.insert(
                    e.name.clone(),
                    DirEntry {
                        name: e.name,
                        hash: e.hash,
                        key: e.key,
                        size: e.size,
                        link_type: e.link_type,
                        meta: e.meta,
                    },
                );
            }
        }
        let mut changed = false;
        for (name, directory) in overlay.dirs {
            let child = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let existing = entries.get(&name);
            if existing.is_some_and(|e| !e.link_type.is_directory_like()) {
                report.issue(&child, "An Iris file already uses this folder name");
                continue;
            }
            let old = existing.map(|e| Cid {
                hash: e.hash,
                key: e.key,
            });
            let new = apply(
                tree,
                old.as_ref(),
                directory,
                &child,
                checkpoint,
                report,
                copied,
                checkpoint_changed,
            )
            .await?;
            if old.as_ref() != Some(&new) {
                let mut e = DirEntry::from_cid(&name, &new);
                e.link_type = LinkType::Dir;
                e.meta = existing.and_then(|e| e.meta.clone());
                entries.insert(name, e);
                changed = true;
            }
        }
        for (name, (relative, file)) in overlay.files {
            let cid = Cid::parse(&file.cid)?;
            let existing = entries.get(&name);
            if let Some(existing) = existing {
                let current = Cid {
                    hash: existing.hash,
                    key: existing.key,
                };
                if !existing.link_type.is_directory_like() && current == cid {
                    report.unchanged += 1;
                    if checkpoint.files.get(&relative) != Some(&file) {
                        checkpoint.files.insert(relative, file);
                        *checkpoint_changed = true;
                    }
                    continue;
                }
                if existing.link_type.is_directory_like()
                    || checkpoint
                        .files
                        .get(&relative)
                        .is_none_or(|old| old.cid != current.to_string())
                {
                    report.issue(
                        &relative,
                        "A different Iris item already exists. Move or rename it to resume",
                    );
                    continue;
                }
            }
            let mut entry = DirEntry::from_cid(&name, &cid)
                .with_size(file.size())
                .with_meta(crate::indexer::file_entry_meta(
                    &file.digest,
                    file.modified(),
                ));
            entry.link_type = if file.size() > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
                LinkType::File
            } else {
                LinkType::Blob
            };
            entries.insert(name, entry);
            checkpoint.files.insert(relative, file);
            *checkpoint_changed = true;
            *copied += 1;
            changed = true;
        }
        if let Some(base) = base.filter(|_| !changed) {
            return Ok(base.clone());
        }
        Ok(tree.put_directory(entries.into_values().collect()).await?)
    })
}
