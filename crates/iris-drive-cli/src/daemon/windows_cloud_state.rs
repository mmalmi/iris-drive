#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsCloudExpectedEntry {
    path: String,
    kind: &'static str,
    size: u64,
    version: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct WindowsCloudLocalStateEntry {
    path: String,
    kind: String,
    size: u64,
    sha256: Option<String>,
    provider_version: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct WindowsCloudCleanupDeleteMarker {
    path: String,
    created_at_unix_ms: u64,
}

impl WindowsCloudLocalStateEntry {
    fn is_directory(&self) -> bool {
        self.kind.eq_ignore_ascii_case("directory")
    }
}

async fn windows_cloud_provider_expected_entries(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
) -> Result<Vec<WindowsCloudExpectedEntry>> {
    let mut entries = Vec::new();
    let mut stack = vec![(String::new(), root.clone())];
    while let Some((parent, dir_cid)) = stack.pop() {
        let mut children = tree.list_directory(&dir_cid).await?;
        children.sort_by(|a, b| a.name.cmp(&b.name));
        for child in children {
            let path = if parent.is_empty() {
                child.name.clone()
            } else {
                format!("{parent}/{}", child.name)
            };
            let cid = Cid {
                hash: child.hash,
                key: child.key,
            };
            let kind = match child.link_type {
                LinkType::Dir => {
                    stack.push((path.clone(), cid.clone()));
                    "directory"
                }
                LinkType::Blob | LinkType::File => "file",
            };
            entries.push(WindowsCloudExpectedEntry {
                path,
                kind,
                size: child.size,
                version: cid.to_string(),
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsCloudProjectionRefresh {
    entry_count: usize,
    removed_paths: Vec<String>,
    changed_paths: Vec<String>,
}

#[cfg(windows)]
fn windows_cloud_projection_root() -> Option<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Iris Drive"))
        .filter(|root| root.is_dir())
}

#[cfg(not(windows))]
fn windows_cloud_projection_root() -> Option<PathBuf> {
    None
}

async fn refresh_windows_cloud_local_projection(
    config_dir: &Path,
    sync_root: &Path,
) -> Result<WindowsCloudProjectionRefresh> {
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let daemon =
        Daemon::open(config_dir).context("opening daemon for Windows Cloud Files projection")?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building Windows Cloud Files projection root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone()).await?;
    let current_entries =
        windows_cloud_provider_expected_entries(daemon.tree(), &visible.root_cid).await?;
    let expected_paths: BTreeSet<String> = current_entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let previous_local_state = load_windows_cloud_local_state(config_dir);
    let changed_paths = windows_cloud_remove_changed_synced_local_files(
        config_dir,
        sync_root,
        &provider,
        &current_entries,
        &previous_local_state,
        &BTreeSet::new(),
    )
    .await?;
    let removed_paths = windows_cloud_remove_stale_synced_local_items(
        sync_root,
        &expected_paths,
        &previous_local_state,
        &BTreeSet::new(),
    );
    write_windows_cloud_local_state(
        config_dir,
        sync_root,
        &current_entries,
        &previous_local_state,
        &BTreeSet::new(),
    );
    Ok(WindowsCloudProjectionRefresh {
        entry_count: current_entries.len(),
        removed_paths,
        changed_paths,
    })
}

fn load_windows_cloud_local_state(config_dir: &Path) -> Vec<WindowsCloudLocalStateEntry> {
    let path = config_dir.join(WINDOWS_CLOUD_LOCAL_STATE_FILE);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    value
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(windows_cloud_local_state_entry_from_json)
        .collect()
}

fn windows_cloud_local_state_entry_from_json(value: &Value) -> Option<WindowsCloudLocalStateEntry> {
    let path = windows_cloud_json_string(value, "path", "Path")
        .and_then(|path| normalize_provider_path(path).ok())?;
    if iris_drive_core::path_has_ignored_component(&path) {
        return None;
    }
    let kind = windows_cloud_json_string(value, "kind", "Kind")
        .unwrap_or("file")
        .to_string();
    let size = windows_cloud_json_u64(value, "size", "Size").unwrap_or(0);
    let sha256 = windows_cloud_json_string(value, "sha256", "Sha256")
        .filter(|hash| !hash.trim().is_empty())
        .map(str::to_string);
    let provider_version = windows_cloud_json_string(value, "providerVersion", "ProviderVersion")
        .filter(|version| !version.trim().is_empty())
        .map(str::to_string);
    Some(WindowsCloudLocalStateEntry {
        path,
        kind,
        size,
        sha256,
        provider_version,
    })
}

fn windows_cloud_json_string<'a>(value: &'a Value, lower: &str, upper: &str) -> Option<&'a str> {
    value
        .get(lower)
        .or_else(|| value.get(upper))
        .and_then(Value::as_str)
}

fn windows_cloud_json_u64(value: &Value, lower: &str, upper: &str) -> Option<u64> {
    value
        .get(lower)
        .or_else(|| value.get(upper))
        .and_then(Value::as_u64)
}

fn windows_cloud_remove_stale_synced_local_items(
    sync_root: &Path,
    expected_paths: &BTreeSet<String>,
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) -> Vec<String> {
    if previous_state.is_empty() {
        return Vec::new();
    }
    let mut state = previous_state.to_vec();
    state.sort_by(|a, b| {
        b.path
            .split('/')
            .count()
            .cmp(&a.path.split('/').count())
            .then_with(|| b.path.cmp(&a.path))
    });
    let mut removed = Vec::new();

    for previous in state {
        let Ok(path) = normalize_provider_path(&previous.path) else {
            continue;
        };
        if iris_drive_core::path_has_ignored_component(&path) || expected_paths.contains(&path) {
            continue;
        }
        if windows_cloud_path_is_protected_local_mutation(&path, protected_paths) {
            continue;
        }
        let full_path = windows_cloud_full_path(sync_root, &path);
        if previous.is_directory() {
            if full_path.is_dir()
                && !windows_cloud_path_is_reparse_point(&full_path)
                && windows_cloud_remove_dir_with_retry(&full_path)
            {
                removed.push(path);
            }
            continue;
        }

        let Some(expected_hash) = previous.sha256.as_deref() else {
            continue;
        };
        if !full_path.is_file() || windows_cloud_path_is_reparse_point(&full_path) {
            continue;
        }
        let Ok(Some(snapshot)) = windows_cloud_snapshot_local_file(&full_path) else {
            continue;
        };
        if snapshot.size != previous.size || snapshot.sha256 != expected_hash {
            continue;
        }
        let _ = windows_cloud_clear_readonly(&full_path);
        if windows_cloud_remove_file_with_retry(&full_path) {
            removed.push(path);
        }
    }

    removed
}

async fn windows_cloud_remove_changed_synced_local_files(
    config_dir: &Path,
    sync_root: &Path,
    provider: &HashTreeProviderFs<FsBlobStore>,
    entries: &[WindowsCloudExpectedEntry],
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    if previous_state.is_empty() {
        return Ok(Vec::new());
    }

    let current_files = entries
        .iter()
        .filter(|entry| entry.kind == "file")
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut removed = Vec::new();
    for previous in previous_state {
        let Ok(path) = normalize_provider_path(&previous.path) else {
            continue;
        };
        if previous.is_directory()
            || previous.sha256.is_none()
            || iris_drive_core::path_has_ignored_component(&path)
            || windows_cloud_path_is_protected_local_mutation(&path, protected_paths)
        {
            continue;
        }
        let Some(current) = current_files.get(path.as_str()) else {
            continue;
        };
        let full_path = windows_cloud_full_path(sync_root, &path);
        let local = match windows_cloud_snapshot_local_file(&full_path) {
            Ok(Some(local)) if local.size == previous.size => local,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) if windows_cloud_file_read_should_skip(&error) => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", full_path.display()));
            }
        };
        if previous.sha256.as_deref() != Some(local.sha256.as_str()) {
            continue;
        }

        let provider_changed = if current.size == local.size {
            let bytes = match std::fs::read(&full_path) {
                Ok(bytes) => bytes,
                Err(error) if windows_cloud_file_read_should_skip(&error) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", full_path.display()));
                }
            };
            !provider_file_matches(provider, &path, &bytes).await?
        } else {
            true
        };
        if !provider_changed {
            continue;
        }

        let cleanup_marker = WindowsCloudCleanupDeleteMarker {
            path: path.clone(),
            created_at_unix_ms: windows_cloud_cleanup_marker_now_ms(),
        };
        append_windows_cloud_cleanup_delete_markers(config_dir, &[cleanup_marker]);
        let _ = windows_cloud_clear_readonly(&full_path);
        if windows_cloud_remove_file_with_retry(&full_path) {
            removed.push(path);
        }
    }

    Ok(removed)
}

fn windows_cloud_remove_file_with_retry(path: &Path) -> bool {
    windows_cloud_remove_with_retry(|| std::fs::remove_file(path))
}

fn windows_cloud_remove_dir_with_retry(path: &Path) -> bool {
    windows_cloud_remove_with_retry(|| std::fs::remove_dir(path))
}

fn windows_cloud_remove_with_retry(mut remove: impl FnMut() -> std::io::Result<()>) -> bool {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    let delay = std::time::Duration::from_millis(100);

    loop {
        match remove() {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if started.elapsed() < timeout => std::thread::sleep(delay),
            Err(_) => return false,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsCloudLocalFileSnapshot {
    size: u64,
    sha256: String,
}

fn windows_cloud_snapshot_local_file(
    path: &Path,
) -> std::io::Result<Option<WindowsCloudLocalFileSnapshot>> {
    if windows_cloud_path_is_reparse_point(path) {
        return Ok(None);
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if windows_cloud_file_read_should_skip(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(WindowsCloudLocalFileSnapshot {
        size: bytes.len() as u64,
        sha256: to_hex(&hashtree_core::sha256(&bytes)),
    }))
}

#[cfg(windows)]
fn windows_cloud_clear_readonly(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn windows_cloud_clear_readonly(path: &Path) -> std::io::Result<()> {
    let _ = std::fs::metadata(path)?;
    Ok(())
}

fn write_windows_cloud_local_state(
    config_dir: &Path,
    sync_root: &Path,
    entries: &[WindowsCloudExpectedEntry],
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) {
    let state =
        snapshot_windows_cloud_local_state(sync_root, entries, previous_state, protected_paths);
    let value = json!({ "entries": windows_cloud_local_state_json_entries(&state, entries) });
    if let Ok(raw) = serde_json::to_string(&value) {
        let _ = std::fs::create_dir_all(config_dir);
        let _ = std::fs::write(config_dir.join(WINDOWS_CLOUD_LOCAL_STATE_FILE), raw);
    }
}

fn windows_cloud_local_state_json_entries(
    state: &[WindowsCloudLocalStateEntry],
    entries: &[WindowsCloudExpectedEntry],
) -> Vec<Value> {
    let versions = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.version.as_str()))
        .collect::<BTreeMap<_, _>>();
    state
        .iter()
        .map(|entry| {
            let mut object = serde_json::Map::new();
            object.insert("path".to_string(), json!(entry.path));
            object.insert("kind".to_string(), json!(entry.kind));
            object.insert("size".to_string(), json!(entry.size));
            if let Some(sha256) = entry.sha256.as_deref() {
                object.insert("sha256".to_string(), json!(sha256));
            }
            let provider_version = entry
                .provider_version
                .as_deref()
                .or_else(|| versions.get(entry.path.as_str()).copied());
            if let Some(version) = provider_version
                && !version.trim().is_empty()
            {
                object.insert("providerVersion".to_string(), json!(version));
            }
            Value::Object(object)
        })
        .collect()
}

fn snapshot_windows_cloud_local_state(
    sync_root: &Path,
    entries: &[WindowsCloudExpectedEntry],
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) -> Vec<WindowsCloudLocalStateEntry> {
    let mut state = Vec::new();
    let mut current_paths = BTreeSet::new();
    let previous_by_path = previous_state
        .iter()
        .filter_map(|entry| {
            normalize_provider_path(&entry.path)
                .ok()
                .map(|path| (path, entry))
        })
        .collect::<BTreeMap<_, _>>();
    for entry in entries {
        if iris_drive_core::path_has_ignored_component(&entry.path) {
            continue;
        }
        if windows_cloud_path_is_protected_local_mutation(&entry.path, protected_paths) {
            continue;
        }
        current_paths.insert(entry.path.clone());
        let full_path = windows_cloud_full_path(sync_root, &entry.path);
        let previous = previous_by_path.get(&entry.path).copied();
        if entry.kind == "directory" {
            if full_path.is_dir() {
                state.push(WindowsCloudLocalStateEntry {
                    path: entry.path.clone(),
                    kind: "directory".to_string(),
                    size: 0,
                    sha256: None,
                    provider_version: Some(entry.version.clone()),
                });
            }
            continue;
        }
        if !full_path.is_file() {
            continue;
        }
        if windows_cloud_path_is_reparse_point(&full_path) {
            state.push(WindowsCloudLocalStateEntry {
                path: entry.path.clone(),
                kind: "file".to_string(),
                size: entry.size,
                sha256: None,
                provider_version: windows_cloud_snapshot_provider_version(
                    entry, previous, true, None,
                ),
            });
            continue;
        }
        if let Ok(Some(snapshot)) = windows_cloud_snapshot_local_file(&full_path) {
            state.push(WindowsCloudLocalStateEntry {
                path: entry.path.clone(),
                kind: "file".to_string(),
                size: snapshot.size,
                sha256: Some(snapshot.sha256.clone()),
                provider_version: windows_cloud_snapshot_provider_version(
                    entry,
                    previous,
                    false,
                    Some(&snapshot),
                ),
            });
        }
    }
    for previous in windows_cloud_retained_stale_local_state(
        sync_root,
        &current_paths,
        previous_state,
        protected_paths,
    ) {
        if current_paths.insert(previous.path.clone()) {
            state.push(previous);
        }
    }
    state.sort_by(|a, b| a.path.cmp(&b.path));
    state
}

fn windows_cloud_snapshot_provider_version(
    entry: &WindowsCloudExpectedEntry,
    previous: Option<&WindowsCloudLocalStateEntry>,
    is_reparse_point: bool,
    local_snapshot: Option<&WindowsCloudLocalFileSnapshot>,
) -> Option<String> {
    let current = (!entry.version.trim().is_empty()).then_some(entry.version.clone());
    let Some(previous) = previous else {
        return current;
    };
    if entry.kind != "file" || previous.is_directory() {
        return current;
    }
    let Some(previous_version) = previous.provider_version.as_deref() else {
        return current;
    };
    if previous_version.trim().is_empty() || current.as_deref() == Some(previous_version) {
        return current;
    }

    if is_reparse_point {
        return Some(previous_version.to_string());
    }

    if let (Some(previous_hash), Some(local)) = (previous.sha256.as_deref(), local_snapshot)
        && previous.size == local.size
        && previous_hash == local.sha256
    {
        return Some(previous_version.to_string());
    }

    current
}

fn windows_cloud_retained_stale_local_state(
    sync_root: &Path,
    current_paths: &BTreeSet<String>,
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) -> Vec<WindowsCloudLocalStateEntry> {
    let mut retained = Vec::new();
    for previous in previous_state {
        let Ok(path) = normalize_provider_path(&previous.path) else {
            continue;
        };
        if iris_drive_core::path_has_ignored_component(&path)
            || current_paths.contains(&path)
            || windows_cloud_path_is_protected_local_mutation(&path, protected_paths)
        {
            continue;
        }
        let full_path = windows_cloud_full_path(sync_root, &path);
        if previous.is_directory() {
            if full_path.is_dir() && !windows_cloud_path_is_reparse_point(&full_path) {
                retained.push(WindowsCloudLocalStateEntry {
                    path,
                    kind: previous.kind.clone(),
                    size: previous.size,
                    sha256: previous.sha256.clone(),
                    provider_version: previous.provider_version.clone(),
                });
            }
            continue;
        }
        let Some(expected_hash) = previous.sha256.as_deref() else {
            continue;
        };
        if !full_path.is_file() || windows_cloud_path_is_reparse_point(&full_path) {
            continue;
        }
        let Ok(Some(snapshot)) = windows_cloud_snapshot_local_file(&full_path) else {
            continue;
        };
        if snapshot.size == previous.size && snapshot.sha256 == expected_hash {
            retained.push(WindowsCloudLocalStateEntry {
                path,
                kind: previous.kind.clone(),
                size: previous.size,
                sha256: previous.sha256.clone(),
                provider_version: previous.provider_version.clone(),
            });
        }
    }
    retained.sort_by(|a, b| a.path.cmp(&b.path));
    retained
}

fn windows_cloud_path_is_protected_local_mutation(
    path: &str,
    protected_paths: &BTreeSet<String>,
) -> bool {
    protected_paths
        .iter()
        .any(|protected| windows_cloud_paths_overlap(path, protected))
}

fn windows_cloud_paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn windows_cloud_full_path(root: &Path, virtual_path: &str) -> PathBuf {
    let mut full_path = root.to_path_buf();
    for part in virtual_path.split('/').filter(|part| !part.is_empty()) {
        full_path.push(part);
    }
    full_path
}

fn windows_cloud_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() || part == "." || part == ".." {
                    return None;
                }
                parts.push(part.into_owned());
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn windows_cloud_path_is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| windows_cloud_metadata_is_reparse_point(&metadata))
}

fn windows_cloud_file_read_should_skip(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
    ) {
        return true;
    }

    // Cloud Files placeholders can report cloud-specific read errors even when
    // the regular reparse-point bit is not enough to identify them.
    matches!(
        error.raw_os_error(),
        Some(362 | 395 | 396 | 397 | 398 | 400 | 402 | 404)
    )
}

#[cfg(windows)]
fn windows_cloud_metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn windows_cloud_metadata_is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}
