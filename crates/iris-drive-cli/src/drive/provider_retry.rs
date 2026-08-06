use super::{BTreeSet, Cid, Context, Daemon, Result, Store};
use std::future::Future;
use std::pin::Pin;

pub(super) const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[
    250, 500, 1_000, 2_000, 4_000, 8_000, 12_000, 16_000, 16_000, 16_000,
];

type ProviderRetryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

async fn retry_provider_operation<T>(
    daemon: &mut Daemon,
    operation: &'static str,
    mut run: impl for<'a> FnMut(&'a mut Daemon) -> ProviderRetryFuture<'a, T>,
) -> Result<T> {
    let mut attempt = 0;
    loop {
        let error = match run(daemon).await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if attempt >= PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
            || !provider_import_error_message_is_retryable(&format!("{error:#}"))
        {
            return Err(error);
        }

        let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
        attempt += 1;
        tracing::warn!(
            error = %error,
            delay_ms,
            operation,
            "provider command hit a transient store read; retrying with current config"
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        let config_dir = daemon.config_dir().to_path_buf();
        *daemon = Daemon::open(&config_dir).with_context(|| {
            format!(
                "reopening daemon at {} for provider retry",
                config_dir.display()
            )
        })?;
    }
}

pub(super) async fn ensure_provider_root_locally_available(
    daemon: &mut Daemon,
    root: &Cid,
) -> Result<()> {
    let root = root.clone();
    let root_label = root.to_string();
    retry_provider_operation(daemon, "mutation preflight", move |daemon| {
        let root = root.clone();
        Box::pin(async move { check_provider_root_locally_available(daemon, &root).await })
    })
    .await
    .with_context(|| format!("provider root {root_label} is not locally readable for mutation"))
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
    daemon: &mut Daemon,
) -> Result<iris_drive_core::PrimaryMergedRoot> {
    retry_provider_operation(daemon, "merged root build", |daemon| {
        Box::pin(async move {
            Ok(iris_drive_core::primary_merged_root(daemon.tree(), daemon.config()).await?)
        })
    })
    .await
}

pub(super) async fn primary_merged_view_and_root_with_retry(
    daemon: &mut Daemon,
) -> Result<(
    iris_drive_core::PrimaryMergedView,
    iris_drive_core::PrimaryMergedRoot,
)> {
    retry_provider_operation(daemon, "merged view build", |daemon| {
        Box::pin(async move {
            let view = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config()).await?;
            let root = iris_drive_core::primary_merged_root_from_view(
                daemon.tree(),
                daemon.config(),
                &view,
            )
            .await?;
            Ok((view, root))
        })
    })
    .await
}

pub(crate) async fn import_provider_root_with_retry(
    daemon: &mut Daemon,
    root: Cid,
    tombstone_base_root: Option<Cid>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<iris_drive_core::daemon::ImportReport> {
    let tombstone_paths = tombstone_paths.cloned();
    retry_provider_operation(daemon, "provider import", move |daemon| {
        let root = root.clone();
        let tombstone_base_root = tombstone_base_root.clone();
        let tombstone_paths = tombstone_paths.clone();
        Box::pin(async move {
            Ok(daemon
                .import_visible_root_with_tombstone_base_and_paths(
                    root,
                    tombstone_base_root,
                    tombstone_paths.as_ref(),
                )
                .await?)
        })
    })
    .await
}

pub(super) fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("Store error")
        && (message.contains("os error 2")
            || message.contains("No such file or directory")
            || message.contains("The system cannot find the file specified"))
        || message.contains("Missing chunk")
        || message.contains("local store is missing provider root block")
}
