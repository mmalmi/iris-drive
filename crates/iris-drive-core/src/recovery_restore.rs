//! Restore a Drive from its recovery phrase alone.
//!
//! The phrase finds the account on relays, admits this install's fresh
//! `AppKey`, and opens every device's latest My Drive root through the wrap
//! each device made for the phrase. The phrase key lives only in memory for
//! this restore. Once this device publishes its own root, made from all of
//! them, the phrase is no longer needed to read the files.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr_sdk::Client;

use crate::network_sync::{DriveRootEventApplyReport, download_applied_drive_roots};
use crate::recovery_phrase::recovery_phrase_to_keys;
use crate::recovery_wraps::apply_drive_root_events_opening_with_recovery_key;
use crate::{AppConfig, AppKey, Drive, PRIMARY_DRIVE_ID, Profile, relay_sync};

/// An account joined with its recovery phrase. Nothing is saved yet: the
/// caller saves the `AppKey`, the profile and the drive together.
pub struct RecoveryPhraseRestore {
    pub profile: Profile,
    /// The primary drive, holding each device's latest root that this device
    /// or the recovery phrase could open.
    pub drive: Drive,
    pub drive_roots: DriveRootEventApplyReport,
}

/// Find the account `recovery_phrase` belongs to and join it.
///
/// Returns `None` when the relays know no account for the phrase.
pub async fn restore_from_recovery_phrase(
    config_dir: &Path,
    client: &Client,
    recovery_phrase: &str,
    app_key_label: Option<String>,
    timeout: Duration,
) -> Result<Option<RecoveryPhraseRestore>> {
    let recovery_keys = recovery_phrase_to_keys(recovery_phrase)?;
    let recovery_pubkey = recovery_keys.public_key().to_hex();
    let candidates =
        relay_sync::fetch_nostr_identity_restore_candidates(client, &recovery_pubkey, timeout)
            .await
            .context("finding the account for this recovery phrase")?;
    let Some(candidate) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let profile = Profile::restore_with_recovery_phrase_keys(
        config_dir,
        &recovery_keys,
        candidate.profile_id,
        candidate.profile_roster_ops,
        app_key_label,
    )?;

    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    let devices = profile
        .state
        .active_root_writer_app_key_pubkeys()
        .into_iter()
        .filter(|pubkey| pubkey != &profile.state.app_key_pubkey)
        .collect::<Vec<_>>();
    let events = relay_sync::fetch_drive_roots(
        client,
        &profile.state.root_scope_id(),
        PRIMARY_DRIVE_ID,
        &devices,
        timeout,
    )
    .await
    .context("fetching the account's Drive roots")?;
    let drive_roots = apply_drive_root_events_opening_with_recovery_key(
        &mut config,
        &events,
        profile.app_key.keys(),
        &recovery_keys,
    )?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .cloned()
        .context("restored profile has no primary Drive")?;
    Ok(Some(RecoveryPhraseRestore {
        profile,
        drive,
        drive_roots,
    }))
}

/// What publishing a restored Drive did.
#[derive(Debug, Clone)]
pub struct RecoveredDriveRoot {
    pub root_cid: String,
    pub published_roster_ops: usize,
}

/// Bring a restored Drive's files to this device and publish them as this
/// device's own root, wrapped for this device and the recovery phrases.
///
/// `root_cids` are the roots the restore opened. Their blocks come from
/// Blossom, where the devices that published them uploaded them.
pub async fn publish_recovered_drive_root(
    config_dir: &Path,
    client: &Client,
    root_cids: &[String],
    timeout: Duration,
) -> Result<RecoveredDriveRoot> {
    let download = download_applied_drive_roots(config_dir, root_cids, None)
        .await
        .context("downloading the restored Drive")?;
    if let Some(error) = download.blossom_download_error {
        anyhow::bail!("downloading the restored Drive: {error}");
    }
    let handoff = crate::prepare_device_approval_root_handoff(config_dir)
        .await
        .context("making this device's Drive root")?;
    let config = AppConfig::load_or_default(crate::paths::config_path_in(config_dir))?;
    let state = config.profile.context("profile disappeared")?;
    let published_roster_ops = tokio::time::timeout(
        timeout,
        relay_sync::publish_nostr_identity_roster_ops(client, &state.profile_roster_ops),
    )
    .await
    .context("publishing this device's admission timed out")??
    .len();
    let app_key = AppKey::load(crate::paths::key_path_in(config_dir))?;
    tokio::time::timeout(
        timeout,
        relay_sync::publish_drive_root(
            config_dir,
            client,
            app_key.keys(),
            &handoff.root_scope_id,
            &handoff.drive_id,
            &handoff.root,
            &handoff.authorized_app_keys,
        ),
    )
    .await
    .context("publishing the restored Drive root timed out")??;
    crate::paths::touch_provider_root_signal_in(config_dir)?;
    Ok(RecoveredDriveRoot {
        root_cid: handoff.root.root_cid,
        published_roster_ops,
    })
}
