use std::future::Future;

use iris_drive_core::{PrimaryMergedRoot, PrimaryMergedView};

use super::{
    Cid, ConfigMutationLock, Context, Daemon, Path, ProviderCmd, ProviderStagedRoot, Result, Store,
    normalize_provider_path, provider_command_base_root_cid, provider_command_is_mutation,
    provider_command_needs_merged_view, read_provider_staging,
};

pub(super) const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[
    250, 500, 1_000, 2_000, 4_000, 8_000, 12_000, 16_000, 16_000, 16_000,
];

pub(super) async fn retry_provider_operation<T, F: Future<Output = Result<T>>>(
    operation: &'static str,
    mut run: impl FnMut() -> F,
) -> Result<T> {
    let mut delays = PROVIDER_IMPORT_RETRY_DELAYS_MS.iter();
    loop {
        match run().await {
            Err(error) if provider_import_error_message_is_retryable(&format!("{error:#}")) => {
                let Some(&delay_ms) = delays.next() else {
                    return Err(error);
                };
                tracing::warn!(error = %format!("{error:#}"), delay_ms, operation,
                    "provider command hit a transient store read; retrying with current config");
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            result => return result,
        }
    }
}

pub(super) struct ProviderCommandState {
    pub(super) daemon: Daemon,
    pub(super) mutation_lock: Option<ConfigMutationLock>,
    pub(super) staged_root: Option<ProviderStagedRoot>,
    pub(super) visible_view: Option<PrimaryMergedView>,
    pub(super) visible: PrimaryMergedRoot,
}

/// Hold the lock from the successful preflight through staging. An incomplete
/// preflight drops it before backoff so peer sync or another local writer can
/// advance. A new attempt then reads the current heads, permissions and stage.
pub(super) async fn prepare_provider_command(
    config_dir: &Path,
    command: &ProviderCmd,
) -> Result<ProviderCommandState> {
    let mutation = provider_command_is_mutation(command);
    let mutation_lock = if mutation {
        iris_drive_core::daemon_liveness::ensure_daemon_available_for_provider_mutation(
            config_dir,
        )?;
        Some(ConfigMutationLock::acquire(config_dir).await?)
    } else {
        None
    };
    let daemon = Daemon::open(config_dir)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    let ensure_writable = |path: &str| {
        iris_drive_core::shared_content::ensure_shared_path_writable(
            daemon.config(),
            &normalize_provider_path(path)?,
        )
    };
    match command {
        ProviderCmd::Write { path, .. }
        | ProviderCmd::Mkdir { path, .. }
        | ProviderCmd::Delete { path, .. } => ensure_writable(path)?,
        ProviderCmd::Rename {
            old_path, new_path, ..
        } => {
            ensure_writable(old_path)?;
            ensure_writable(new_path)?;
        }
        _ => {}
    }
    let staged_root = read_provider_staging(config_dir).context("reading staged provider root")?;
    let supplied = provider_command_base_root_cid(command)?;
    let anchored = staged_root
        .as_ref()
        .map(ProviderStagedRoot::root)
        .transpose()?
        .or(supplied);
    let needs_view = provider_command_needs_merged_view(command);
    let (visible_view, canonical) = if needs_view || anchored.is_none() {
        let (view, root, _) = daemon
            .provider_projection()
            .await
            .context("building virtual provider root")?;
        (needs_view.then_some(view), Some(root))
    } else {
        (None, None)
    };
    let visible = if let Some(root_cid) = anchored {
        PrimaryMergedRoot {
            root_cid,
            file_count: 0,
            top_level_entries: 0,
        }
    } else {
        let root = canonical.expect("unanchored commands build the current root");
        if mutation {
            check_provider_root_locally_available(&daemon, &root.root_cid).await?;
        }
        root
    };
    Ok(ProviderCommandState {
        daemon,
        mutation_lock,
        staged_root,
        visible_view,
        visible,
    })
}

async fn check_provider_root_locally_available(daemon: &Daemon, root: &Cid) -> Result<()> {
    let hashes = iris_drive_core::block_sync::collect_live_sync_hashes_without_leaf_reads(
        daemon.tree(),
        root,
        8,
    )
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

pub(super) fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("Store error")
        && (message.contains("os error 2")
            || message.contains("No such file or directory")
            || message.contains("The system cannot find the file specified"))
        || message.contains("Missing chunk")
        || message.contains("local store is missing provider root block")
}
