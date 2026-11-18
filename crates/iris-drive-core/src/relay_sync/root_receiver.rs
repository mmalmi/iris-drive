use std::collections::BTreeMap;

use nostr_sdk::{Event, Keys};

use crate::NostrIdentityId;
use crate::config::{AppConfig, AppKeyRootRef};
use crate::nostr_events::{
    parse_drive_root_event, parse_drive_root_event_for_device, parse_drive_root_event_preview,
};

use super::{DriveRootApply, RelayError};

/// Apply a remote drive-root event to `config`. Drops events for foreign root
/// scopes, unknown drives, unauthorized app actors, or roots older than what's
/// already recorded.
pub fn apply_remote_drive_root_event(
    config: &mut AppConfig,
    event: &Event,
    device_keys: Option<&Keys>,
) -> Result<DriveRootApply, RelayError> {
    let preview = parse_drive_root_event_preview(event)?;
    let app_key_hex = preview.app_key_pubkey_hex.clone();
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if preview.root_scope_id == account.root_scope_id() {
        if preview.drive_id == crate::private_trash::DRIVE_ID {
            // First contact may precede any local Trash action. Only register the
            // drive after a roster-authorized, decryptable event is applied.
            if !crate::private_trash::can_write(account, &app_key_hex) {
                return Ok(DriveRootApply::UnauthorizedAppKey);
            }
            if device_keys.is_some_and(|keys| {
                !crate::private_trash::can_open(account, &keys.public_key().to_hex())
            }) {
                return Ok(DriveRootApply::KeyUnavailable);
            }
            if let Some(drive) = config.drives.iter_mut().find(|drive| {
                drive.drive_id == crate::private_trash::DRIVE_ID
                    && drive.root_scope_id == preview.root_scope_id
            }) {
                return apply_root_to_app_key_roots(
                    &mut drive.app_key_roots,
                    event,
                    device_keys,
                    &preview,
                );
            }
            let mut drive = crate::private_trash::owner_drive(preview.root_scope_id.clone());
            let outcome = apply_root_to_app_key_roots(
                &mut drive.app_key_roots,
                event,
                device_keys,
                &preview,
            )?;
            if outcome == DriveRootApply::Applied {
                config.upsert_drive(drive);
            }
            return Ok(outcome);
        }
        let Some(drive_index) = config
            .drives
            .iter()
            .position(|drive| drive.drive_id == preview.drive_id)
        else {
            return Ok(DriveRootApply::UnknownDrive);
        };
        if !crate::drive_root_app_key_can_write_roots(
            account,
            &config.drives[drive_index],
            &app_key_hex,
        ) {
            return Ok(DriveRootApply::UnauthorizedAppKey);
        }
        let drive = &mut config.drives[drive_index];
        return apply_root_to_app_key_roots(&mut drive.app_key_roots, event, device_keys, &preview);
    }

    let Ok(share_id) = preview.root_scope_id.parse::<NostrIdentityId>() else {
        return Ok(DriveRootApply::NotOurScope);
    };
    if preview.drive_id != crate::PRIMARY_DRIVE_ID {
        return Ok(DriveRootApply::UnknownDrive);
    }
    let Some(shared_folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
    else {
        return Ok(DriveRootApply::NotOurScope);
    };
    let authorization =
        crate::shared_folder_app_key_write_authorization(shared_folder, &app_key_hex);
    if !authorization.is_authorized() {
        return Ok(DriveRootApply::UnauthorizedAppKey);
    }
    apply_root_to_app_key_roots(
        &mut shared_folder.app_key_roots,
        event,
        device_keys,
        &preview,
    )
}

fn apply_root_to_app_key_roots(
    app_key_roots: &mut BTreeMap<String, AppKeyRootRef>,
    event: &Event,
    device_keys: Option<&Keys>,
    preview: &crate::nostr_events::DriveRootEventPreview,
) -> Result<DriveRootApply, RelayError> {
    let app_key_hex = preview.app_key_pubkey_hex.clone();
    let existing = app_key_roots.get(&app_key_hex).cloned();
    let may_refresh_hint_placeholder = device_keys.is_some()
        && existing
            .as_ref()
            .is_some_and(app_key_root_ref_may_need_full_event_refresh);
    if !may_refresh_hint_placeholder
        && let Some(existing) = existing.as_ref()
        && incoming_root_is_stale(existing, preview.app_key_seq, preview.published_at)
    {
        return Ok(DriveRootApply::StaleTimestamp);
    }
    let (_, _, _, incoming_root) = if let Some(keys) = device_keys {
        match parse_drive_root_event_for_device(event, keys) {
            Ok(parsed) => parsed,
            Err(crate::nostr_events::WireError::RootKeyUnavailable) => {
                return Ok(DriveRootApply::KeyUnavailable);
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        parse_drive_root_event(event)?
    };
    if let Some(existing) = existing.as_ref() {
        if incoming_root_is_stale(existing, preview.app_key_seq, preview.published_at) {
            if incoming_full_root_refreshes_existing(existing, &incoming_root) {
                app_key_roots.insert(app_key_hex, incoming_root);
                return Ok(DriveRootApply::Applied);
            }
            return Ok(DriveRootApply::StaleTimestamp);
        }
        if existing.root_cid == incoming_root.root_cid {
            if incoming_full_root_refreshes_existing(existing, &incoming_root) {
                app_key_roots.insert(app_key_hex, incoming_root);
                return Ok(DriveRootApply::Applied);
            }
            return Ok(DriveRootApply::StaleTimestamp);
        }
    }
    app_key_roots.insert(app_key_hex, incoming_root);
    Ok(DriveRootApply::Applied)
}

fn app_key_root_ref_may_need_full_event_refresh(existing: &AppKeyRootRef) -> bool {
    existing.dck_generation == 0 || existing.parents.is_empty() || existing.observed.is_empty()
}

fn incoming_full_root_refreshes_existing(
    existing: &AppKeyRootRef,
    incoming: &AppKeyRootRef,
) -> bool {
    existing.root_cid == incoming.root_cid
        && (incoming.app_key_seq > existing.app_key_seq
            || incoming.dck_generation > existing.dck_generation
            || (existing.parents.is_empty() && !incoming.parents.is_empty())
            || (existing.observed.is_empty() && !incoming.observed.is_empty()))
}

fn incoming_root_is_stale(
    existing: &AppKeyRootRef,
    incoming_app_key_seq: u64,
    incoming_published_at: i64,
) -> bool {
    if existing.app_key_seq > 0 || incoming_app_key_seq > 0 {
        if incoming_app_key_seq == 0 {
            return true;
        }
        if existing.app_key_seq == 0 {
            return false;
        }
        return incoming_app_key_seq <= existing.app_key_seq;
    }
    existing.published_at >= incoming_published_at
}
