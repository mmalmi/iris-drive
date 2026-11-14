//! My Drive root keys wrapped for the account's recovery phrases.
//!
//! A device wraps its My Drive root key for the account's devices, and also
//! for every active recovery phrase key. Then a device that joins with the
//! phrase alone, when no other device is left, can still open every device's
//! latest root. Readers look up only their own key, so the extra wraps change
//! nothing for anyone else. Shared folders are not wrapped this way: their
//! other members' devices already resend them.

use nostr_sdk::{Event, Keys};

use crate::network_sync::{DriveRootEventApplyReport, drive_root_recipient_app_key_pubkeys};
use crate::relay_sync::{self, DriveRootApply};
use crate::{
    AppConfig, Drive, NostrIdentityKeyPurpose, NostrIdentityRosterOp, ProfileState,
    SignedNostrIdentityRosterOp,
};

/// Public keys of the account's active recovery phrases.
#[must_use]
pub fn active_recovery_phrase_pubkeys(state: &ProfileState) -> Vec<String> {
    state
        .profile_projection()
        .active_facets
        .values()
        .filter(|facet| facet.has_purpose(NostrIdentityKeyPurpose::RecoveryPhrase))
        .map(|facet| facet.pubkey.clone())
        .collect()
}

/// Everyone a My Drive root key is wrapped for: the account's devices and
/// its active recovery phrases.
#[must_use]
pub fn drive_root_key_recipient_pubkeys(state: &ProfileState, drive: &Drive) -> Vec<String> {
    let mut recipients = drive_root_recipient_app_key_pubkeys(state, drive);
    recipients.extend(active_recovery_phrase_pubkeys(state));
    recipients.sort();
    recipients.dedup();
    recipients
}

/// Whether any of `ops` adds a recovery phrase that is still active.
///
/// A device that publishes such an op republishes its root, so the root it
/// published before carries a wrap for the new phrase too.
#[must_use]
pub fn ops_add_active_recovery_phrase(
    state: &ProfileState,
    ops: &[SignedNostrIdentityRosterOp],
) -> bool {
    let active = active_recovery_phrase_pubkeys(state);
    ops.iter().any(|op| match &op.content.op {
        NostrIdentityRosterOp::AddFacet { facet } => active.contains(&facet.pubkey),
        _ => false,
    })
}

/// Apply drive-root events, opening each root with this device's key or,
/// when the root has no wrap for this device, with the recovery phrase key.
///
/// Used once, while restoring from a recovery phrase: the devices that
/// published these roots may be gone and never wrap them for this device.
pub fn apply_drive_root_events_opening_with_recovery_key(
    config: &mut AppConfig,
    events: &[Event],
    device_keys: &Keys,
    recovery_keys: &Keys,
) -> Result<DriveRootEventApplyReport, relay_sync::RelayError> {
    let mut report = DriveRootEventApplyReport {
        seen: events.len(),
        ..DriveRootEventApplyReport::default()
    };
    for event in events {
        let mut outcome =
            relay_sync::apply_remote_drive_root_event(config, event, Some(device_keys))?;
        let mut opened_with = device_keys;
        if matches!(outcome, DriveRootApply::KeyUnavailable) {
            outcome =
                relay_sync::apply_remote_drive_root_event(config, event, Some(recovery_keys))?;
            opened_with = recovery_keys;
        }
        if let Ok((_, _, _, root)) =
            crate::nostr_events::parse_drive_root_event_for_device(event, opened_with)
            && !report.root_cids_to_download.contains(&root.root_cid)
        {
            report.root_cids_to_download.push(root.root_cid);
        }
        if matches!(outcome, DriveRootApply::Applied) {
            report.applied += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}

/// Republish this device's latest My Drive root when `ops` add a recovery
/// phrase, so the root it published before carries a wrap for the phrase.
///
/// Devices that publish their root on every start (the desktop daemon) get
/// the wrap that way; this covers devices that publish roots only on change.
pub async fn republish_root_for_added_recovery_phrase(
    config_dir: &std::path::Path,
    client: &nostr_sdk::Client,
    state: &ProfileState,
    ops: &[SignedNostrIdentityRosterOp],
) -> anyhow::Result<bool> {
    if !state.can_write_roots() || !ops_add_active_recovery_phrase(state, ops) {
        return Ok(false);
    }
    let config = AppConfig::load_or_default(crate::paths::config_path_in(config_dir))?;
    let Some(drive) = config.drive(crate::PRIMARY_DRIVE_ID) else {
        return Ok(false);
    };
    let Some(root) =
        crate::publishable_root::publishable_app_key_root(config_dir, drive, state).await?
    else {
        return Ok(false);
    };
    let app_key = crate::AppKey::load(crate::paths::key_path_in(config_dir))?;
    relay_sync::publish_drive_root(
        config_dir,
        client,
        app_key.keys(),
        &state.root_scope_id(),
        &drive.drive_id,
        &root,
        &drive_root_key_recipient_pubkeys(state, drive),
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
#[path = "recovery_wraps/tests.rs"]
mod tests;
