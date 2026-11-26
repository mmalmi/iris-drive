use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex;

use anyhow::Context;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use iris_drive_core::config::DEFAULT_RELAYS;
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::provider::{
    compose_provider_path, create_provider_dir, list_provider_dir_entries, list_provider_entries,
    normalize_provider_document_path, normalize_provider_parent_path, normalize_provider_path,
    optional_normalized_provider_path, provider_file_probable_os_placeholder_family,
    provider_list_summary, provider_modified_at_index, provider_path_is_child_document,
    provider_write_is_probable_os_placeholder, rename_provider_path, sanitized_provider_file_name,
    split_provider_path, unique_provider_path, write_provider_file, write_provider_file_from_path,
};
use iris_drive_core::{AppConfig, Profile};
use serde_json::json;

use crate::ffi::load_native_runtime_config_cached;

const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[25, 50, 100, 200, 400];
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;
const APPROVAL_ACK_FAST_SYNC_TIMEOUT_SECS: u64 = 3;

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
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let display_name = sanitized_provider_file_name(display_name);
    let runtime = native_provider_runtime()?;
    runtime.block_on(import_provider_file(
        data_dir,
        &display_name,
        Path::new(source_path),
        None,
    ))
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
    let entries = list_provider_dir_entries(daemon.tree(), &visible_root, "").await?;
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

/// Stage an iOS share in the selected folder. The extension publishes the batch after
/// committing each item; a later background launch resumes a pending upload.
pub(crate) fn stage_shared_file_json(
    dir: &str,
    parent: &str,
    name: &str,
    source: &str,
) -> serde_json::Value {
    let result = native_provider_runtime().and_then(|runtime| {
        let parent = optional_normalized_provider_path(parent)?.unwrap_or_default();
        runtime.block_on(import_provider_file(
            dir,
            &sanitized_provider_file_name(name),
            Path::new(source),
            Some(&parent),
        ))
    });
    match result {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn publish_shared_files_json(
    dir: &str,
    allow_network: bool,
    cancelled: impl Fn() -> bool,
) -> serde_json::Value {
    let dir = Path::new(dir);
    if !iris_drive_core::paths::key_path_in(dir).is_file() {
        return json!({"ok": true, "pending": false});
    }
    let result = native_provider_runtime().and_then(|runtime| runtime.block_on(async {
        tokio::select! {
            result = pending_upload::PendingUpload::SharedFiles.publish(dir, None, allow_network) => result,
            () = async {
                while !cancelled() { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
            } => anyhow::bail!("Upload paused"),
        }
    }));
    pending_upload::result_json(result)
}

async fn import_provider_file(
    data_dir: &str,
    display_name: &str,
    source: &Path,
    share_sheet_folder: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let size = source_len(source)?;
    let config_lock =
        iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
    let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
    let parent = share_sheet_folder.unwrap_or_default();
    let entries = list_provider_dir_entries(daemon.tree(), &visible_root, parent).await?;
    let path = unique_provider_path(&entries, parent, display_name, None);
    if size == 0 && provider_write_is_probable_os_placeholder(&entries, &path, &[]) {
        anyhow::bail!("refusing probable FileProvider placeholder copy: {path}");
    }
    write_provider_file_from_path(&provider, &path, source).await?;
    if share_sheet_folder.is_some() {
        let profile = daemon
            .config()
            .profile
            .as_ref()
            .filter(|p| p.is_authorized())
            .context("Sign in to save to Iris Drive")?
            .profile_id
            .to_string();
        let root = hashtree_core::Cid::parse(provider.anchor().await.as_str())
            .context("reading shared file root")?;
        pending_upload::PendingUpload::SharedFiles.queue(
            Path::new(data_dir),
            &profile,
            &root.hash,
        )?;
        persist_provider_root(&mut daemon, root, Some(visible_root)).await?;
        return Ok(json!({"path": path}));
    }
    import_provider_mutation(
        &mut daemon,
        &provider,
        &path,
        Some(visible_root),
        config_lock,
    )
    .await
}

fn source_len(source: &Path) -> anyhow::Result<u64> {
    Ok(std::fs::metadata(source)
        .with_context(|| format!("reading {}", source.display()))?
        .len())
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

pub(crate) fn run_native_provider_list(data_dir: &str) -> anyhow::Result<serde_json::Value> {
    provider_list(data_dir, false)
}

pub(crate) fn provider_list(
    data_dir: &str,
    refresh_storage: bool,
) -> anyhow::Result<serde_json::Value> {
    #[cfg(test)]
    work_pause::wait(data_dir, "provider");
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let config_dir = Path::new(data_dir);
        let config = load_native_runtime_config_cached(&config_path_in(config_dir))
            .map_err(anyhow::Error::msg)?;
        let daemon = iris_drive_core::Daemon::open_with_config(config_dir, config)
            .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
        let (visible_view, visible_root, _) =
            iris_drive_core::provider::projection::provider_projection(&daemon)
                .await
                .context("building provider view")?;
        if refresh_storage
            && let Err(error) = iris_drive_core::device_storage::local::refresh_local_report(
                &daemon,
                &visible_root.root_cid,
            )
            .await
        {
            tracing::warn!(%error, "measuring local Drive storage");
        }
        let modified_at_by_path = provider_modified_at_index(&visible_view);
        let entries =
            list_provider_entries(daemon.tree(), &visible_root.root_cid, &modified_at_by_path)
                .await?;
        let anchor = visible_root.root_cid.to_string();
        let summary = provider_list_summary(&anchor, &entries);
        Ok(json!({
            "anchor": anchor,
            "root_cid": visible_root.root_cid.to_string(),
            "file_count": summary.file_count,
            "visible_file_bytes": summary.visible_file_bytes,
            "share_action_paths": iris_drive_core::share_folders::share_action_folders(
                daemon.config(),
                &summary.directory_paths,
            ),
            "save_folders": iris_drive_core::share_folders::save_folders(
                daemon.config(),
                &summary.directory_paths,
            ),
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
        let output = PathBuf::from(output_path);
        let item = iris_drive_core::provider::read::read_provider_file_to_path(
            Path::new(data_dir),
            &provider,
            &path,
            &output,
        )
        .await?;
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
        let source = Path::new(source_path);
        let size = source_len(source)?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        apply_provider_open_delay_for_test();
        if provider_file_probable_os_placeholder_family(&path, size).is_some() {
            let (parent, _) = split_provider_path(&path)?;
            let entries = list_provider_dir_entries(daemon.tree(), &visible_root, &parent).await?;
            if provider_write_is_probable_os_placeholder(&entries, &path, &[]) {
                anyhow::bail!("refusing probable FileProvider placeholder copy: {path}");
            }
        }
        write_provider_file_from_path(&provider, &path, source).await?;
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
    let (_, visible, _) = iris_drive_core::provider::projection::provider_projection(&daemon)
        .await
        .context("building provider root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone())
        .await
        .context("opening provider root")?;
    Ok((daemon, provider, visible.root_cid))
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
    commit_provider_root(daemon, root, tombstone_base_root, changed_path, config_lock).await
}

mod delete;
pub(crate) use delete::native_provider_delete_json;
mod path;
pub(crate) use path::{native_provider_compose_path_json, native_provider_reimport};
use path::{run_native_provider_normalize_path, run_native_provider_resolve_path};
mod pending_upload;
pub(crate) mod photo_backup;

async fn commit_provider_root(
    daemon: &mut iris_drive_core::Daemon,
    root: hashtree_core::Cid,
    tombstone_base_root: Option<hashtree_core::Cid>,
    changed_path: &str,
    config_lock: iris_drive_core::config_lock::ConfigMutationLock,
) -> anyhow::Result<serde_json::Value> {
    let report = persist_provider_root(daemon, root, tombstone_base_root).await?;
    let prepared_publish = match prepare_current_app_key_root_publish(daemon).await {
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

async fn persist_provider_root(
    daemon: &mut iris_drive_core::Daemon,
    root: hashtree_core::Cid,
    base: Option<hashtree_core::Cid>,
) -> anyhow::Result<iris_drive_core::ImportReport> {
    let report = import_provider_root_with_retry(daemon, root, base).await?;
    iris_drive_core::paths::touch_provider_root_signal_in(daemon.config_dir())
        .context("signaling provider root change")?;
    Ok(report)
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

async fn prepare_current_app_key_root_publish(
    daemon: &iris_drive_core::Daemon,
) -> anyhow::Result<PreparedProviderRootPublish> {
    // The lightweight provider daemon omits the signed roster sidecar. Load
    // it under the caller's mutation lock so every current reader gets a wrap.
    let config = AppConfig::load_or_default(config_path_in(daemon.config_dir()))?;
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
    let Some(root) = iris_drive_core::publishable_root::publishable_app_key_root(
        daemon.config_dir(),
        drive,
        account,
    )
    .await?
    else {
        return Ok(PreparedProviderRootPublish::Unavailable(
            json!({"published_drive_root": false, "error": "AppKey root missing"}),
        ));
    };
    let loaded_account =
        Profile::load(account.clone(), daemon.config_dir()).context("loading profile keys")?;
    let authorized_app_keys = iris_drive_core::drive_root_key_recipient_pubkeys(account, drive);
    let event = iris_drive_core::nostr_events::build_drive_root_publish_event(
        daemon.config_dir(),
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
    #[cfg(test)]
    {
        // Unit tests create throwaway profiles with production defaults. Keep
        // them hermetic; a test that publishes configures a loopback relay.
        if !relays
            .iter()
            .all(|relay| relay.starts_with("ws://127.0.0.1:") || relay.starts_with("ws://[::1]:"))
        {
            return Ok(json!({
                "published_drive_root": false,
                "error": "unit tests publish only to loopback relays",
            }));
        }
    }
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
    #[cfg(test)]
    work_pause::wait(data_dir, "sync");
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
#[path = "native_provider_test_hooks.rs"]
mod test_hooks;
#[cfg(test)]
use test_hooks::{apply_provider_open_delay_for_test, provider_publish_lock_probe_result_for_test};
#[cfg(test)]
pub(crate) use test_hooks::{provider_open_delay_for_test, provider_publish_lock_probe_for_test};

#[cfg(not(test))]
fn apply_provider_open_delay_for_test() {}

#[cfg(test)]
#[path = "native_provider_work_pause.rs"]
pub(crate) mod work_pause;
