pub async fn layer_root_meta<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    meta: &DriveRootMeta,
) -> Result<Cid, IndexError> {
    let segments: Vec<&str> = ROOT_META_PATH
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let (name, parent_segs) = segments.split_last().expect("ROOT_META_PATH is non-empty");
    for depth in 1..=parent_segs.len() {
        let dir_path: Vec<String> = parent_segs[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| IndexError::RootMeta(e.to_string()))?;
    let (cid, size) = tree.put(&bytes).await?;
    let new_root = tree
        .set_entry(&root, parent_segs, name, &cid, size, LinkType::Blob)
        .await?;
    Ok(new_root)
}

/// Read root-level causal metadata from `.hashtree/root.json`, if
/// present. Absence is normal for legacy roots.
pub async fn read_root_meta<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Option<DriveRootMeta>, IndexError> {
    let entries = tree.list_directory(root).await?;
    let Some(meta_entry) = entries
        .iter()
        .find(|e| e.name == META_DIR && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    let meta_cid = Cid {
        hash: meta_entry.hash,
        key: meta_entry.key,
    };
    let meta_entries = tree.list_directory(&meta_cid).await?;
    let name = ROOT_META_PATH
        .rsplit_once('/')
        .map_or(ROOT_META_PATH, |(_, name)| name);
    let Some(root_meta) = meta_entries
        .iter()
        .find(|e| e.name == name && e.link_type != LinkType::Dir)
    else {
        return Ok(None);
    };
    let cid = Cid {
        hash: root_meta.hash,
        key: root_meta.key,
    };
    let raw = tree
        .get(&cid, None)
        .await?
        .ok_or_else(|| HashTreeError::MissingChunk(to_hex(&root_meta.hash)))?;
    let meta = serde_json::from_slice(&raw).map_err(|e| IndexError::RootMeta(e.to_string()))?;
    Ok(Some(meta))
}

/// Add or replace durable conflict provenance records under
/// `.hashtree/conflicts/<conflict_id>.json`.
pub async fn layer_conflict_records<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    records: &[ConflictRecord],
) -> Result<Cid, IndexError> {
    if records.is_empty() {
        return Ok(root);
    }
    let conflict_dir: Vec<&str> = CONFLICTS_PREFIX
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    for depth in 1..=conflict_dir.len() {
        let dir_path: Vec<String> = conflict_dir[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }

    for record in records {
        let name = conflict_record_leaf_name(&record.conflict_id)?;
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|e| IndexError::ConflictRecord(e.to_string()))?;
        let (cid, size) = tree.put(&bytes).await?;
        root = tree
            .set_entry(&root, &conflict_dir, &name, &cid, size, LinkType::Blob)
            .await?;
    }
    Ok(root)
}

/// Read durable conflict provenance records from `.hashtree/conflicts`.
/// Absence is normal for snapshots without conflicts.
pub async fn read_conflict_records<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Vec<ConflictRecord>, IndexError> {
    let Some(conflicts_dir) = resolve_conflicts_dir(tree, root).await? else {
        return Ok(Vec::new());
    };
    let mut entries = tree.list_directory(&conflicts_dir).await?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut records = Vec::new();
    for entry in entries {
        if entry.link_type == LinkType::Dir {
            continue;
        }
        let Some(conflict_id) = entry.name.strip_suffix(".json") else {
            continue;
        };
        validate_conflict_id(conflict_id)?;
        let cid = Cid {
            hash: entry.hash,
            key: entry.key,
        };
        let raw = tree
            .get(&cid, None)
            .await?
            .ok_or_else(|| HashTreeError::MissingChunk(to_hex(&entry.hash)))?;
        let record: ConflictRecord =
            serde_json::from_slice(&raw).map_err(|e| IndexError::ConflictRecord(e.to_string()))?;
        if record.conflict_id != conflict_id {
            return Err(IndexError::ConflictRecord(format!(
                "conflict_id {} does not match record filename {conflict_id}",
                record.conflict_id
            )));
        }
        records.push(record);
    }
    Ok(records)
}

async fn resolve_conflicts_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Option<Cid>, IndexError> {
    let root_entries = tree.list_directory(root).await?;
    let Some(meta_entry) = root_entries
        .iter()
        .find(|e| e.name == META_DIR && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    let meta_cid = Cid {
        hash: meta_entry.hash,
        key: meta_entry.key,
    };
    let meta_entries = tree.list_directory(&meta_cid).await?;
    let conflict_dir_name = CONFLICTS_PREFIX
        .rsplit_once('/')
        .map_or(CONFLICTS_PREFIX, |(_, name)| name);
    let Some(conflicts_entry) = meta_entries
        .iter()
        .find(|e| e.name == conflict_dir_name && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    Ok(Some(Cid {
        hash: conflicts_entry.hash,
        key: conflicts_entry.key,
    }))
}

fn conflict_record_leaf_name(conflict_id: &str) -> Result<String, IndexError> {
    validate_conflict_id(conflict_id)?;
    Ok(format!("{conflict_id}.json"))
}

fn validate_conflict_id(conflict_id: &str) -> Result<(), IndexError> {
    if conflict_id.is_empty()
        || conflict_id == "."
        || conflict_id == ".."
        || conflict_id.contains('/')
        || conflict_id.contains('\\')
    {
        return Err(IndexError::ConflictRecord(
            "conflict_id must be one non-empty path segment".into(),
        ));
    }
    Ok(())
}

pub async fn layer_prev_link<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous: &Cid,
) -> Result<Cid, IndexError> {
    let segments: Vec<&str> = PREV_LINK_PATH
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let (name, parent_segs) = segments.split_last().expect("PREV_LINK_PATH is non-empty");
    // Ensure each ancestor (just `.hashtree/` for now) exists.
    for depth in 1..=parent_segs.len() {
        let dir_path: Vec<String> = parent_segs[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }
    let new_root = tree
        .set_entry(&root, parent_segs, name, previous, 0, LinkType::Dir)
        .await?;
    Ok(new_root)
}

fn collect_local_file_paths(
    dir: &Path,
    prefix: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), IndexError> {
    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|s| IndexError::NonUtf8(s.to_string_lossy().into_owned()))?;
        entries.push((name, entry.path()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path) in entries {
        if should_ignore_name(&name) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let logical_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            collect_local_file_paths(&path, &logical_path, out)?;
        } else if metadata.is_file() {
            out.insert(logical_path);
        }
    }
    Ok(())
}

async fn layer_tombstones<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    tombstones: &BTreeMap<String, i64>,
) -> Result<Cid, IndexError> {
    let mut tombstone_tree = TombstoneDir::default();
    for (orig_path, ts) in tombstones {
        tombstone_tree.insert(orig_path, *ts);
    }
    let tombstone_root = materialize_tombstone_dir(tree, &tombstone_tree).await?;
    let meta_root = tree
        .put_directory(vec![
            DirEntry::from_cid("tombstones", &tombstone_root).with_link_type(LinkType::Dir),
        ])
        .await?;
    root = tree
        .set_entry(&root, &[], META_DIR, &meta_root, 0, LinkType::Dir)
        .await?;
    Ok(root)
}

#[derive(Default)]
struct TombstoneDir {
    dirs: BTreeMap<String, TombstoneDir>,
    files: BTreeMap<String, i64>,
}

impl TombstoneDir {
    fn insert(&mut self, path: &str, tombstoned_at: i64) {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        self.insert_parts(&parts, tombstoned_at);
    }

    fn insert_parts(&mut self, parts: &[&str], tombstoned_at: i64) {
        match parts {
            [] => {}
            [name] => {
                self.files.insert((*name).to_string(), tombstoned_at);
            }
            [dir, rest @ ..] => {
                self.dirs
                    .entry((*dir).to_string())
                    .or_default()
                    .insert_parts(rest, tombstoned_at);
            }
        }
    }
}

fn materialize_tombstone_dir<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a TombstoneDir,
) -> futures::future::BoxFuture<'a, Result<Cid, IndexError>> {
    Box::pin(async move {
        let mut entries = Vec::with_capacity(dir.dirs.len() + dir.files.len());
        for (name, child) in &dir.dirs {
            let cid = materialize_tombstone_dir(tree, child).await?;
            entries.push(DirEntry::from_cid(name, &cid).with_link_type(LinkType::Dir));
        }
        for (name, tombstoned_at) in &dir.files {
            let bytes = tombstoned_at.to_string().into_bytes();
            let (cid, size) = tree.put(&bytes).await?;
            entries.push(
                DirEntry::from_cid(name, &cid)
                    .with_size(size)
                    .with_link_type(LinkType::Blob),
            );
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tree.put_directory(entries).await?)
    })
}

/// Create `dir_path` as a directory under `root` if it isn't already.
/// All ancestors of `dir_path` must already exist (call this in
/// shortest-prefix-first order over a set of paths).
async fn ensure_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    dir_path: &[String],
) -> Result<Cid, IndexError> {
    let segs: Vec<&str> = dir_path.iter().map(String::as_str).collect();
    let (name, parent_segs) = segs.split_last().expect("dir_path must be non-empty");
    let parent_cid = resolve_dir(tree, root, parent_segs).await?;
    let entries = tree.list_directory(&parent_cid).await?;
    if entries
        .iter()
        .any(|e| e.name == *name && e.link_type == LinkType::Dir)
    {
        return Ok(root.clone());
    }
    let empty = tree.put_directory(Vec::new()).await?;
    let new_root = tree
        .set_entry(root, parent_segs, name, &empty, 0, LinkType::Dir)
        .await?;
    Ok(new_root)
}

/// Resolve `segments` down from `root`, returning the CID of the
/// directory at that path. Empty `segments` returns `root`.
async fn resolve_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    segments: &[&str],
) -> Result<Cid, IndexError> {
    let mut current = root.clone();
    for seg in segments {
        let entries = tree.list_directory(&current).await?;
        let entry = entries
            .iter()
            .find(|e| e.name == *seg && e.link_type == LinkType::Dir)
            .ok_or_else(|| IndexError::Tree(HashTreeError::PathNotFound((*seg).to_string())))?;
        current = Cid {
            hash: entry.hash,
            key: entry.key,
        };
    }
    Ok(current)
}

fn index_dir_inner<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Path,
) -> futures::future::BoxFuture<'a, Result<Cid, IndexError>> {
    Box::pin(async move {
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut children: Vec<(String, std::path::PathBuf)> = Vec::new();

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .into_string()
                .map_err(|s| IndexError::NonUtf8(s.to_string_lossy().into_owned()))?;
            children.push((name, entry.path()));
        }

        // Sort for determinism.
        children.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, path) in children {
            if should_ignore_name(&name) {
                continue;
            }
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => return Err(IndexError::Io(e)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let child_cid = index_dir_inner(tree, &path).await?;
                let mut e = DirEntry::from_cid(name, &child_cid);
                e.link_type = LinkType::Dir;
                entries.push(e);
            } else if metadata.is_file() {
                let bytes = std::fs::read(&path)?;
                let size = bytes.len() as u64;
                let whole_file_hash = hashtree_core::sha256(&bytes);
                let (cid, _) = tree.put(&bytes).await?;
                let link_type = if size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
                    LinkType::File
                } else {
                    LinkType::Blob
                };
                let mut e = DirEntry::from_cid(name, &cid)
                    .with_size(size)
                    .with_meta(file_entry_meta(&whole_file_hash));
                e.link_type = link_type;
                entries.push(e);
            }
        }

        let cid = tree.put_directory(entries).await?;
        Ok(cid)
    })
}

fn file_entry_meta(whole_file_hash: &[u8; 32]) -> HashMap<String, serde_json::Value> {
    HashMap::from([(
        WHOLE_FILE_HASH_META_KEY.to_string(),
        serde_json::Value::String(to_hex(whole_file_hash)),
    )])
}
