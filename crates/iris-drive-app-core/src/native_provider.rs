use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use hashtree_provider::{HashTreeProviderFs, ItemKind, ProviderFs};
use iris_drive_core::config::DEFAULT_RELAYS;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::provider::{
    ProviderListEntry, normalize_provider_document_path, normalize_provider_parent_path,
    normalize_provider_path, optional_normalized_provider_path, provider_list_summary,
    provider_path_is_child_document, sanitized_provider_file_name, split_provider_path,
    unique_provider_path,
};
use iris_drive_core::{Account, AppConfig};
use serde_json::json;

use crate::provider_metadata::provider_modified_at_index;

const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[25, 50, 100, 200, 400];
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;

pub(crate) fn native_provider_list_json(data_dir: &str) -> serde_json::Value {
    match run_native_provider_list(data_dir) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_read_json(
    data_dir: &str,
    path: &str,
    output_path: &str,
) -> serde_json::Value {
    match run_native_provider_read(data_dir, path, output_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_write_json(
    data_dir: &str,
    path: &str,
    source_path: &str,
) -> serde_json::Value {
    match run_native_provider_write(data_dir, path, source_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_mkdir_json(data_dir: &str, path: &str) -> serde_json::Value {
    match run_native_provider_mkdir(data_dir, path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_delete_json(data_dir: &str, path: &str) -> serde_json::Value {
    match run_native_provider_delete(data_dir, path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_rename_json(
    data_dir: &str,
    old_path: &str,
    new_path: &str,
) -> serde_json::Value {
    match run_native_provider_rename(data_dir, old_path, new_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_import_shared_file_json(
    data_dir: &str,
    display_name: &str,
    source_path: &str,
) -> serde_json::Value {
    match native_provider_import_shared_file(data_dir, display_name, source_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_resolve_path_json(
    data_dir: &str,
    parent_path: &str,
    display_name: &str,
    excluding_path: &str,
) -> serde_json::Value {
    match run_native_provider_resolve_path(data_dir, parent_path, display_name, excluding_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_normalize_path_json(path: &str) -> serde_json::Value {
    match run_native_provider_normalize_path(path) {
        Ok(value) => value,
        Err(error) => json!({
            "path": "",
            "parent_path": "",
            "display_name": "",
            "error": format!("{error:#}"),
        }),
    }
}

pub(crate) fn native_provider_is_child_document_json(
    parent_path: &str,
    document_path: &str,
) -> serde_json::Value {
    match provider_path_is_child_document(parent_path, document_path) {
        Ok(is_child) => json!({
            "is_child": is_child,
            "error": "",
        }),
        Err(error) => json!({
            "is_child": false,
            "error": format!("{error:#}"),
        }),
    }
}

pub(crate) fn native_provider_import_shared_file(
    data_dir: &str,
    display_name: &str,
    source_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let display_name = sanitized_provider_file_name(display_name);
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        let modified_at_by_path = BTreeMap::new();
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        let path = unique_provider_path(&entries, "", &display_name, None);
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        write_provider_file(&provider, &path, &bytes).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_normalize_path(path: &str) -> anyhow::Result<serde_json::Value> {
    let path = normalize_provider_document_path(path)?;
    let (parent_path, display_name) = split_provider_path(&path)?;
    Ok(json!({
        "parent_path": parent_path,
        "display_name": display_name,
        "path": path,
        "error": "",
    }))
}

fn run_native_provider_resolve_path(
    data_dir: &str,
    parent_path: &str,
    display_name: &str,
    excluding_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let parent_path = normalize_provider_parent_path(parent_path)?;
        let display_name = sanitized_provider_file_name(display_name);
        let excluding_path = optional_normalized_provider_path(excluding_path)?;
        let (_daemon, provider, _visible_root) = native_provider(data_dir).await?;
        let modified_at_by_path = BTreeMap::new();
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        let path = unique_provider_path(
            &entries,
            &parent_path,
            &display_name,
            excluding_path.as_deref(),
        );
        let (resolved_parent_path, resolved_display_name) = split_provider_path(&path)?;
        Ok(json!({
            "parent_path": resolved_parent_path,
            "display_name": resolved_display_name,
            "path": path,
            "error": "",
        }))
    })
}

pub(crate) fn run_native_provider_list(data_dir: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let daemon = iris_drive_core::Daemon::open(data_dir)
            .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
        let visible_view = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .context("building provider view")?;
        let modified_at_by_path = provider_modified_at_index(&visible_view);
        let visible_root = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("building provider root")?;
        let provider =
            HashTreeProviderFs::open(daemon.tree_handle(), visible_root.root_cid.clone())
                .await
                .context("opening provider root")?;
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        let summary = provider_list_summary(provider.anchor().await.as_str(), &entries);
        Ok(json!({
            "anchor": provider.anchor().await.as_str(),
            "root_cid": visible_root.root_cid.to_string(),
            "file_count": summary.file_count,
            "visible_file_bytes": summary.visible_file_bytes,
            "directory_paths": summary.directory_paths,
            "change_key": summary.change_key,
            "entries": entries,
        }))
    })
}

fn run_native_provider_read(
    data_dir: &str,
    path: &str,
    output_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (_daemon, provider, _visible_root) = native_provider(data_dir).await?;
        let item = provider.item(&path).await?;
        if item.kind == ItemKind::Directory {
            anyhow::bail!("cannot read directory: {path}");
        }
        let bytes = provider.read(&path, 0, item.size).await?;
        let output = PathBuf::from(output_path);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&output, bytes).with_context(|| format!("writing {}", output.display()))?;
        Ok(json!({
            "path": path,
            "output": output.display().to_string(),
            "size": item.size,
        }))
    })
}

fn run_native_provider_write(
    data_dir: &str,
    path: &str,
    source_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        write_provider_file(&provider, &path, &bytes).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_mkdir(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        create_provider_dir(&provider, &path).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_delete(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        delete_provider_path(&provider, &path).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_rename(
    data_dir: &str,
    old_path: &str,
    new_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let old_path = normalize_provider_path(old_path)?;
        let new_path = normalize_provider_path(new_path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        rename_provider_path(&provider, &old_path, &new_path).await?;
        import_provider_mutation(&mut daemon, &provider, &new_path, Some(visible_root)).await
    })
}

fn native_provider_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    install_rustls_crypto_provider();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building native provider runtime")
}

pub(crate) fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn native_provider(
    data_dir: &str,
) -> anyhow::Result<(
    iris_drive_core::Daemon,
    HashTreeProviderFs<hashtree_fs::FsBlobStore>,
    hashtree_core::Cid,
)> {
    let daemon = iris_drive_core::Daemon::open(data_dir)
        .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building provider root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone())
        .await
        .context("opening provider root")?;
    Ok((daemon, provider, visible.root_cid))
}

async fn provider_entries<P>(
    provider: &P,
    modified_at_by_path: &BTreeMap<String, i64>,
) -> anyhow::Result<Vec<ProviderListEntry>>
where
    P: ProviderFs<ItemId = String>,
{
    let mut entries = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent) = stack.pop() {
        let mut children = provider.read_dir(&parent).await?;
        children.sort_by(|left, right| left.name.cmp(&right.name));
        for child in children {
            let item = provider.item(&child.id).await?;
            let kind = match item.kind {
                ItemKind::Directory => {
                    stack.push(child.id.clone());
                    "directory"
                }
                ItemKind::File => "file",
            };
            let modified_at = modified_at_by_path.get(&child.id).copied();
            entries.push(ProviderListEntry {
                path: child.id,
                parent_path: parent.clone(),
                display_name: child.name,
                kind,
                size: item.size,
                version: provider.anchor().await.as_str().to_owned(),
                modified_at,
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

async fn import_provider_mutation<P>(
    daemon: &mut iris_drive_core::Daemon,
    provider: &P,
    changed_path: &str,
    tombstone_base_root: Option<hashtree_core::Cid>,
) -> anyhow::Result<serde_json::Value>
where
    P: ProviderFs<ItemId = String>,
{
    let root = hashtree_core::Cid::parse(provider.anchor().await.as_str())
        .context("reading provider root CID")?;
    let report = import_provider_root_with_retry(daemon, root, tombstone_base_root).await?;
    let publish = publish_current_device_root_best_effort(daemon.config_dir()).await;
    Ok(json!({
        "path": changed_path,
        "root_cid": report.root_cid,
        "file_count": report.file_count,
        "top_level_entries": report.top_level_entries,
        "publish": publish,
    }))
}

async fn import_provider_root_with_retry(
    daemon: &mut iris_drive_core::Daemon,
    root: hashtree_core::Cid,
    tombstone_base_root: Option<hashtree_core::Cid>,
) -> anyhow::Result<iris_drive_core::ImportReport> {
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
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn publish_current_device_root_best_effort(config_dir: &Path) -> serde_json::Value {
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        publish_current_device_root(config_dir),
    )
    .await
    {
        Ok(Ok(published)) => published,
        Ok(Err(error)) => json!({"published_drive_root": false, "error": format!("{error:#}")}),
        Err(_) => json!({"published_drive_root": false, "error": "publish timed out"}),
    }
}

async fn publish_current_device_root(config_dir: &Path) -> anyhow::Result<serde_json::Value> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(account) = config.account.as_ref() else {
        return Ok(json!({"published_drive_root": false, "error": "account missing"}));
    };
    let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
        return Ok(json!({"published_drive_root": false, "error": "primary drive missing"}));
    };
    let Some(root) = drive.device_roots.get(&account.device_pubkey) else {
        return Ok(json!({"published_drive_root": false, "error": "device root missing"}));
    };
    let loaded_account =
        Account::load(account.clone(), config_dir).context("loading account keys")?;

    let relays = if config.relays.is_empty() {
        default_relays()
    } else {
        config.relays.clone()
    };
    let client = iris_drive_core::relay_sync::connect(&relays).await?;
    let authorized_devices = authorized_device_pubkeys(account);
    let result = iris_drive_core::relay_sync::publish_drive_root(
        &client,
        loaded_account.device.keys(),
        &account.owner_pubkey,
        &drive.drive_id,
        root,
        &authorized_devices,
    )
    .await;
    let _ = client.disconnect().await;
    let event_id = result?;
    Ok(json!({
        "published_drive_root": true,
        "drive_root_event_id": event_id.to_hex(),
    }))
}

pub(crate) fn run_native_sync_once(
    data_dir: &str,
) -> anyhow::Result<iris_drive_core::NetworkSyncReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(iris_drive_core::network_sync_once(
        Path::new(data_dir),
        &[],
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
    ))
}

#[cfg(test)]
pub(crate) fn run_native_sync_once_with_drive_root_events_for_test(
    config_dir: &Path,
    events: &[nostr_sdk::Event],
) -> anyhow::Result<iris_drive_core::DriveRootEventApplyReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let report = iris_drive_core::apply_drive_root_events(config_dir, &mut config, events)?;
        config.save(config_path_in(config_dir))?;
        Ok(report)
    })
}

pub(crate) fn native_sync_status_label(
    report: &iris_drive_core::NetworkSyncReport,
) -> &'static str {
    if report.fips_download.is_some() || report.blossom_download.is_some() {
        "synced"
    } else if report.drive_root_events_applied > 0 || report.files_root_event_outcome == "applied" {
        "root synced"
    } else {
        "up to date"
    }
}

fn authorized_device_pubkeys(state: &iris_drive_core::AccountState) -> Vec<String> {
    let mut devices: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| {
            snap.devices
                .iter()
                .map(|device| device.pubkey.clone())
                .collect()
        })
        .unwrap_or_default();
    if !devices.contains(&state.device_pubkey) {
        devices.push(state.device_pubkey.clone());
    }
    devices
}

async fn write_provider_file<P>(provider: &P, path: &str, bytes: &[u8]) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_owned()).await {
        Ok(item) if item.kind == ItemKind::Directory => {
            delete_provider_path(provider, path).await?;
            provider.create_file(&parent, &name).await?;
        }
        Ok(_) => {
            provider.truncate(&path.to_owned(), 0).await?;
        }
        Err(_) => {
            provider.create_file(&parent, &name).await?;
        }
    }
    if !bytes.is_empty() {
        provider.write(&path.to_owned(), 0, bytes).await?;
    }
    Ok(())
}

async fn create_provider_dir<P>(provider: &P, path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_owned()).await {
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

async fn ensure_provider_dirs<P>(provider: &P, parent: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let mut current = String::new();
    for segment in parent.split('/').filter(|segment| !segment.is_empty()) {
        let next = if current.is_empty() {
            segment.to_owned()
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

async fn delete_provider_path<P>(provider: &P, path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let mut directories = Vec::new();
    let mut stack = vec![path.to_owned()];
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

async fn rename_provider_path<P>(provider: &P, old_path: &str, new_path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (old_parent, old_name) = split_provider_path(old_path)?;
    let (new_parent, new_name) = split_provider_path(new_path)?;
    ensure_provider_dirs(provider, &new_parent).await?;
    if provider.item(&new_path.to_owned()).await.is_ok() {
        delete_provider_path(provider, new_path).await?;
    }
    provider
        .rename(&old_parent, &old_name, &new_parent, &new_name)
        .await?;
    Ok(())
}

fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("block not found")
        || message.contains("missing block")
        || message.contains("No such file or directory")
}

fn default_relays() -> Vec<String> {
    DEFAULT_RELAYS
        .iter()
        .map(|relay| (*relay).to_owned())
        .collect()
}
