#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_sync(
    config_dir: &std::path::Path,
    relay_override: &[String],
    timeout_secs: u64,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let report = iris_drive_core::network_sync_once(
            config_dir,
            relay_override,
            std::time::Duration::from_secs(timeout_secs),
        )
        .await?;

        println!(
            "{}",
            json!({
                "relays": report.relays,
                "blossom_servers": report.blossom_servers,
                "app_keys_event_applied": report.app_keys_event_applied,
                "profile_roster_ops_seen": report.profile_roster_ops_seen,
                "profile_roster_ops_applied": report.profile_roster_ops_applied,
                "drive_root_events_seen": report.drive_root_events_seen,
                "drive_root_events_applied": report.drive_root_events_applied,
                "drive_root_events_skipped": report.drive_root_events_skipped,
                "files_root_event_seen": report.files_root_event_seen,
                "files_root_event_outcome": report.files_root_event_outcome,
                "files_root_fetch_error": report.files_root_fetch_error,
                "fips_download": report.fips_download.as_ref().map(download_report_json),
                "fips_download_error": report.fips_download_error,
                "blossom_download": report.blossom_download.as_ref().map(download_report_json),
                "blossom_download_error": report.blossom_download_error,
                "materialized_root_cid": report.materialized_root_cid,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}
