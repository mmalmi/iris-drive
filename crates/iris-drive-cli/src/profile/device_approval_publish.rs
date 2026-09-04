use anyhow::{Context, Result};
use iris_drive_core::{ProfileState, config::AppConfig};

use super::APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS;

#[derive(Debug, Default)]
pub(super) struct DeviceApprovalPublishReport {
    pub published_events: usize,
    pub published_drive_root: bool,
    pub root_cid: Option<String>,
    pub blossom_upload: Option<iris_drive_core::blossom_sync::UploadReport>,
}

impl DeviceApprovalPublishReport {
    pub(super) fn into_output_json(
        self,
        approved_app_key_npub: &str,
        roster_size: usize,
        approval_publish_error: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "approved_app_key_npub": approved_app_key_npub,
            "roster_size": roster_size,
            "published_approval_events": self.published_events,
            "published_drive_root": self.published_drive_root,
            "root_cid": self.root_cid,
            "blossom_upload": self.blossom_upload.map(|report| serde_json::json!({
                "total_hashes": report.total_hashes,
                "uploaded": report.uploaded,
                "already_present": report.already_present,
            })),
            "approval_publish_error": approval_publish_error,
        })
    }
}

pub(super) fn publish_device_approval_with_error(
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &ProfileState,
    pending: &iris_drive_core::profile::PendingDeviceApprovalReceipt,
) -> (DeviceApprovalPublishReport, Option<String>) {
    publish_device_approval(config_dir, config, state, pending).map_or_else(
        |error| {
            (
                DeviceApprovalPublishReport::default(),
                Some(format!("{error:#}")),
            )
        },
        |report| (report, None),
    )
}

fn publish_device_approval(
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &ProfileState,
    pending: &iris_drive_core::profile::PendingDeviceApprovalReceipt,
) -> Result<DeviceApprovalPublishReport> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building device approval relay runtime")?;
    let relays = iris_drive_core::relay_config::normalize_relay_urls(&config.relays)
        .context("normalizing relay config")?;
    runtime.block_on(async {
        let device = iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(config_dir))
            .context("loading AppKey for device approval")?;
        let drive_root = iris_drive_core::prepare_device_approval_root_handoff(config_dir).await?;
        let client = iris_drive_core::relay_sync::connect(&relays).await?;
        let result = async {
            let mut report = DeviceApprovalPublishReport {
                root_cid: Some(drive_root.root.root_cid.clone()),
                blossom_upload: Some(drive_root.blossom_upload),
                ..DeviceApprovalPublishReport::default()
            };
            if !state.profile_roster_ops.is_empty() {
                let event_ids = tokio::time::timeout(
                    std::time::Duration::from_secs(APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS),
                    iris_drive_core::relay_sync::publish_nostr_identity_roster_ops(
                        &client,
                        &state.profile_roster_ops,
                    ),
                )
                .await
                .map_err(|_| anyhow::anyhow!("publishing approval roster timed out"))??;
                report.published_events += event_ids.len();
            }

            tokio::time::timeout(
                std::time::Duration::from_secs(APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS),
                iris_drive_core::relay_sync::publish_drive_root(
                    &client,
                    device.keys(),
                    &drive_root.root_scope_id,
                    &drive_root.drive_id,
                    &drive_root.root,
                    &drive_root.authorized_app_keys,
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("publishing approval Drive root timed out"))??;
            report.published_events += 1;
            report.published_drive_root = true;

            tokio::time::timeout(
                std::time::Duration::from_secs(APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS),
                iris_drive_core::relay_sync::publish_pending_device_approval_receipt(
                    &client, pending,
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("publishing device approval receipt timed out"))??;
            report.published_events += 1;
            Ok(report)
        }
        .await;
        iris_drive_core::relay_sync::shutdown_client(&client).await;
        result
    })
}
