use std::io::Read;
use std::path::Path;

use anyhow::{Context, ensure};
use hashtree_core::{Cid, HashTree, LinkType, Store};
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use serde::Serialize;

use super::{ProviderListEntry, provider_file_probable_os_placeholder_family, split_provider_path};

/// File Provider is reconciling an item already on disk. A collision lets the
/// OS keep both items, while an absent dataless file can safely be forgotten.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", content = "entry", rename_all = "snake_case")]
pub enum Reimport {
    Match(ProviderListEntry),
    Collision(ProviderListEntry),
    Absent,
    Created,
}

/// Call with the current provider root under the configuration mutation lock.
/// Checking the exact path and inserting when absent must be one transaction.
pub async fn reimport_item<S: Store + 'static>(
    tree: &HashTree<S>,
    provider: &HashTreeProviderFs<S>,
    path: &str,
    directory: bool,
    contents: Option<&Path>,
    ensure_writable: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<Reimport> {
    let (parent, name) = split_provider_path(path)?;
    let mut dir = provider.current_root().await;
    for component in parent.split('/').filter(|part| !part.is_empty()) {
        let child = tree
            .list_directory_required(&dir)
            .await?
            .into_iter()
            .find(|child| child.name == component)
            .context("reimport parent is missing")?;
        ensure!(
            matches!(child.link_type, LinkType::Dir | LinkType::Fanout),
            "reimport parent is not a folder"
        );
        dir = Cid {
            hash: child.hash,
            key: child.key,
        };
    }
    let siblings = tree.list_directory_required(&dir).await?;
    if let Some(child) = siblings.iter().find(|child| child.name == name) {
        let cid = Cid {
            hash: child.hash,
            key: child.key,
        };
        let existing_directory = matches!(child.link_type, LinkType::Dir | LinkType::Fanout);
        let entry = ProviderListEntry {
            path: path.to_owned(),
            parent_path: parent,
            display_name: name,
            kind: if existing_directory {
                "directory"
            } else {
                "file"
            },
            size: child.size,
            version: cid.to_string(),
            modified_at: provider.item(&path.to_owned()).await?.modified_at,
        };
        let matches = if existing_directory != directory {
            false
        } else if let (false, Some(source)) = (directory, contents) {
            file_matches(tree, &cid, child.size, source).await?
        } else {
            true
        };
        return Ok(if matches {
            Reimport::Match(entry)
        } else {
            Reimport::Collision(entry)
        });
    }
    if !directory && contents.is_none() {
        return Ok(Reimport::Absent);
    }
    ensure_writable()?;
    if directory {
        provider.create_dir(&parent, &name).await?;
    } else if let Some(source) = contents {
        if let Some(family) =
            provider_file_probable_os_placeholder_family(path, source.metadata()?.len())
        {
            let family_name = split_provider_path(&family)?.1;
            ensure!(
                !siblings.iter().any(|sibling| matches!(
                    sibling.link_type,
                    LinkType::File | LinkType::Blob
                ) && sibling.size > 0
                    && super::provider_collision_family_path(&sibling.name).0 == family_name),
                "refusing probable FileProvider placeholder copy: {path}"
            );
        }
        provider.create_file(&parent, &name).await?;
        provider.write_file_from_path(path, source).await?;
    }
    Ok(Reimport::Created)
}

async fn file_matches<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
    size: u64,
    source: &Path,
) -> anyhow::Result<bool> {
    let mut file = std::fs::File::open(source)?;
    if file.metadata()?.len() != size {
        return Ok(false);
    }
    let mut local = vec![0; 256 * 1024];
    let mut offset = 0;
    while offset < size {
        let len = usize::try_from((size - offset).min(local.len() as u64))?;
        file.read_exact(&mut local[..len])?;
        let end = offset + len as u64;
        let stored = tree
            .read_file_range_cid(cid, offset, Some(end))
            .await?
            .context("reimport file contents are unavailable")?;
        ensure!(stored.len() == len, "incomplete reimport file contents");
        if stored != local[..len] {
            return Ok(false);
        }
        offset = end;
    }
    Ok(file.read(&mut local[..1])? == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{HashTreeConfig, MemoryStore};
    use std::sync::Arc;

    #[tokio::test]
    async fn replay_matches_exact_bytes_and_keeps_both_sides_of_a_collision() {
        let store = Arc::new(MemoryStore::new());
        let tree = Arc::new(HashTree::new(HashTreeConfig::new(store.clone()).public()));
        let provider = HashTreeProviderFs::fresh(tree.clone()).await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("local");
        let mut bytes = vec![7; 300_001];
        std::fs::write(&source, &bytes).unwrap();
        let name = "photos/photo (2).jpeg";
        macro_rules! check {
            ($path:expr, $dir:expr, $source:expr) => {
                reimport_item(&tree, &provider, $path, $dir, $source, || Ok(())).await
            };
        }
        assert!(matches!(
            check!("photos", true, None).unwrap(),
            Reimport::Created
        ));
        assert!(matches!(
            check!(name, false, Some(&source)).unwrap(),
            Reimport::Created
        ));
        let root = provider.current_root().await;
        assert!(matches!(
            check!(name, false, Some(&source)).unwrap(),
            Reimport::Match(_)
        ));
        assert!(matches!(
            check!(name, false, None).unwrap(),
            Reimport::Match(_)
        ));
        assert!(matches!(
            check!("photos", true, None).unwrap(),
            Reimport::Match(_)
        ));
        assert!(matches!(
            check!(name, true, None).unwrap(),
            Reimport::Collision(_)
        ));
        *bytes.last_mut().unwrap() = 8;
        std::fs::write(&source, &bytes).unwrap();
        assert!(matches!(
            check!(name, false, Some(&source)).unwrap(),
            Reimport::Collision(_)
        ));
        assert!(matches!(
            check!("photos/stale", false, None).unwrap(),
            Reimport::Absent
        ));
        assert_eq!(provider.current_root().await, root);
        assert_eq!(
            provider.read(&name.to_owned(), 300_000, 1).await.unwrap(),
            [7]
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        let folder = tree.resolve(&root, "photos").await.unwrap().unwrap();
        store.delete(&folder.hash).await.unwrap();
        assert!(check!("photos/stale", false, None).is_err());
        assert!(check!("photos/stale", false, Some(&source)).is_err());
    }
}
