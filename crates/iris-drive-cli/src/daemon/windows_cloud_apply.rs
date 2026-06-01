async fn apply_windows_cloud_rename(
    provider: &HashTreeProviderFs<FsBlobStore>,
    sync_root: &Path,
    old_path: &str,
    new_path: &str,
    placeholder_paths: &BTreeSet<String>,
) -> Result<bool> {
    let old_path = normalize_provider_path(old_path)?;
    let new_path = normalize_provider_path(new_path)?;
    if iris_drive_core::path_has_ignored_component(&new_path) {
        let deleted_old = apply_windows_cloud_delete(provider, &old_path).await?;
        let deleted_new = apply_windows_cloud_delete(provider, &new_path).await?;
        return Ok(deleted_old || deleted_new);
    }
    if iris_drive_core::path_has_ignored_component(&old_path) {
        let deleted_old = apply_windows_cloud_delete(provider, &old_path).await?;
        let upserted_new =
            apply_windows_cloud_upsert(provider, sync_root, &new_path, placeholder_paths).await?;
        return Ok(deleted_old || upserted_new);
    }
    let new_full_path = windows_cloud_full_path(sync_root, &new_path);
    if windows_cloud_path_is_reparse_point(&new_full_path) {
        match provider.item(&old_path).await {
            Ok(_) => {
                rename_provider_path(provider, &old_path, &new_path).await?;
                return Ok(true);
            }
            Err(_) => return Ok(false),
        }
    }

    let deleted = apply_windows_cloud_delete(provider, &old_path).await?;
    let upserted =
        apply_windows_cloud_upsert(provider, sync_root, &new_path, placeholder_paths).await?;
    Ok(deleted || upserted)
}

async fn apply_windows_cloud_delete(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<bool> {
    let Ok(path) = normalize_provider_path(path) else {
        return Ok(false);
    };
    match provider.item(&path).await {
        Ok(_) => {
            delete_provider_path(provider, &path).await?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

async fn apply_windows_cloud_delete_if_local_missing(
    provider: &HashTreeProviderFs<FsBlobStore>,
    sync_root: &Path,
    path: &str,
) -> Result<bool> {
    let Ok(path) = normalize_provider_path(path) else {
        return Ok(false);
    };
    let full_path = windows_cloud_full_path(sync_root, &path);
    match std::fs::symlink_metadata(&full_path) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading metadata for {}", full_path.display()));
        }
    }
    apply_windows_cloud_delete(provider, &path).await
}

async fn apply_windows_cloud_upsert(
    provider: &HashTreeProviderFs<FsBlobStore>,
    sync_root: &Path,
    path: &str,
    placeholder_paths: &BTreeSet<String>,
) -> Result<bool> {
    let Ok(path) = normalize_provider_path(path) else {
        return Ok(false);
    };
    if iris_drive_core::path_has_ignored_component(&path) {
        return apply_windows_cloud_delete(provider, &path).await;
    }
    if placeholder_paths.contains(&path) && provider.item(&path).await.is_err() {
        return Ok(false);
    }
    let mut changed = false;
    let mut stack = vec![path];
    while let Some(path) = stack.pop() {
        let full_path = windows_cloud_full_path(sync_root, &path);
        let metadata = match std::fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", full_path.display()));
            }
        };
        let is_reparse_point = windows_cloud_metadata_is_reparse_point(&metadata);
        if is_reparse_point && !metadata.is_dir() {
            continue;
        }
        if metadata.is_dir() {
            if !is_reparse_point && !provider_dir_exists(provider, &path).await? {
                create_provider_dir(provider, &path).await?;
                changed = true;
            }
            let mut children = Vec::new();
            for entry in std::fs::read_dir(&full_path).with_context(|| format!("reading {path}"))?
            {
                let entry = entry?;
                let child = entry.path();
                let child_metadata = match std::fs::symlink_metadata(&child) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("reading metadata for {}", child.display()));
                    }
                };
                if windows_cloud_metadata_is_reparse_point(&child_metadata)
                    && !child_metadata.is_dir()
                {
                    continue;
                }
                if let Some(relative) = windows_cloud_relative_path(sync_root, &child) {
                    children.push(relative);
                }
            }
            children.sort_by(|a, b| b.cmp(a));
            stack.extend(children);
        } else if metadata.is_file() {
            let bytes = match std::fs::read(&full_path) {
                Ok(bytes) => bytes,
                Err(error) if windows_cloud_file_read_should_skip(&error) => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", full_path.display()));
                }
            };
            if provider_file_matches(provider, &path, &bytes).await? {
                continue;
            }
            write_provider_file(provider, &path, &bytes).await?;
            changed = true;
        }
    }
    Ok(changed)
}

async fn provider_dir_exists(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<bool> {
    match provider.item(&path.to_string()).await {
        Ok(item) => Ok(item.kind == ItemKind::Directory),
        Err(_) => Ok(false),
    }
}

async fn provider_file_matches(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
    bytes: &[u8],
) -> Result<bool> {
    let path = path.to_string();
    let item = match provider.item(&path).await {
        Ok(item) if item.kind == ItemKind::File => item,
        Ok(_) | Err(_) => return Ok(false),
    };
    if item.size != bytes.len() as u64 {
        return Ok(false);
    }
    let existing = provider
        .read(&path, 0, item.size)
        .await
        .with_context(|| format!("reading provider file {path}"))?;
    Ok(existing == bytes)
}

async fn prune_ignored_provider_paths(
    provider: &HashTreeProviderFs<FsBlobStore>,
) -> Result<Vec<String>> {
    let mut pruned = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent) = stack.pop() {
        let mut children = provider.read_dir(&parent).await?;
        children.sort_by(|a, b| a.id.cmp(&b.id));
        for child in children {
            let path = child.id;
            if iris_drive_core::path_has_ignored_component(&path) {
                if apply_windows_cloud_delete(provider, &path).await? {
                    pruned.push(path);
                }
                continue;
            }
            let item = provider.item(&path).await?;
            if item.kind == ItemKind::Directory {
                stack.push(path);
            }
        }
    }
    Ok(pruned)
}

fn windows_cloud_local_projected_paths(root: &Path) -> Result<Vec<String>> {
    windows_cloud_local_projected_paths_since(root, None)
}

const WINDOWS_CLOUD_RECENT_RESCAN_SECS: u64 = 300;

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_cloud_cached_delete_recovery_enabled() -> bool {
    // Cached placeholder delete recovery is opt-in because projection lag can
    // make remote files appear locally missing during normal Cloud Files sync.
    std::env::var("IRIS_DRIVE_WINDOWS_CLOUD_FULL_RESCAN").is_ok_and(|value| value == "1")
}

fn windows_cloud_recent_local_projected_paths(root: &Path) -> Result<Vec<String>> {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            WINDOWS_CLOUD_RECENT_RESCAN_SECS,
        ))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    windows_cloud_local_projected_paths_since(root, Some(cutoff))
}

fn windows_cloud_local_projected_paths_since(
    root: &Path,
    modified_since: Option<std::time::SystemTime>,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    if !root.is_dir() {
        return Ok(paths);
    }
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", path.display()));
            }
        };
        if windows_cloud_metadata_is_reparse_point(&metadata) && !metadata.is_dir() {
            continue;
        }
        if let Some(cutoff) = modified_since
            && metadata.modified().is_ok_and(|modified| modified < cutoff)
        {
            continue;
        }
        let Some(relative) = windows_cloud_relative_path(root, &path) else {
            continue;
        };
        paths.push(relative);
    }
    paths.sort();
    Ok(paths)
}

fn windows_cloud_missing_cached_provider_paths(
    root: &Path,
    cached_paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for path in cached_paths {
        let full_path = windows_cloud_full_path(root, path);
        match std::fs::symlink_metadata(&full_path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                paths.push(path.clone());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", full_path.display()));
            }
        }
    }
    paths.sort_by(|a, b| {
        a.split('/')
            .count()
            .cmp(&b.split('/').count())
            .then_with(|| a.cmp(b))
    });
    Ok(paths)
}

fn windows_cloud_rescan_missing_cached_provider_paths(
    root: &Path,
    cached_paths: &BTreeSet<String>,
    full: bool,
) -> Result<Vec<String>> {
    if !full {
        return Ok(Vec::new());
    }
    windows_cloud_missing_cached_provider_paths(root, cached_paths)
}

fn windows_cloud_missing_previous_local_state_paths(
    root: &Path,
    previous_state: &[WindowsCloudLocalStateEntry],
    protected_paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for previous in previous_state {
        let Ok(path) = normalize_provider_path(&previous.path) else {
            continue;
        };
        if iris_drive_core::path_has_ignored_component(&path)
            || windows_cloud_path_is_protected_local_mutation(&path, protected_paths)
        {
            continue;
        }
        let full_path = windows_cloud_full_path(root, &path);
        match std::fs::symlink_metadata(&full_path) {
            Ok(metadata) => {
                if windows_cloud_previous_local_state_reparse_counts_as_missing(
                    previous,
                    windows_cloud_metadata_is_reparse_point(&metadata),
                ) {
                    paths.push(path);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                paths.push(path);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", full_path.display()));
            }
        }
    }
    paths.sort_by(|a, b| {
        b.split('/')
            .count()
            .cmp(&a.split('/').count())
            .then_with(|| b.cmp(a))
    });
    Ok(paths)
}

fn windows_cloud_previous_local_state_reparse_counts_as_missing(
    _previous: &WindowsCloudLocalStateEntry,
    _is_reparse_point: bool,
) -> bool {
    // Reparse points can be normal Cloud Files placeholders; explicit NotFound
    // events handle real local deletes without pruning projected remote files.
    false
}

fn load_windows_cloud_provider_path_cache(config_dir: &Path) -> BTreeSet<String> {
    let path = config_dir.join("windows-cloud-provider-paths.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return BTreeSet::new();
    };
    value
        .get("paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|path| normalize_provider_path(path).ok())
        .collect()
}

fn consume_windows_cloud_cleanup_delete_marker(config_dir: &Path, path: &str) -> bool {
    let Ok(path) = normalize_provider_path(path) else {
        return false;
    };
    let marker_path = config_dir.join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE);
    let Ok(raw) = std::fs::read_to_string(&marker_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let Some(entries) = value.get("entries").and_then(Value::as_array) else {
        return false;
    };

    let now_ms = windows_cloud_cleanup_marker_now_ms();
    let min_created_at_ms = now_ms.saturating_sub(WINDOWS_CLOUD_CLEANUP_DELETE_MARKER_SECS * 1_000);
    let mut matched = false;
    let mut changed = false;
    let mut retained = Vec::new();
    for entry in entries {
        let Some(marker) = windows_cloud_cleanup_delete_marker_from_json(entry) else {
            changed = true;
            continue;
        };
        if marker.created_at_unix_ms < min_created_at_ms {
            changed = true;
            continue;
        }
        if windows_cloud_paths_overlap(&marker.path, &path)
            || windows_cloud_paths_overlap(&path, &marker.path)
        {
            matched = true;
            changed = true;
            continue;
        }
        retained.push(marker);
    }

    if changed {
        write_windows_cloud_cleanup_delete_markers(config_dir, &retained);
    }

    matched
}

fn windows_cloud_cleanup_delete_marker_from_json(
    value: &Value,
) -> Option<WindowsCloudCleanupDeleteMarker> {
    let path = windows_cloud_json_string(value, "path", "Path")
        .and_then(|path| normalize_provider_path(path).ok())?;
    let created_at_unix_ms = windows_cloud_json_u64(value, "created_at_unix_ms", "CreatedAtUnixMs")
        .or_else(|| value.get("createdAtUnixMs").and_then(Value::as_u64))?;
    Some(WindowsCloudCleanupDeleteMarker {
        path,
        created_at_unix_ms,
    })
}

fn write_windows_cloud_cleanup_delete_markers(
    config_dir: &Path,
    markers: &[WindowsCloudCleanupDeleteMarker],
) {
    let path = config_dir.join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE);
    if markers.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if std::fs::create_dir_all(config_dir).is_err() {
        return;
    }
    let value = json!({ "entries": markers });
    if let Ok(raw) = serde_json::to_string(&value) {
        let _ = std::fs::write(path, raw);
    }
}

fn append_windows_cloud_cleanup_delete_markers(
    config_dir: &Path,
    markers: &[WindowsCloudCleanupDeleteMarker],
) {
    if markers.is_empty() {
        return;
    }

    let marker_path = config_dir.join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE);
    let mut retained = Vec::new();
    if let Ok(raw) = std::fs::read_to_string(&marker_path)
        && let Ok(value) = serde_json::from_str::<Value>(&raw)
        && let Some(entries) = value.get("entries").and_then(Value::as_array)
    {
        let now_ms = windows_cloud_cleanup_marker_now_ms();
        let min_created_at_ms =
            now_ms.saturating_sub(WINDOWS_CLOUD_CLEANUP_DELETE_MARKER_SECS * 1_000);
        retained.extend(
            entries
                .iter()
                .filter_map(windows_cloud_cleanup_delete_marker_from_json)
                .filter(|marker| marker.created_at_unix_ms >= min_created_at_ms),
        );
    }

    retained.extend_from_slice(markers);
    retained.sort_by(|a, b| a.path.cmp(&b.path));
    retained.dedup_by(|a, b| a.path == b.path);
    write_windows_cloud_cleanup_delete_markers(config_dir, &retained);
}

fn windows_cloud_cleanup_marker_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

const WINDOWS_CLOUD_LOCAL_STATE_FILE: &str = "windows-cloud-local-state.json";
const WINDOWS_CLOUD_CLEANUP_DELETE_FILE: &str = "windows-cloud-cleanup-deletes.json";
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_CLOUD_LOCAL_STATE_VALIDATE_INTERVAL_SECS: u64 = 2;
const WINDOWS_CLOUD_CLEANUP_DELETE_MARKER_SECS: u64 = 30;
