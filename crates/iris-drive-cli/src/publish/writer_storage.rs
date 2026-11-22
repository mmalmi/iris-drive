/// A writer can acquire authority after startup, or regain storage after a
/// failed startup publication. In both cases upload the currently publishable
/// original, including when its visible local snapshot has since advanced.
pub(crate) async fn retry_writer_storage_publication(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    approved: &ProfileState,
) {
    let mut delay_secs = 2;
    loop {
        let result = async {
            let config = AppConfig::load_or_default(config_path_in(config_dir))?;
            let Some(state) = config.profile.as_ref().filter(|p| {
                p.profile_id == approved.profile_id
                    && p.app_key_pubkey == approved.app_key_pubkey
                    && p.can_write_roots()
            }) else {
                return Ok(());
            };
            let primary = config.drive(PRIMARY_DRIVE_ID);
            let replica = iris_drive_core::calendar::local_replica_tree(config_dir, state)?
                .and_then(|name| config.drive(&name));
            if config.blossom_servers.is_empty() || primary.is_none() && replica.is_none() {
                return Ok(());
            }
            let has_primary_root = if let Some(drive) = primary {
                publishable_app_key_root(config_dir, drive, state)
                    .await?
                    .is_some()
            } else {
                false
            };
            if !has_primary_root && replica.is_none() {
                return Ok(());
            }
            let report = Box::pin(publish_current_state(
                client, config_dir, &config, state, true,
            ))
            .await?;
            if let Some(error) = report.blossom_upload_error {
                anyhow::bail!(error);
            }
            anyhow::ensure!(
                !report.calendar_replica_pending
                    && (!has_primary_root || report.blossom_upload.is_some()),
                "authorized writer upload is pending"
            );
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => break,
            Err(error) => tracing::warn!(%error, delay_secs, "writer storage publication pending"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        delay_secs = (delay_secs * 2).min(300);
    }
}
