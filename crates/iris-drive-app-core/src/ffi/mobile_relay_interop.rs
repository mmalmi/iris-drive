use super::{AppConfig, Event, NativeAppKeyLinkRelayEventApply};

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use super::{BTreeSet, NATIVE_SYNC_RELAY_TIMEOUT_SECS, native_profile_roster_ops_pending_publish};
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
use super::{
    Mutex, Path, apply_native_app_key_link_relay_event_to_config, config_path_in, key_path_in,
};
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use nostr_sdk::JsonUtil;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) struct PersistedNativeRelayEventApply {
    pub(super) outcome: NativeAppKeyLinkRelayEventApply,
    pub(super) profile_id: Option<iris_drive_core::NostrIdentityId>,
    pub(super) approval_was_ready: bool,
    pub(super) approval_is_ready: bool,
    pub(super) drive_root_to_download: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApprovalReceiptRelayStep {
    SendAck,
    BackfillRoster,
}

pub(super) fn approval_receipt_relay_steps(
    is_approval_receipt: bool,
    outcome: NativeAppKeyLinkRelayEventApply,
    approval_is_ready: bool,
) -> [ApprovalReceiptRelayStep; 2] {
    debug_assert!(
        is_approval_receipt
            && matches!(
                outcome,
                NativeAppKeyLinkRelayEventApply::AppliedRoster
                    | NativeAppKeyLinkRelayEventApply::Current
            )
    );
    if approval_is_ready {
        [
            ApprovalReceiptRelayStep::SendAck,
            ApprovalReceiptRelayStep::BackfillRoster,
        ]
    } else {
        [
            ApprovalReceiptRelayStep::BackfillRoster,
            ApprovalReceiptRelayStep::SendAck,
        ]
    }
}

pub(super) fn native_device_approval_ack_is_ready(
    config_dir: &Path,
    config: &AppConfig,
) -> Result<bool, String> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(false);
    };
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(false);
    };
    let receipt_count = pending.approval_receipt_event.len();
    if receipt_count == 0 {
        return Ok(false);
    }
    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key for approval readiness: {error}"))?;
    let acknowledgements =
        iris_drive_core::app_key_link_transport::device_approval_applied_ack_events(
            state,
            device.keys(),
            super::unix_now_seconds(),
        )
        .map_err(|error| format!("checking device approval readiness: {error}"))?;
    Ok(acknowledgements.len() == receipt_count)
}

pub(super) fn apply_native_drive_root_relay_event_to_config(
    config: &mut AppConfig,
    event: &Event,
    device_keys: &nostr_sdk::Keys,
) -> Result<NativeAppKeyLinkRelayEventApply, String> {
    if event.kind.as_u16() != iris_drive_core::nostr_events::KIND_DRIVE_ROOT {
        return Ok(NativeAppKeyLinkRelayEventApply::Ignored);
    }
    let outcome = iris_drive_core::relay_sync::apply_remote_drive_root_event(
        config,
        event,
        Some(device_keys),
    )
    .map_err(|error| format!("applying Drive root relay event: {error}"))?;
    Ok(match outcome {
        iris_drive_core::relay_sync::DriveRootApply::Applied => {
            NativeAppKeyLinkRelayEventApply::AppliedDriveRoot
        }
        iris_drive_core::relay_sync::DriveRootApply::StaleTimestamp => {
            NativeAppKeyLinkRelayEventApply::Current
        }
        iris_drive_core::relay_sync::DriveRootApply::NotOurScope
        | iris_drive_core::relay_sync::DriveRootApply::UnknownDrive
        | iris_drive_core::relay_sync::DriveRootApply::UnauthorizedAppKey
        | iris_drive_core::relay_sync::DriveRootApply::KeyUnavailable => {
            NativeAppKeyLinkRelayEventApply::Ignored
        }
    })
}

pub(super) fn drive_root_event_download_target(
    config: &AppConfig,
    event: &Event,
    device_keys: &nostr_sdk::Keys,
    outcome: NativeAppKeyLinkRelayEventApply,
) -> Result<Option<String>, String> {
    if event.kind.as_u16() != iris_drive_core::nostr_events::KIND_DRIVE_ROOT
        || !matches!(
            outcome,
            NativeAppKeyLinkRelayEventApply::AppliedDriveRoot
                | NativeAppKeyLinkRelayEventApply::Current
        )
    {
        return Ok(None);
    }
    let (publisher, root_scope_id, drive_id, root) =
        iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device_keys)
            .map_err(|error| format!("reading current Drive root: {error}"))?;
    let current = config
        .drives
        .iter()
        .find(|drive| drive.root_scope_id == root_scope_id && drive.drive_id == drive_id)
        .and_then(|drive| drive.app_key_roots.get(&publisher))
        .or_else(|| {
            config
                .shared_folders
                .iter()
                .find(|folder| {
                    folder.share_id.to_string() == root_scope_id
                        && drive_id == iris_drive_core::PRIMARY_DRIVE_ID
                })
                .and_then(|folder| folder.app_key_roots.get(&publisher))
        });
    Ok(current
        .is_some_and(|current| {
            iris_drive_core::root_meta::root_cid_identity_matches(&current.root_cid, &root.root_cid)
        })
        .then_some(root.root_cid))
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) async fn publish_native_profile_roster_ops(
    config_dir: &Path,
    relay_client: &nostr_sdk::Client,
    state: &iris_drive_core::ProfileState,
    published_roster_op_ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    let pending_ops = native_profile_roster_ops_pending_publish(state, published_roster_op_ids);
    let pending_receipts = state
        .pending_device_approval_receipts
        .iter()
        .filter_map(|pending| Event::from_json(&pending.event_json).ok())
        .filter(|event| !published_roster_op_ids.contains(&event.id.to_hex()))
        .collect::<Vec<_>>();
    if pending_ops.is_empty() && pending_receipts.is_empty() {
        return Ok(());
    }

    // Do all local materialization and remote block upload before publishing
    // the roster authorization. A failed upload must not leave a newly
    // authorized AppKey waiting on a root it can never resolve.
    let handoff = if pending_receipts.is_empty() {
        None
    } else {
        Some(
            tokio::time::timeout(
                std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
                iris_drive_core::prepare_device_approval_root_handoff(config_dir),
            )
            .await
            .map_err(|_| "preparing native approval Drive root timed out".to_string())?
            .map_err(|error| format!("preparing native approval Drive root: {error:#}"))?,
        )
    };
    if !pending_ops.is_empty() {
        tokio::time::timeout(
            std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
            iris_drive_core::relay_sync::publish_nostr_identity_roster_ops(
                relay_client,
                &pending_ops,
            ),
        )
        .await
        .map_err(|_| "publishing native profile roster ops timed out".to_string())?
        .map_err(|error| format!("publishing native profile roster ops: {error}"))?;
    }

    for op in pending_ops {
        published_roster_op_ids.insert(op.op_id);
    }
    if let Some(handoff) = handoff.as_ref() {
        let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
            .map_err(|error| format!("loading AppKey for native approval root: {error}"))?;
        tokio::time::timeout(
            std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
            iris_drive_core::relay_sync::publish_drive_root(
                relay_client,
                app_key.keys(),
                &handoff.root_scope_id,
                &handoff.drive_id,
                &handoff.root,
                &handoff.authorized_app_keys,
            ),
        )
        .await
        .map_err(|_| "publishing native approval Drive root timed out".to_string())?
        .map_err(|error| format!("publishing native approval Drive root: {error}"))?;
        tracing::debug!(
            root_cid = handoff.root.root_cid,
            blossom_total_hashes = handoff.blossom_upload.total_hashes,
            blossom_uploaded = handoff.blossom_upload.uploaded,
            blossom_already_present = handoff.blossom_upload.already_present,
            "published durable native approval Drive root before receipt"
        );
    }
    for event in pending_receipts {
        let pending = state
            .pending_device_approval_receipts
            .iter()
            .find(|pending| pending.event_json == event.as_json())
            .ok_or_else(|| "pending native device approval receipt disappeared".to_string())?;
        tokio::time::timeout(
            std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
            iris_drive_core::relay_sync::publish_pending_device_approval_receipt(
                relay_client,
                pending,
            ),
        )
        .await
        .map_err(|_| "publishing native device approval receipt timed out".to_string())?
        .map_err(|error| format!("publishing native device approval receipt: {error}"))?;
        published_roster_op_ids.insert(event.id.to_hex());
    }
    Ok(())
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) async fn apply_and_persist_native_relay_event(
    config_dir: &Path,
    config_mutation: &Mutex<()>,
    event: &Event,
) -> Result<PersistedNativeRelayEventApply, String> {
    // FileProvider runs in a separate process and therefore cannot share the
    // native shell's in-memory mutex. Take its disk transaction lock first,
    // then re-read config only after both writers are serialized.
    let disk_mutation = iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .map_err(|error| format!("locking app-key-link relay event: {error}"))?;
    let config_mutation_guard = config_mutation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    let approval_was_ready = native_device_approval_ack_is_ready(config_dir, &config)?;
    #[cfg(test)]
    let approval_was_ready = false;
    let is_drive_root = event.kind.as_u16() == iris_drive_core::nostr_events::KIND_DRIVE_ROOT;
    let app_key = if is_drive_root {
        Some(
            iris_drive_core::AppKey::load(key_path_in(config_dir))
                .map_err(|error| format!("loading AppKey for Drive root: {error}"))?,
        )
    } else {
        None
    };
    let outcome = if let Some(app_key) = app_key.as_ref() {
        apply_native_drive_root_relay_event_to_config(&mut config, event, app_key.keys())?
    } else {
        apply_native_app_key_link_relay_event_to_config(&mut config, event)?
    };
    let approval_is_ready = native_device_approval_ack_is_ready(config_dir, &config)?;
    let drive_root_to_download = if let Some(app_key) = app_key.as_ref() {
        drive_root_event_download_target(&config, event, app_key.keys(), outcome)?
    } else {
        None
    };
    if matches!(
        outcome,
        NativeAppKeyLinkRelayEventApply::AppliedRoster
            | NativeAppKeyLinkRelayEventApply::AppliedApprovalAck
            | NativeAppKeyLinkRelayEventApply::AppliedDriveRoot
    ) {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving app-key-link relay event: {error}"))?;
    }
    let persisted = PersistedNativeRelayEventApply {
        outcome,
        profile_id: config.profile.as_ref().map(|state| state.profile_id),
        approval_was_ready,
        approval_is_ready,
        drive_root_to_download,
    };
    drop(config_mutation_guard);
    drop(disk_mutation);
    Ok(persisted)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) async fn backfill_native_device_approval_roster(
    config_dir: &Path,
    config_mutation: &Mutex<()>,
    relay_client: &nostr_sdk::Client,
    profile_id: iris_drive_core::NostrIdentityId,
) -> Result<bool, String> {
    let events = iris_drive_core::relay_sync::fetch_nostr_identity_roster_ops(
        relay_client,
        profile_id,
        iris_drive_core::relay_sync::DEVICE_APPROVAL_ROSTER_BACKFILL_TIMEOUT,
    )
    .await
    .map_err(|error| format!("fetching approved device roster: {error}"))?;
    let _disk_mutation = iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .map_err(|error| format!("locking approved device roster: {error}"))?;
    let _config_mutation = config_mutation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("reloading approval state: {error}"))?;
    let mut changed = false;
    for roster_event in &events {
        changed |= matches!(
            iris_drive_core::relay_sync::apply_remote_nostr_identity_roster_op_event(
                &mut config,
                roster_event,
            )
            .map_err(|error| format!("applying approved device roster: {error}"))?,
            iris_drive_core::relay_sync::NostrIdentityRosterOpApply::Applied
        );
    }
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving approved device roster: {error}"))?;
    }
    native_device_approval_ack_is_ready(config_dir, &config)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) async fn download_live_native_drive_root(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    event: &Event,
    root_cid: &str,
) -> Result<(), String> {
    let report = iris_drive_core::download_applied_drive_roots(
        config_dir,
        &[root_cid.to_owned()],
        Some(sync),
    )
    .await
    .map_err(|error| format!("downloading live Drive root: {error}"))?;
    let downloaded = report.fips_download.is_some() || report.blossom_download.is_some();
    if downloaded {
        iris_drive_core::paths::touch_provider_root_signal_in(config_dir)
            .map_err(|error| format!("signaling live Drive root: {error}"))?;
    } else {
        tracing::warn!(
            root_cid,
            fips_error = report.fips_download_error.as_deref().unwrap_or_default(),
            blossom_error = report.blossom_download_error.as_deref().unwrap_or_default(),
            "live Drive root metadata applied without available blocks"
        );
    }
    tracing::debug!(
        event_id = %event.id.to_hex(),
        root_cid,
        downloaded,
        "applied native Drive root over relay"
    );
    Ok(())
}
