use super::{BTreeSet, Cid, Context, Daemon, Result, Store};

pub(super) const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[
    250, 500, 1_000, 2_000, 4_000, 8_000, 12_000, 16_000, 16_000, 16_000,
];

pub(super) async fn ensure_provider_root_locally_available(
    daemon: &Daemon,
    root: &Cid,
) -> Result<()> {
    let mut attempt = 0;
    loop {
        match check_provider_root_locally_available(daemon, root).await {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
                    && provider_import_error_message_is_retryable(&format!("{error:#}")) =>
            {
                let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
                attempt += 1;
                tracing::warn!(
                    error = %error,
                    delay_ms,
                    root_cid = %root,
                    "provider command hit a transient local root read; retrying mutation preflight"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("provider root {root} is not locally readable for mutation")
                });
            }
        }
    }
}

async fn check_provider_root_locally_available(daemon: &Daemon, root: &Cid) -> Result<()> {
    let hashes = iris_drive_core::block_sync::collect_live_sync_hashes(daemon.tree(), root, 4)
        .await
        .context("walking local provider root blocks")?;
    let store = daemon.tree().get_store().clone();
    for hash in hashes {
        if !store
            .has(&hash)
            .await
            .with_context(|| format!("checking local block {}", hashtree_core::to_hex(&hash)))?
        {
            anyhow::bail!(
                "local store is missing provider root block {}",
                hashtree_core::to_hex(&hash)
            );
        }
    }
    Ok(())
}

pub(super) async fn primary_merged_root_with_retry(
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

pub(super) async fn primary_merged_view_with_retry(
    daemon: &Daemon,
) -> Result<iris_drive_core::PrimaryMergedView> {
    let mut attempt = 0;
    loop {
        match iris_drive_core::primary_merged_view(daemon.tree(), daemon.config()).await {
            Ok(view) => return Ok(view),
            Err(error)
                if attempt < PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
                    && provider_import_error_message_is_retryable(&error.to_string()) =>
            {
                let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
                attempt += 1;
                tracing::warn!(
                    error = %error,
                    delay_ms,
                    "provider command hit a transient store read; retrying merged view build"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(super) async fn primary_merged_root_from_view_with_retry(
    daemon: &Daemon,
    view: &iris_drive_core::PrimaryMergedView,
) -> Result<iris_drive_core::PrimaryMergedRoot> {
    let mut attempt = 0;
    loop {
        match iris_drive_core::primary_merged_root_from_view(daemon.tree(), daemon.config(), view)
            .await
        {
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
                    "provider command hit a transient store read; retrying merged root materialization"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) async fn import_provider_root_with_retry(
    daemon: &mut Daemon,
    root: Cid,
    tombstone_base_root: Option<Cid>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<iris_drive_core::daemon::ImportReport> {
    let mut attempt = 0;
    loop {
        match daemon
            .import_visible_root_with_tombstone_base_and_paths(
                root.clone(),
                tombstone_base_root.clone(),
                tombstone_paths,
            )
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

pub(super) fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("Store error")
        && (message.contains("os error 2")
            || message.contains("No such file or directory")
            || message.contains("The system cannot find the file specified"))
        || message.contains("Missing chunk")
        || message.contains("local store is missing provider root block")
}
