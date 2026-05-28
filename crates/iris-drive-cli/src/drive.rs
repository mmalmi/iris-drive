#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_drives(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if config.drives.is_empty() {
        println!("(no drives — run `idrive init`)");
        return Ok(());
    }
    for d in &config.drives {
        println!(
            "{:<24}  {:<7}  {:<32}  {}",
            d.drive_id,
            drive_role_label(d.role),
            short_pubkey(&d.owner_pubkey),
            d.display_name,
        );
    }
    Ok(())
}

pub(crate) fn cmd_import(config_dir: &std::path::Path, source_dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = daemon
            .import_source_dir(source_dir)
            .await
            .with_context(|| format!("importing {}", source_dir.display()))?;
        let reported_source = report.source_dir.as_deref().unwrap_or(source_dir);
        println!(
            "{}",
            json!({
                "source_dir": reported_source.display().to_string(),
                "root_cid": report.root_cid,
                "drive_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                "files_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                "snapshot_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                "permalink_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                "local_gateway": local_gateway_urls_for_root(
                    Some(&report.root_cid),
                    DEFAULT_GATEWAY_PORT,
                    daemon.config().local_nhash_resolver_enabled,
                ),
                "file_count": report.file_count,
                "top_level_entries": report.top_level_entries,
                "blocks_dir": daemon.blocks_dir().display().to_string(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

pub(crate) fn cmd_materialize(
    config_dir: &std::path::Path,
    target_dir: &std::path::Path,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = iris_drive_core::materialize_primary_drive(
            daemon.tree_handle(),
            daemon.config(),
            target_dir,
        )
        .await
        .with_context(|| format!("materializing {}", target_dir.display()))?;
        println!(
            "{}",
            json!({
                "target_dir": target_dir.display().to_string(),
                "written": report.written,
                "updated": report.updated,
                "deleted": report.deleted,
                "unchanged": report.unchanged,
                "skipped": report.skipped,
                "changed": report.changed(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

pub(crate) fn cmd_list(config_dir: &std::path::Path, at: usize) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let config = daemon.config();
        let drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
        let account = config
            .account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account; run `idrive init` first"))?;
        let authorized = authorized_device_pubkeys(account);

        // Fetch each authorized device's tree + tombstones from htree.
        // With `--at N`, this device's own root walks back N revisions
        // via the `.hashtree/prev` chain; other devices' roots stay at their
        // current state.
        let mut snapshots_data = Vec::new();
        for device_pubkey in &authorized {
            let Some(root) = drive.device_roots.get(device_pubkey) else {
                continue; // device hasn't published its root yet
            };
            let mut cid = Cid::parse(&root.root_cid)
                .with_context(|| format!("parsing root CID for device {device_pubkey}"))?;
            if at > 0 && *device_pubkey == account.device_pubkey {
                cid = iris_drive_core::history::revision_at(daemon.tree(), &cid, at)
                    .await
                    .with_context(|| format!("revision -{at} not in chain"))?;
            }
            let (files, tombstones) = walk_device_tree(daemon.tree(), &cid).await?;
            snapshots_data.push((device_pubkey.clone(), root.clone(), files, tombstones));
        }

        let authorized_refs: Vec<&str> = authorized.iter().map(String::as_str).collect();
        let snapshots: Vec<DeviceSnapshot> = snapshots_data
            .iter()
            .map(|(pk, root, files, tombs)| DeviceSnapshot {
                device_pubkey: pk.as_str(),
                root,
                files: files.clone(),
                tombstones: tombs.clone(),
            })
            .collect();

        let view = merge_drives(&authorized_refs, &snapshots);

        println!(
            "{}",
            json!({
                "drive_id": drive.drive_id,
                "at_revision": at,
                "authorized_devices": authorized.len(),
                "device_roots_present": snapshots.len(),
                "files": view
                    .files
                    .iter()
                    .map(|e| json!({
                        "path": e.path,
                        "size": e.size,
                        "sha256": e.whole_file_hash.unwrap_or(e.hash).map(|byte| format!("{byte:02x}")).join(""),
                        "source_device": e.source_device,
                        "published_at": e.published_at,
                    }))
                    .collect::<Vec<_>>(),
                "suppressed_by_tombstone": view.suppressed_by_tombstone,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[derive(Debug, Serialize)]
struct ProviderListEntry {
    path: String,
    kind: &'static str,
    size: u64,
    version: String,
}

pub(crate) fn cmd_provider(config_dir: &std::path::Path, command: ProviderCmd) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let started = std::time::Instant::now();
        let _config_lock = if matches!(
            &command,
            ProviderCmd::Write { .. }
                | ProviderCmd::Mkdir { .. }
                | ProviderCmd::Delete { .. }
                | ProviderCmd::Rename { .. }
        ) {
            Some(ConfigMutationLock::acquire(config_dir).await?)
        } else {
            None
        };
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis(),
            "provider command opened daemon"
        );
        let phase = std::time::Instant::now();
        let visible = primary_merged_root_with_retry(&daemon)
            .await
            .context("building virtual provider root")?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "provider command built merged root"
        );
        let phase = std::time::Instant::now();
        let provider =
            HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone()).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "provider command opened provider root"
        );

        match command {
            ProviderCmd::List => {
                let phase = std::time::Instant::now();
                let entries = provider_entries(daemon.tree(), &visible.root_cid).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command listed entries"
                );
                println!(
                    "{}",
                    json!({
                        "anchor": provider.anchor().await.as_str(),
                        "root_cid": visible.root_cid.to_string(),
                        "file_count": visible.file_count,
                        "top_level_entries": visible.top_level_entries,
                        "entries": entries,
                    })
                );
            }
            ProviderCmd::Read { path, output } => {
                let path = normalize_provider_path(&path)?;
                let item = provider.item(&path).await?;
                if item.kind == ItemKind::Directory {
                    anyhow::bail!("cannot read directory: {path}");
                }
                let bytes = provider.read(&path, 0, item.size).await?;
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                std::fs::write(&output, bytes)
                    .with_context(|| format!("writing {}", output.display()))?;
                println!(
                    "{}",
                    json!({
                        "path": path,
                        "output": output.display().to_string(),
                        "size": item.size,
                    })
                );
            }
            ProviderCmd::MaterializeCache { dir } => {
                let phase = std::time::Instant::now();
                let entries = provider_entries(daemon.tree(), &visible.root_cid).await?;
                let report = materialize_provider_cache(&provider, &entries, &dir).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command materialized private cache"
                );
                println!(
                    "{}",
                    json!({
                        "target_dir": dir.display().to_string(),
                        "file_count": report.file_count,
                        "directory_count": report.directory_count,
                        "written": report.written,
                        "updated": report.updated,
                        "unchanged": report.unchanged,
                        "skipped": report.skipped,
                        "changed": report.written + report.updated,
                    })
                );
            }
            ProviderCmd::Write { path, source } => {
                let path = normalize_provider_path(&path)?;
                let bytes = std::fs::read(&source)
                    .with_context(|| format!("reading {}", source.display()))?;
                let phase = std::time::Instant::now();
                write_provider_file(&provider, &path, &bytes).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command wrote file"
                );
                print_provider_mutation(
                    &mut daemon,
                    &provider,
                    &path,
                    Some(visible.root_cid.clone()),
                )
                .await?;
            }
            ProviderCmd::Mkdir { path } => {
                let path = normalize_provider_path(&path)?;
                let phase = std::time::Instant::now();
                create_provider_dir(&provider, &path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command created directory"
                );
                print_provider_mutation(
                    &mut daemon,
                    &provider,
                    &path,
                    Some(visible.root_cid.clone()),
                )
                .await?;
            }
            ProviderCmd::Delete { path } => {
                let path = normalize_provider_path(&path)?;
                let phase = std::time::Instant::now();
                delete_provider_path(&provider, &path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command deleted path"
                );
                print_provider_mutation(
                    &mut daemon,
                    &provider,
                    &path,
                    Some(visible.root_cid.clone()),
                )
                .await?;
            }
            ProviderCmd::Rename { old_path, new_path } => {
                let old_path = normalize_provider_path(&old_path)?;
                let new_path = normalize_provider_path(&new_path)?;
                let phase = std::time::Instant::now();
                rename_provider_path(&provider, &old_path, &new_path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command renamed path"
                );
                print_provider_mutation(
                    &mut daemon,
                    &provider,
                    &new_path,
                    Some(visible.root_cid.clone()),
                )
                .await?;
            }
        }

        Ok::<_, anyhow::Error>(())
    })
}

async fn provider_entries(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
) -> Result<Vec<ProviderListEntry>> {
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
            entries.push(ProviderListEntry {
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

#[derive(Default)]
struct ProviderCacheReport {
    file_count: usize,
    directory_count: usize,
    written: usize,
    updated: usize,
    unchanged: usize,
    skipped: usize,
}

async fn materialize_provider_cache(
    provider: &HashTreeProviderFs<FsBlobStore>,
    entries: &[ProviderListEntry],
    target_dir: &Path,
) -> Result<ProviderCacheReport> {
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("creating {}", target_dir.display()))?;
    let mut report = ProviderCacheReport::default();
    for entry in entries {
        let Some(destination) = provider_cache_destination(target_dir, &entry.path) else {
            report.skipped += 1;
            continue;
        };
        if entry.kind == "directory" {
            report.directory_count += 1;
            if destination.is_dir() {
                report.unchanged += 1;
                continue;
            }
            let existed = destination.exists();
            remove_provider_cache_destination(&destination)?;
            std::fs::create_dir_all(&destination)
                .with_context(|| format!("creating {}", destination.display()))?;
            if existed {
                report.updated += 1;
            } else {
                report.written += 1;
            }
            continue;
        }

        report.file_count += 1;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let bytes = provider.read(&entry.path, 0, entry.size).await?;
        if destination.is_file() {
            let existing = std::fs::read(&destination)
                .with_context(|| format!("reading {}", destination.display()))?;
            if existing == bytes {
                report.unchanged += 1;
                continue;
            }
            std::fs::write(&destination, bytes)
                .with_context(|| format!("writing {}", destination.display()))?;
            report.updated += 1;
            continue;
        }

        let existed = destination.exists();
        remove_provider_cache_destination(&destination)?;
        std::fs::write(&destination, bytes)
            .with_context(|| format!("writing {}", destination.display()))?;
        if existed {
            report.updated += 1;
        } else {
            report.written += 1;
        }
    }
    Ok(report)
}

fn provider_cache_destination(
    target_dir: &Path,
    provider_path: &str,
) -> Option<std::path::PathBuf> {
    let mut destination = target_dir.to_path_buf();
    for segment in provider_path.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            return None;
        }
        destination.push(segment);
    }
    Some(destination)
}

fn remove_provider_cache_destination(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    } else if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

pub(crate) async fn write_provider_file(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
    bytes: &[u8],
) -> Result<()> {
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_string()).await {
        Ok(item) if item.kind == ItemKind::Directory => {
            delete_provider_path(provider, path).await?;
            provider.create_file(&parent, &name).await?;
        }
        Ok(_) => {
            provider.truncate(&path.to_string(), 0).await?;
        }
        Err(_) => {
            provider.create_file(&parent, &name).await?;
        }
    }
    if !bytes.is_empty() {
        provider.write(&path.to_string(), 0, bytes).await?;
    }
    Ok(())
}

pub(crate) async fn create_provider_dir(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<()> {
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_string()).await {
        Ok(item) if item.kind == ItemKind::Directory => Ok(()),
        Ok(_) => {
            provider.remove(&parent, &name).await?;
            provider.create_dir(&parent, &name).await?;
            Ok(())
        }
        Err(_) => {
            provider.create_dir(&parent, &name).await?;
            Ok(())
        }
    }
}

async fn ensure_provider_dirs(
    provider: &HashTreeProviderFs<FsBlobStore>,
    parent: &str,
) -> Result<()> {
    let mut current = String::new();
    for segment in parent.split('/').filter(|segment| !segment.is_empty()) {
        let next = if current.is_empty() {
            segment.to_string()
        } else {
            format!("{current}/{segment}")
        };
        match provider.item(&next).await {
            Ok(item) if item.kind == ItemKind::Directory => {}
            Ok(_) => {
                provider.remove(&current, segment).await?;
                provider.create_dir(&current, segment).await?;
            }
            Err(_) => {
                provider.create_dir(&current, segment).await?;
            }
        }
        current = next;
    }
    Ok(())
}

pub(crate) async fn delete_provider_path(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<()> {
    let root = path.to_string();
    let mut stack = vec![root.clone()];
    let mut directories = Vec::new();
    while let Some(current) = stack.pop() {
        let item = match provider.item(&current).await {
            Ok(item) => item,
            Err(hashtree_provider::ProviderError::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        if item.kind == ItemKind::Directory {
            directories.push(current.clone());
            for child in provider.read_dir(&current).await? {
                stack.push(child.id);
            }
        } else {
            let (parent, name) = split_provider_path(&current)?;
            match provider.remove(&parent, &name).await {
                Ok(()) | Err(hashtree_provider::ProviderError::NotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    for directory in directories.into_iter().rev() {
        let (parent, name) = split_provider_path(&directory)?;
        match provider.remove(&parent, &name).await {
            Ok(()) | Err(hashtree_provider::ProviderError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[250, 500, 1_000, 2_000, 4_000, 8_000];

async fn primary_merged_root_with_retry(
    daemon: &Daemon,
) -> Result<iris_drive_core::PrimaryMergedRoot> {
    let mut attempt = 0;
    loop {
        match iris_drive_core::primary_merged_root(daemon.tree(), daemon.config()).await {
            Ok(root) => return Ok(root),
            Err(error)
                if attempt < PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
                    && provider_import_error_message_is_retryable(&error.to_string()) =>
            {
                let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
                attempt += 1;
                tracing::warn!(
                    error = %error,
                    delay_ms,
                    "provider command hit a transient store read; retrying merged root build"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) async fn rename_provider_path(
    provider: &HashTreeProviderFs<FsBlobStore>,
    old_path: &str,
    new_path: &str,
) -> Result<()> {
    if old_path == new_path {
        return Ok(());
    }
    let (old_parent, old_name) = split_provider_path(old_path)?;
    let (new_parent, new_name) = split_provider_path(new_path)?;
    ensure_provider_dirs(provider, &new_parent).await?;
    if provider.item(&new_path.to_string()).await.is_ok() {
        delete_provider_path(provider, new_path).await?;
    }
    provider
        .rename(&old_parent, &old_name, &new_parent, &new_name)
        .await?;
    Ok(())
}

async fn print_provider_mutation(
    daemon: &mut Daemon,
    provider: &HashTreeProviderFs<FsBlobStore>,
    changed_path: &str,
    tombstone_base_root: Option<Cid>,
) -> Result<()> {
    let phase = std::time::Instant::now();
    let root = provider.current_root().await;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        "provider command read current root"
    );
    let phase = std::time::Instant::now();
    let report = import_provider_root_with_retry(daemon, root, tombstone_base_root).await?;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        "provider command imported provider root"
    );
    println!(
        "{}",
        json!({
            "path": changed_path,
            "root_cid": report.root_cid,
            "file_count": report.file_count,
            "top_level_entries": report.top_level_entries,
        })
    );
    Ok(())
}

async fn import_provider_root_with_retry(
    daemon: &mut Daemon,
    root: Cid,
    tombstone_base_root: Option<Cid>,
) -> Result<iris_drive_core::daemon::ImportReport> {
    let mut attempt = 0;
    loop {
        match daemon
            .import_visible_root_with_tombstone_base(root.clone(), tombstone_base_root.clone())
            .await
        {
            Ok(report) => return Ok(report),
            Err(error)
                if attempt < PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
                    && provider_import_error_message_is_retryable(&error.to_string()) =>
            {
                let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
                attempt += 1;
                tracing::warn!(
                    error = %error,
                    delay_ms,
                    "provider import hit a transient store read; retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("Store error")
        && (message.contains("os error 2")
            || message.contains("No such file or directory")
            || message.contains("The system cannot find the file specified"))
        || message.contains("Missing chunk")
}

pub(crate) fn normalize_provider_path(path: &str) -> Result<String> {
    let trimmed = path.trim_matches('/');
    let parts = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if segment == "." || segment == ".." {
                anyhow::bail!("invalid virtual path component: {segment}");
            }
            Ok(segment)
        })
        .collect::<Result<Vec<_>>>()?;
    if parts.is_empty() {
        anyhow::bail!("virtual path must not be empty");
    }
    Ok(parts.join("/"))
}

fn split_provider_path(path: &str) -> Result<(String, String)> {
    let path = normalize_provider_path(path)?;
    let mut parts = path.rsplitn(2, '/');
    let name = parts.next().unwrap_or_default().to_string();
    let parent = parts.next().unwrap_or_default().to_string();
    Ok((parent, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iris_drive_core::root_meta::DriveRootMeta;

    fn init_config(config_dir: &Path) -> Account {
        let account = Account::create(config_dir, Some("local".into())).unwrap();
        let mut config = AppConfig {
            account: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_drive(Drive::primary(account.state.owner_pubkey.clone()));
        config.save(config_path_in(config_dir)).unwrap();
        account
    }

    fn init_config_with_remote_device(config_dir: &Path) -> (Account, String, DriveRootMeta) {
        let account = init_config(config_dir);
        let remote = iris_drive_core::identity::Identity::generate(config_dir.join("remote.key"))
            .pubkey_hex();
        let mut config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
        let state = config.account.as_mut().unwrap();
        state
            .app_keys
            .as_mut()
            .unwrap()
            .devices
            .push(iris_drive_core::app_keys::DeviceEntry {
                pubkey: remote.clone(),
                added_at: 100,
                label: Some("remote".into()),
            });
        state.app_keys.as_mut().unwrap().normalize();
        config.save(config_path_in(config_dir)).unwrap();

        let remote_meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: PRIMARY_DRIVE_ID.into(),
            device_id: remote.clone(),
            device_seq: 1,
            dck_generation: 1,
            materialized_only: false,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            created_at: 100,
        };
        (account, remote, remote_meta)
    }

    #[test]
    fn provider_delete_local_file_is_idempotent() {
        let config_dir = tempfile::tempdir().unwrap();
        init_config(config_dir.path());
        let source_dir = tempfile::tempdir().unwrap();
        std::fs::write(source_dir.path().join("juuh.txt"), b"delete me").unwrap();
        cmd_import(config_dir.path(), source_dir.path()).unwrap();

        cmd_provider(
            config_dir.path(),
            ProviderCmd::Delete {
                path: "juuh.txt".into(),
            },
        )
        .unwrap();
        cmd_provider(
            config_dir.path(),
            ProviderCmd::Delete {
                path: "juuh.txt".into(),
            },
        )
        .unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let daemon = Daemon::open(config_dir.path()).unwrap();
            let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
                .await
                .unwrap();
            assert!(
                merged
                    .view
                    .files
                    .iter()
                    .all(|entry| entry.path != "juuh.txt"),
                "deleted file should not reappear in the merged view"
            );
        });
    }

    #[test]
    fn provider_delete_directory_removes_tree() {
        let config_dir = tempfile::tempdir().unwrap();
        init_config(config_dir.path());
        let source_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(source_dir.path().join("folder")).unwrap();
        std::fs::write(
            source_dir.path().join("folder").join("child.txt"),
            b"delete me",
        )
        .unwrap();
        cmd_import(config_dir.path(), source_dir.path()).unwrap();

        cmd_provider(
            config_dir.path(),
            ProviderCmd::Delete {
                path: "folder".into(),
            },
        )
        .unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let daemon = Daemon::open(config_dir.path()).unwrap();
            let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
                .await
                .unwrap();
            assert!(
                merged
                    .view
                    .files
                    .iter()
                    .all(|entry| !entry.path.starts_with("folder/")),
                "deleted directory children should not remain in the merged view"
            );
        });
    }

    #[test]
    fn provider_delete_tombstones_foreign_visible_files() {
        let config_dir = tempfile::tempdir().unwrap();
        let (_account, remote, remote_meta) = init_config_with_remote_device(config_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let remote_root = runtime.block_on(async {
            let daemon = Daemon::open(config_dir.path()).unwrap();
            let remote_dir = tempfile::tempdir().unwrap();
            std::fs::write(remote_dir.path().join("foreign.txt"), b"from remote").unwrap();
            iris_drive_core::indexer::index_dir_with_history_and_meta(
                daemon.tree(),
                remote_dir.path(),
                None,
                remote_meta.created_at,
                Some(&remote_meta),
            )
            .await
            .unwrap()
        });

        let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
        drive.device_roots.insert(
            remote.clone(),
            DeviceRootRef::from_meta(
                remote_root.to_string(),
                remote_meta.created_at,
                &remote_meta,
            ),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_provider(
            config_dir.path(),
            ProviderCmd::Delete {
                path: "foreign.txt".into(),
            },
        )
        .unwrap();

        runtime.block_on(async {
            let daemon = Daemon::open(config_dir.path()).unwrap();
            let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
                .await
                .unwrap();
            assert!(
                merged
                    .view
                    .files
                    .iter()
                    .all(|entry| entry.path != "foreign.txt")
            );
            assert_eq!(
                merged.view.suppressed_by_tombstone,
                vec!["foreign.txt".to_string()]
            );
        });
    }

    #[test]
    fn provider_import_retries_windows_transient_missing_store_reads() {
        assert!(provider_import_error_message_is_retryable(
            "index: tree: Store error: IO error: The system cannot find the file specified. (os error 2)"
        ));
        assert!(provider_import_error_message_is_retryable(
            "index: tree: Store error: IO error: No such file or directory (os error 2)"
        ));
        assert!(provider_import_error_message_is_retryable(
            "index: tree: Missing chunk: abc123"
        ));
        assert!(!provider_import_error_message_is_retryable(
            "config: invalid json"
        ));
    }
}

pub(crate) async fn walk_device_tree(
    tree: &HashTree<hashtree_fs::FsBlobStore>,
    root: &Cid,
) -> Result<(Vec<DeviceFileEntry>, Vec<DeviceTombstone>)> {
    iris_drive_core::merge::walk_device_tree(tree, root)
        .await
        .map_err(anyhow::Error::from)
}

pub(crate) fn cmd_relays(config_dir: &std::path::Path, sub: Option<RelaysCmd>) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub.unwrap_or(RelaysCmd::List) {
        RelaysCmd::List => {}
        RelaysCmd::Add { url } => {
            let url = normalize_relay_url(&url);
            if !config.relays.contains(&url) {
                config.relays.push(url);
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Update { old_url, new_url } => {
            let old_url = normalize_relay_url(&old_url);
            let new_url = normalize_relay_url(&new_url);
            let mut changed = false;
            for relay in &mut config.relays {
                if relay == &old_url {
                    relay.clone_from(&new_url);
                    changed = true;
                }
            }
            dedupe_relays(&mut config.relays);
            if changed {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Remove { url } => {
            let url = normalize_relay_url(&url);
            let before = config.relays.len();
            config.relays.retain(|s| s != &url);
            if config.relays.len() != before {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Reset => {
            config.relays = iris_drive_core::config::DEFAULT_RELAYS
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            config.save(config_path_in(config_dir))?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&config.relays)?);
    Ok(())
}

pub(crate) fn normalize_relay_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        trimmed.to_string()
    } else {
        format!("wss://{trimmed}")
    }
}

pub(crate) fn dedupe_relays(relays: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    relays.retain(|relay| seen.insert(relay.clone()));
}

pub(crate) fn cmd_history(config_dir: &std::path::Path, limit: usize) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let config = daemon.config();
        let account = config
            .account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account; run `idrive init` first"))?;
        let drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
        let Some(root_ref) = drive.device_roots.get(&account.device_pubkey) else {
            println!("{}", json!({"revisions": [], "note": "no imports yet"}));
            return Ok::<_, anyhow::Error>(());
        };
        let root_cid = Cid::parse(&root_ref.root_cid)
            .with_context(|| format!("parsing root cid {}", root_ref.root_cid))?;
        let chain = iris_drive_core::history::walk_history(daemon.tree(), &root_cid, limit)
            .await
            .context("walking history chain")?;

        let mut revisions = Vec::new();
        for (idx, cid) in chain.iter().enumerate() {
            // Count user-visible top-level entries (skip the .hashtree meta dir).
            let entries = daemon
                .tree()
                .list_directory(cid)
                .await
                .with_context(|| format!("listing rev {idx}"))?;
            let user_visible = entries
                .iter()
                .filter(|e| e.name != iris_drive_core::merge::META_DIR)
                .count();
            revisions.push(json!({
                "rev": idx,
                "root_cid": cid.to_string(),
                "top_level_entries": user_visible,
            }));
        }
        println!(
            "{}",
            json!({
                "device_pubkey": account.device_pubkey,
                "limit": limit,
                "chain_length": revisions.len(),
                "revisions": revisions,
            })
        );
        Ok(())
    })
}

pub(crate) fn cmd_event_app_keys(config_dir: &std::path::Path) -> Result<()> {
    let state = load_account_state(config_dir)?;
    let snap = state
        .app_keys
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no AppKeys snapshot yet (run `idrive init` first)"))?;
    if !state.has_owner_signing_authority {
        return Err(anyhow::anyhow!(
            "this device does not have owner-signing authority — only owner-capable installs can publish AppKeys"
        ));
    }
    let account = Account::load(state.clone(), config_dir).context("loading account")?;
    let owner_keys = account
        .owner_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
        .keys();
    let event = iris_drive_core::nostr_events::build_app_keys_event(owner_keys, snap)
        .context("building AppKeys event")?;
    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}

pub(crate) fn cmd_event_drive_root(config_dir: &std::path::Path) -> Result<()> {
    let state = load_account_state(config_dir)?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let drive = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
    let root_ref = drive
        .device_roots
        .get(&state.device_pubkey)
        .ok_or_else(|| {
            anyhow::anyhow!("no root for this device yet — run `idrive import <dir>` first")
        })?;
    let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let event = iris_drive_core::nostr_events::build_drive_root_event(
        device.keys(),
        &state.owner_pubkey,
        &drive.drive_id,
        root_ref,
        &authorized_device_pubkeys(&state),
    )
    .context("building drive-root event")?;
    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}

pub(crate) fn cmd_index(dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())));
        let cid = index_dir(&tree, dir)
            .await
            .with_context(|| format!("indexing {}", dir.display()))?;
        let listing = tree
            .list_directory(&cid)
            .await
            .context("listing top-level entries")?;
        println!(
            "{}",
            json!({
                "dir": dir.display().to_string(),
                "root_cid": cid.to_string(),
                "top_level_entries": listing.len(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}
