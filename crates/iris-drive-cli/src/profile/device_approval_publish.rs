use anyhow::{Context, Result};
use iris_drive_core::{ProfileState, config::AppConfig};

use super::APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS;

pub(super) fn publish_device_approval(
    config: &AppConfig,
    state: &ProfileState,
    pending: &iris_drive_core::profile::PendingDeviceApprovalReceipt,
) -> Result<usize> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building device approval relay runtime")?;
    let relays = iris_drive_core::relay_config::normalize_relay_urls(&config.relays)
        .context("normalizing relay config")?;
    let event_ids = runtime.block_on(async {
        let client = iris_drive_core::relay_sync::connect(&relays).await?;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS),
            iris_drive_core::relay_sync::publish_device_approval_receipt(&client, state, pending),
        )
        .await
        .map_err(|_| {
            iris_drive_core::relay_sync::RelayError::Client(
                "publishing device approval receipt timed out".to_string(),
            )
        })?;
        iris_drive_core::relay_sync::shutdown_client(&client).await;
        result
    })?;
    Ok(event_ids.len())
}
