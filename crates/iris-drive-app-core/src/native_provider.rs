use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{Mutex, atomic::AtomicU64, atomic::Ordering};

use anyhow::Context;
use hashtree_provider::{HashTreeProviderFs, ItemKind, ProviderFs};
use iris_drive_core::config::DEFAULT_RELAYS;
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::provider::{
    ProviderListEntry, compose_provider_path, create_provider_dir, delete_provider_path,
    normalize_provider_document_path, normalize_provider_parent_path, normalize_provider_path,
    optional_normalized_provider_path, provider_entry_is_probable_os_placeholder,
    provider_file_probable_os_placeholder_family, provider_list_summary,
    provider_path_is_child_document, provider_write_is_probable_os_placeholder,
    rename_provider_path, sanitized_provider_file_name, split_provider_path, unique_provider_path,
    write_provider_file,
};
use iris_drive_core::{AppConfig, Profile};
use serde_json::json;

use crate::ffi::load_native_runtime_config_cached;
use crate::provider_metadata::provider_modified_at_index;

const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[25, 50, 100, 200, 400];
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;
const APPROVAL_ACK_FAST_SYNC_TIMEOUT_SECS: u64 = 3;

#[cfg(test)]
static PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderMutationLiveness {
    RequireDaemonLock,
    InProcessProvider,
}

pub(crate) fn provider_mutation_liveness_for_target(target_os: &str) -> ProviderMutationLiveness {
    match target_os {
        "android" | "ios" => ProviderMutationLiveness::InProcessProvider,
        _ => ProviderMutationLiveness::RequireDaemonLock,
    }
}

fn provider_mutation_liveness() -> ProviderMutationLiveness {
    provider_mutation_liveness_for_target(std::env::consts::OS)
}

pub(crate) fn native_sync_starts_direct_fips_for_target(target_os: &str) -> bool {
    !matches!(target_os, "android" | "ios")
}

fn native_sync_options() -> iris_drive_core::NetworkSyncOptions {
    iris_drive_core::NetworkSyncOptions {
        start_direct_fips_download: native_sync_starts_direct_fips_for_target(std::env::consts::OS),
    }
}

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

pub(crate) fn native_provider_compose_path_json(
    parent_path: &str,
    display_name: &str,
) -> serde_json::Value {
    match run_native_provider_compose_path(parent_path, display_name) {
        Ok(value) => value,
        Err(error) => json!({
            "path": "",
            "parent_path": "",
            "display_name": "",
            "error": format!("{error:#}"),
        }),
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
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let display_name = sanitized_provider_file_name(display_name);
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        import_provider_bytes(data_dir, &display_name, &bytes).await
    })
}

pub(crate) fn native_provider_import_content_link(
    data_dir: &str,
    link: &str,
) -> anyhow::Result<serde_json::Value> {
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let classification = iris_drive_core::classify_link_input(link);
    if !matches!(classification.kind.as_str(), "nhash_file" | "mutable_file") {
        anyhow::bail!("unsupported content link");
    }
    if !classification.is_valid {
        let error = classification.error.trim();
        if error.is_empty() {
            anyhow::bail!("content link is invalid");
        }
        anyhow::bail!("{error}");
    }
    let url = classification.local_open_url.trim();
    if url.is_empty() {
        anyhow::bail!("content link has no local resolver URL");
    }
    let display_name = sanitized_provider_file_name(&classification.open_display_name);
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let bytes = download_content_link_bytes(url).await?;
        import_provider_bytes(data_dir, &display_name, &bytes).await
    })
}

async fn import_provider_bytes(
    data_dir: &str,
    display_name: &str,
    bytes: &[u8],
) -> anyhow::Result<serde_json::Value> {
    let config_lock =
        iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
    let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
    let modified_at_by_path = BTreeMap::new();
    let entries = provider_entries(&provider, &modified_at_by_path).await?;
    let path = unique_provider_path(&entries, "", display_name, None);
    if provider_write_is_probable_os_placeholder(&entries, &path, bytes) {
        anyhow::bail!("refusing probable FileProvider placeholder copy: {path}");
    }
    write_provider_file(&provider, &path, bytes).await?;
    import_provider_mutation(
        &mut daemon,
        &provider,
        &path,
        Some(visible_root),
        config_lock,
    )
    .await
}

async fn download_content_link_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    #[cfg(test)]
    if let Some(bytes) = CONTENT_LINK_DOWNLOAD_BYTES_FOR_TEST
        .lock()
        .expect("content link test bytes lock")
        .clone()
    {
        return Ok(bytes);
    }

    let response = reqwest::get(url)
        .await
        .with_context(|| format!("downloading {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("content link returned HTTP {status}");
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("reading response body from {url}"))?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
static CONTENT_LINK_DOWNLOAD_BYTES_FOR_TEST: Mutex<Option<Vec<u8>>> = Mutex::new(None);

#[cfg(test)]
pub(crate) struct ContentLinkDownloadBytesGuard;

#[cfg(test)]
impl Drop for ContentLinkDownloadBytesGuard {
    fn drop(&mut self) {
        *CONTENT_LINK_DOWNLOAD_BYTES_FOR_TEST
            .lock()
            .expect("content link test bytes lock") = None;
    }
}

#[cfg(test)]
pub(crate) fn download_content_link_bytes_for_test(
    bytes: Vec<u8>,
) -> ContentLinkDownloadBytesGuard {
    *CONTENT_LINK_DOWNLOAD_BYTES_FOR_TEST
        .lock()
        .expect("content link test bytes lock") = Some(bytes);
    ContentLinkDownloadBytesGuard
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

fn run_native_provider_compose_path(
    parent_path: &str,
    display_name: &str,
) -> anyhow::Result<serde_json::Value> {
    let (parent_path, display_name, path) = compose_provider_path(parent_path, display_name)?;
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
        let config_dir = Path::new(data_dir);
        let config = load_native_runtime_config_cached(&config_path_in(config_dir))
            .map_err(anyhow::Error::msg)?;
        let daemon = iris_drive_core::Daemon::open_with_config(config_dir, config)
            .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
        let visible_view = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .context("building provider view")?;
        let modified_at_by_path = provider_modified_at_index(&visible_view);
        let visible_root = iris_drive_core::primary_merged_root_from_view(
            daemon.tree(),
            daemon.config(),
            &visible_view,
        )
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
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        apply_provider_open_delay_for_test();
        if provider_file_probable_os_placeholder_family(&path, bytes.len() as u64).is_some() {
            let entries = provider_entries(&provider, &BTreeMap::new()).await?;
            if provider_write_is_probable_os_placeholder(&entries, &path, &bytes) {
                anyhow::bail!("refusing probable FileProvider placeholder copy: {path}");
            }
        }
        write_provider_file(&provider, &path, &bytes).await?;
        import_provider_mutation(
            &mut daemon,
            &provider,
            &path,
            Some(visible_root),
            config_lock,
        )
        .await
    })
}

fn run_native_provider_mkdir(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        create_provider_dir(&provider, &path).await?;
        import_provider_mutation(
            &mut daemon,
            &provider,
            &path,
            Some(visible_root),
            config_lock,
        )
        .await
    })
}

fn run_native_provider_delete(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        delete_provider_path(&provider, &path).await?;
        import_provider_mutation(
            &mut daemon,
            &provider,
            &path,
            Some(visible_root),
            config_lock,
        )
        .await
    })
}

fn run_native_provider_rename(
    data_dir: &str,
    old_path: &str,
    new_path: &str,
) -> anyhow::Result<serde_json::Value> {
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let old_path = normalize_provider_path(old_path)?;
        let new_path = normalize_provider_path(new_path)?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        rename_provider_path(&provider, &old_path, &new_path).await?;
        import_provider_mutation(
            &mut daemon,
            &provider,
            &new_path,
            Some(visible_root),
            config_lock,
        )
        .await
    })
}

fn ensure_daemon_available_for_provider_mutation(data_dir: &str) -> anyhow::Result<()> {
    if provider_mutation_liveness() == ProviderMutationLiveness::RequireDaemonLock {
        iris_drive_core::daemon_liveness::ensure_daemon_available_for_provider_mutation(
            Path::new(data_dir),
        )?;
    }
    Ok(())
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
            let modified_at = modified_at_by_path
                .get(&child.id)
                .copied()
                .or(item.modified_at);
            entries.push(ProviderListEntry {
                path: child.id,
                parent_path: parent.clone(),
                display_name: child.name,
                kind,
                size: item.size,
                version: item.version,
                modified_at,
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let all_entries = entries.clone();
    entries.retain(|entry| !provider_entry_is_probable_os_placeholder(&all_entries, entry));
    Ok(entries)
}

async fn import_provider_mutation<P>(
    daemon: &mut iris_drive_core::Daemon,
    provider: &P,
    changed_path: &str,
    tombstone_base_root: Option<hashtree_core::Cid>,
    config_lock: iris_drive_core::config_lock::ConfigMutationLock,
) -> anyhow::Result<serde_json::Value>
where
    P: ProviderFs<ItemId = String>,
{
    let root = hashtree_core::Cid::parse(provider.anchor().await.as_str())
        .context("reading provider root CID")?;
    let report = import_provider_root_with_retry(daemon, root, tombstone_base_root).await?;
    iris_drive_core::paths::touch_provider_root_signal_in(daemon.config_dir())
        .context("signaling provider root change")?;
    let prepared_publish = match prepare_current_app_key_root_publish(daemon) {
        Ok(prepared) => prepared,
        Err(error) => PreparedProviderRootPublish::Unavailable(json!({
            "published_drive_root": false,
            "error": format!("{error:#}"),
        })),
    };
    // Import, config persistence, and event signing are one serialized local
    // transaction. Release the cross-process lock before Blossom or relay I/O.
    // Signing here also preserves replaceable-event ordering if deliveries
    // from two successive mutations finish out of order.
    drop(config_lock);
    let publish =
        publish_prepared_app_key_root_best_effort(daemon.config_dir(), prepared_publish).await;
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

struct PreparedProviderRootPublishReady {
    config: AppConfig,
    root: iris_drive_core::AppKeyRootRef,
    event: nostr_sdk::Event,
}

enum PreparedProviderRootPublish {
    Ready(Box<PreparedProviderRootPublishReady>),
    Unavailable(serde_json::Value),
}

fn prepare_current_app_key_root_publish(
    daemon: &iris_drive_core::Daemon,
) -> anyhow::Result<PreparedProviderRootPublish> {
    let config = daemon.config().clone();
    let Some(account) = config.profile.as_ref() else {
        return Ok(PreparedProviderRootPublish::Unavailable(
            json!({"published_drive_root": false, "error": "account missing"}),
        ));
    };
    let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
        return Ok(PreparedProviderRootPublish::Unavailable(
            json!({"published_drive_root": false, "error": "primary drive missing"}),
        ));
    };
    let Some(root) = drive.app_key_roots.get(&account.app_key_pubkey).cloned() else {
        return Ok(PreparedProviderRootPublish::Unavailable(
            json!({"published_drive_root": false, "error": "AppKey root missing"}),
        ));
    };
    let loaded_account =
        Profile::load(account.clone(), daemon.config_dir()).context("loading profile keys")?;
    let authorized_app_keys = iris_drive_core::drive_root_recipient_app_key_pubkeys(account, drive);
    let event = iris_drive_core::nostr_events::build_drive_root_publish_event(
        loaded_account.app_key.keys(),
        &account.root_scope_id(),
        &drive.drive_id,
        &root,
        &authorized_app_keys,
    )
    .map_err(iris_drive_core::relay_sync::RelayError::from)?;

    Ok(PreparedProviderRootPublish::Ready(Box::new(
        PreparedProviderRootPublishReady {
            config,
            root,
            event,
        },
    )))
}

async fn publish_prepared_app_key_root_best_effort(
    config_dir: &Path,
    prepared: PreparedProviderRootPublish,
) -> serde_json::Value {
    let prepared = match prepared {
        PreparedProviderRootPublish::Ready(prepared) => prepared,
        PreparedProviderRootPublish::Unavailable(result) => return result,
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
        publish_prepared_app_key_root(config_dir, *prepared),
    )
    .await
    {
        Ok(Ok(published)) => published,
        Ok(Err(error)) => json!({"published_drive_root": false, "error": format!("{error:#}")}),
        Err(_) => json!({"published_drive_root": false, "error": "publish timed out"}),
    }
}

async fn publish_prepared_app_key_root(
    config_dir: &Path,
    prepared: PreparedProviderRootPublishReady,
) -> anyhow::Result<serde_json::Value> {
    #[cfg(test)]
    if let Some(result) = provider_publish_lock_probe_result_for_test(
        config_dir,
        &prepared.root,
        Some(&prepared.event),
    )
    .await
    {
        return Ok(result);
    }

    let blossom_upload = if provider_root_blossom_upload_enabled(&prepared.config)? {
        Some(
            upload_current_app_key_root_to_blossom(
                config_dir,
                &prepared.config,
                &prepared.root.root_cid,
            )
            .await
            .context("making provider root blocks available")?,
        )
    } else {
        None
    };

    let relays = if prepared.config.relays.is_empty() {
        default_relays()
    } else {
        prepared.config.relays.clone()
    };
    let client = iris_drive_core::relay_sync::connect(&relays).await?;
    let result =
        iris_drive_core::relay_sync::publish_prebuilt_drive_root(&client, &prepared.event).await;
    iris_drive_core::relay_sync::shutdown_client(&client).await;
    let event_id = result?;
    Ok(json!({
        "published_drive_root": true,
        "drive_root_event_id": event_id.to_hex(),
        "blossom_total_hashes": blossom_upload.map_or(0, |upload| upload.total_hashes),
        "blossom_uploaded": blossom_upload.map_or(0, |upload| upload.uploaded),
        "blossom_already_present": blossom_upload.map_or(0, |upload| upload.already_present),
    }))
}

fn provider_root_blossom_upload_enabled(config: &AppConfig) -> anyhow::Result<bool> {
    if config.blossom_servers.is_empty() {
        anyhow::bail!(
            "cannot publish provider root before its blocks are available: no Blossom servers configured"
        );
    }
    #[cfg(test)]
    {
        // Unit tests create throwaway profiles with production defaults. Keep
        // them hermetic; focused upload tests use an explicit loopback server.
        Ok(config.blossom_servers.iter().all(|server| {
            server.starts_with("http://127.0.0.1:") || server.starts_with("http://[::1]:")
        }))
    }
    #[cfg(not(test))]
    {
        Ok(true)
    }
}

pub(crate) async fn upload_current_app_key_root_to_blossom(
    config_dir: &Path,
    config: &AppConfig,
    root_cid: &str,
) -> anyhow::Result<iris_drive_core::blossom_sync::UploadReport> {
    if config.blossom_servers.is_empty() {
        anyhow::bail!("no Blossom servers configured");
    }
    let root = hashtree_core::Cid::parse(root_cid)
        .with_context(|| format!("parsing provider root CID {root_cid}"))?;
    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .context("loading AppKey for Blossom upload")?;
    let client =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let daemon = iris_drive_core::Daemon::open(config_dir)
        .context("opening provider block store for Blossom upload")?;
    iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &root, &client)
        .await
        .context("uploading provider root to Blossom")
}

pub(crate) fn run_native_sync_once(
    data_dir: &str,
) -> anyhow::Result<iris_drive_core::NetworkSyncReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(iris_drive_core::sync_once_with_options(
        Path::new(data_dir),
        &[],
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
        native_sync_options(),
    ))
}

pub(crate) fn run_native_sync_pending_device_approval_acks(
    data_dir: &str,
) -> anyhow::Result<iris_drive_core::NetworkSyncReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(iris_drive_core::sync_pending_device_approval_acks(
        Path::new(data_dir),
        &[],
        std::time::Duration::from_secs(APPROVAL_ACK_FAST_SYNC_TIMEOUT_SECS),
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
    } else if report.profile_roster_ops_applied > 0 {
        "profile synced"
    } else {
        "up to date"
    }
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

#[cfg(test)]
static PROVIDER_OPEN_DELAY_MS_FOR_TEST: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) struct ProviderOpenDelayGuard;

#[cfg(test)]
impl Drop for ProviderOpenDelayGuard {
    fn drop(&mut self) {
        PROVIDER_OPEN_DELAY_MS_FOR_TEST.store(0, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn provider_open_delay_for_test(delay: std::time::Duration) -> ProviderOpenDelayGuard {
    let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
    PROVIDER_OPEN_DELAY_MS_FOR_TEST.store(delay_ms, Ordering::SeqCst);
    ProviderOpenDelayGuard
}

#[cfg(test)]
fn apply_provider_open_delay_for_test() {
    let delay_ms = PROVIDER_OPEN_DELAY_MS_FOR_TEST.load(Ordering::SeqCst);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
}

#[cfg(not(test))]
fn apply_provider_open_delay_for_test() {}

#[cfg(test)]
pub(crate) struct ProviderPublishLockProbeGuard {
    config_dir: PathBuf,
}

#[cfg(test)]
impl Drop for ProviderPublishLockProbeGuard {
    fn drop(&mut self) {
        let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if probe.as_deref() == Some(self.config_dir.as_path()) {
            *probe = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn provider_publish_lock_probe_for_test(
    config_dir: &Path,
) -> ProviderPublishLockProbeGuard {
    let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        probe.is_none(),
        "provider publish lock probe already active"
    );
    *probe = Some(config_dir.to_path_buf());
    ProviderPublishLockProbeGuard {
        config_dir: config_dir.to_path_buf(),
    }
}

#[cfg(test)]
async fn provider_publish_lock_probe_result_for_test(
    config_dir: &Path,
    root: &iris_drive_core::AppKeyRootRef,
    event: Option<&nostr_sdk::Event>,
) -> Option<serde_json::Value> {
    let should_probe = {
        let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if probe.as_deref() == Some(config_dir) {
            *probe = None;
            true
        } else {
            false
        }
    };
    if !should_probe {
        return None;
    }

    let reacquired = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir),
    )
    .await
    .expect("provider publish began while the config mutation lock was still held")
    .expect("reacquiring provider config mutation lock before network");
    drop(reacquired);

    let event = event.expect("provider publish must prepare its exact Drive-root event under lock");
    let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .expect("loading provider AppKey for publish probe");
    let (_, _, _, event_root) =
        iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, app_key.keys())
            .expect("decrypting prepared provider Drive-root event");
    assert_eq!(event_root.root_cid, root.root_cid);
    assert_eq!(event_root.dck_generation, root.dck_generation);
    assert_eq!(event_root.app_key_seq, root.app_key_seq);
    assert_eq!(event_root.parents, root.parents);
    assert_eq!(event_root.observed, root.observed);
    assert_eq!(event_root.local_only, root.local_only);
    assert!(
        event_root.published_at >= root.published_at,
        "replaceable event timestamp went backwards from the imported root"
    );
    Some(json!({
        "published_drive_root": false,
        "error": "provider publish lock probe skipped network",
        "prepared_root_cid": event_root.root_cid,
    }))
}
