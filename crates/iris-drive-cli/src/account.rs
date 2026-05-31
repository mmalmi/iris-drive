#[allow(clippy::wildcard_imports)]
use super::*;

mod device_link_urls;
pub(crate) use device_link_urls::*;
pub(crate) use iris_drive_core::device_link_transport::{
    DEVICE_LINK_REQUEST_APP_TOPIC, DEVICE_LINK_ROSTER_ACK_APP_TOPIC, DEVICE_LINK_ROSTER_APP_TOPIC,
    DeviceLinkRequestFrame, DeviceLinkRosterAckFrame, DeviceLinkRosterFrame,
};

pub(crate) fn cmd_init(
    config_dir: &std::path::Path,
    force: bool,
    label: Option<String>,
    username: Option<&str>,
    profile_photo: Option<&str>,
) -> Result<()> {
    if already_initialized(config_dir) && !force {
        eprintln!("iris-drive already initialized at {}", config_dir.display());
        eprintln!("use --force to print the existing state instead of erroring");
        return Err(anyhow::anyhow!("already initialized"));
    }
    let account = Account::create(config_dir, label).context("creating account")?;
    finish_account_init(
        config_dir,
        &account,
        UserProfile::from_optional(username, profile_photo),
    )
}

pub(crate) fn cmd_restore(
    config_dir: &std::path::Path,
    nsec: &str,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) && !force {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let account = Account::restore(config_dir, nsec, label).context("restoring account")?;
    finish_account_init(config_dir, &account, None)
}

pub(crate) fn cmd_link(
    config_dir: &std::path::Path,
    owner: &str,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    cmd_link_with_admin_device(config_dir, owner, None, force, label)
}

pub(crate) fn cmd_link_with_admin_device(
    config_dir: &std::path::Path,
    owner: &str,
    admin_device: Option<&str>,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) && !force {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let target = resolve_device_link_target_with_admin(owner, admin_device)?;
    let mut account =
        Account::link(config_dir, target.owner_hex, label).context("linking device")?;
    let link_secret = if target.link_secret.trim().is_empty() {
        account.state.device_link_secret.clone()
    } else {
        target.link_secret
    };
    if let Some(admin_device_hex) = target
        .admin_device_hex
        .or_else(|| Some(account.state.owner_pubkey.clone()))
    {
        account
            .state
            .queue_outbound_device_link_request(admin_device_hex, &link_secret, unix_now_seconds())
            .context("queueing device link request")?;
    }
    finish_account_init(config_dir, &account, None)
}

pub(crate) fn finish_account_init(
    config_dir: &std::path::Path,
    account: &Account,
    user_profile: Option<UserProfile>,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    config.account = Some(account.state.clone());
    if user_profile.is_some() {
        config.user_profile = user_profile;
    }
    if config.drive(PRIMARY_DRIVE_ID).is_none() {
        config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
    }
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "config_dir": config_dir.display().to_string(),
            "owner_npub": account_npub(&account.state.owner_pubkey),
            "device_npub": account_npub(&account.state.device_pubkey),
            "has_owner_signing_authority": account.state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(&account.state),
            "device_link_request": device_link_request_json(&account.state),
            "device_link_invite": device_link_invite_json(&account.state),
            "drives": config.drives.iter().map(|d| &d.drive_id).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

pub(crate) fn cmd_logout(config_dir: &std::path::Path) -> Result<()> {
    let report = iris_drive_core::logout_local_account(config_dir).context("logging out")?;
    println!(
        "{}",
        json!({
            "logged_out": true,
            "changed": report.changed(),
            "config_dir": config_dir.display().to_string(),
            "removed_key": report.removed_key,
            "removed_owner_key": report.removed_owner_key,
            "removed_sync_cache": report.removed_sync_cache,
            "cleared_account": report.cleared_account,
            "cleared_user_profile": report.cleared_user_profile,
            "cleared_drives": report.cleared_drives,
            "cleared_backup_targets": report.cleared_backup_targets,
        })
    );
    Ok(())
}

pub(crate) fn cmd_approve(
    config_dir: &std::path::Path,
    device: &str,
    label: Option<String>,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let (device_hex, label) = resolve_device_approval_input(device, &state.owner_pubkey, label)
        .context("parsing device approval request")?;
    let approved_device_npub = account_npub(&device_hex);
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account
        .approve_device(&device_hex, label)
        .context("approving device")?;
    let device_count = snap.devices.len();
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "approved_device_npub": approved_device_npub,
            "roster_size": device_count,
        })
    );
    Ok(())
}

pub(crate) fn cmd_revoke(config_dir: &std::path::Path, device: &str) -> Result<()> {
    let device_hex = normalize_pubkey(device).context("parsing device pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if state.device_pubkey == device_hex {
        return Err(anyhow::anyhow!("cannot revoke this device from itself"));
    }
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account
        .revoke_device(&device_hex)
        .context("revoking device")?;
    let device_count = snap.devices.len();
    let dck_generation = snap.dck_generation;
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "revoked_device_npub": account_npub(&device_hex),
            "roster_size": device_count,
            "dck_generation": dck_generation,
        })
    );
    Ok(())
}

pub(crate) fn cmd_appoint_admin(config_dir: &std::path::Path, device: &str) -> Result<()> {
    set_device_admin_role(config_dir, device, true)
}

pub(crate) fn cmd_demote_admin(config_dir: &std::path::Path, device: &str) -> Result<()> {
    set_device_admin_role(config_dir, device, false)
}

fn set_device_admin_role(
    config_dir: &std::path::Path,
    device: &str,
    make_admin: bool,
) -> Result<()> {
    let device_hex = normalize_pubkey(device).context("parsing device pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = if make_admin {
        account
            .appoint_admin(&device_hex)
            .context("promoting device to admin")?
    } else {
        account
            .demote_admin(&device_hex)
            .context("demoting device admin")?
    };
    let role = snap
        .device(&device_hex)
        .map_or(iris_drive_core::DeviceRole::Member, |device| device.role);
    let dck_generation = snap.dck_generation;
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "device_npub": account_npub(&device_hex),
            "role": device_role_label(role),
            "dck_generation": dck_generation,
        })
    );
    Ok(())
}

pub(crate) fn cmd_roster(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let snap = state.app_keys.as_ref();
    println!(
        "{}",
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "current_device_npub": account_npub(&state.device_pubkey),
            "authorization_state": authorization_state_label(&state),
            "device_link_invite": device_link_invite_json(&state),
            "inbound_device_link_requests": inbound_device_link_requests_json(&state),
            "app_keys": snap.map(|s| json!({
                "created_at": s.created_at,
                "dck_generation": s.dck_generation,
                "devices": s.devices.iter().map(|d| json!({
                    "pubkey": d.pubkey,
                    "npub": account_npub(&d.pubkey),
                    "added_at": d.added_at,
                    "label": d.label,
                    "role": device_role_label(d.role),
                    "is_current_device": d.pubkey == state.device_pubkey,
                    "has_dck_wrap": s.wrapped_dck.contains_key(&d.pubkey),
                })).collect::<Vec<_>>(),
            })),
        })
    );
    Ok(())
}

pub(crate) fn cmd_rotate_dck(config_dir: &std::path::Path) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account.rotate_dck().context("rotating DCK")?;
    let dck_gen = snap.dck_generation;
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "dck_generation": dck_gen,
            "device_wrap_count": account
                .state
                .app_keys
                .as_ref()
                .map_or(0, |s| s.wrapped_dck.len()),
        })
    );
    Ok(())
}

pub(crate) fn cmd_whoami(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    println!(
        "{}",
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "device_npub": account_npub(&state.device_pubkey),
            "has_owner_signing_authority": state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(&state),
            "device_link_request": device_link_request_json(&state),
            "device_link_invite": device_link_invite_json(&state),
            "inbound_device_link_requests": inbound_device_link_requests_json(&state),
        })
    );
    Ok(())
}

pub(crate) fn load_account_state(config_dir: &std::path::Path) -> Result<AccountState> {
    AppConfig::load_or_default(config_path_in(config_dir))?
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))
}

pub(crate) fn already_initialized(config_dir: &std::path::Path) -> bool {
    // An install is "initialized" when both a device key and a non-empty
    // config (with account) exist. Owner key may or may not be present
    // depending on flow (link installs don't have one).
    key_path_in(config_dir).exists()
        && config_path_in(config_dir).exists()
        && AppConfig::load_or_default(config_path_in(config_dir))
            .ok()
            .and_then(|c| c.account)
            .is_some()
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) const DEVICE_LINK_REQUEST_RETRY_SECS: u64 = 10;
pub(crate) const DEVICE_LINK_ROSTER_RETRY_SECS: u64 = 2;
pub(crate) const DEVICE_LINK_TICK_SECS: u64 = 1;

pub(crate) async fn send_pending_device_link_request(
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
    sent_cache: &mut BTreeMap<String, std::time::Instant>,
) -> Result<Option<Value>> {
    let Some(sync) = fips_blocks else {
        return Ok(None);
    };
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(None);
    };
    if state.can_manage_devices()
        || state.authorization_state != iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    {
        return Ok(None);
    }
    let Some(pending) = state.outbound_device_link_request.as_ref() else {
        return Ok(None);
    };

    let admin_npub = account_npub(&pending.admin_device_pubkey);
    let fingerprint = format!(
        "{}:{}:{}",
        pending.admin_device_pubkey, state.device_pubkey, pending.requested_at
    );
    let now = std::time::Instant::now();
    if sent_cache.get(&fingerprint).is_some_and(|last_sent| {
        now.duration_since(*last_sent)
            < std::time::Duration::from_secs(DEVICE_LINK_REQUEST_RETRY_SECS)
    }) {
        return Ok(None);
    }

    sync.refresh_authorized_peers(&config).await;
    let Some(frame) =
        iris_drive_core::device_link_transport::pending_device_link_request_frame(state)
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&frame)?;
    sync.send_app_message(&admin_npub, DEVICE_LINK_REQUEST_APP_TOPIC, bytes.clone())
        .await?;
    sent_cache.insert(fingerprint, now);

    Ok(Some(json!({
        "event": "device_link_request_sent",
        "topic": DEVICE_LINK_REQUEST_APP_TOPIC,
        "admin_device_npub": admin_npub,
        "device_npub": account_npub(&state.device_pubkey),
        "requested_at": pending.requested_at,
        "sent_bytes": bytes.len(),
    })))
}

pub(crate) async fn send_authorized_device_link_rosters(
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
    sent_cache: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<Option<Value>> {
    let Some(sync) = fips_blocks else {
        return Ok(None);
    };
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(None);
    };
    if !state.can_manage_devices() {
        return Ok(None);
    }
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(None);
    };
    if !app_keys.contains(&state.device_pubkey) {
        return Ok(None);
    }

    let now = std::time::Instant::now();
    let due_devices = app_keys
        .devices
        .iter()
        .filter(|device| device.pubkey != state.device_pubkey)
        .filter(|device| {
            let fingerprint = device_link_roster_fingerprint(device.pubkey.as_str(), app_keys);
            if acked_rosters.contains(&fingerprint) {
                return false;
            }
            !sent_cache.get(&fingerprint).is_some_and(|last_sent| {
                now.duration_since(*last_sent)
                    < std::time::Duration::from_secs(DEVICE_LINK_ROSTER_RETRY_SECS)
            })
        })
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    if due_devices.is_empty() {
        return Ok(None);
    }

    sync.refresh_authorized_peers(&config).await;
    let (event_id, event_json) = signed_roster_event_for_state(config_dir, state, app_keys)?;
    let frame = DeviceLinkRosterFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        admin_device_pubkey: state.device_pubkey.clone(),
        app_keys: app_keys.clone(),
        app_keys_event_id: event_id.clone(),
        app_keys_event_json: event_json,
        sent_at: unix_now_seconds(),
    };
    let bytes = serde_json::to_vec(&frame)?;
    let mut recipients = Vec::new();
    for device_pubkey in due_devices {
        let recipient_npub = account_npub(&device_pubkey);
        sync.send_app_message(&recipient_npub, DEVICE_LINK_ROSTER_APP_TOPIC, bytes.clone())
            .await?;
        sent_cache.insert(
            device_link_roster_fingerprint(&device_pubkey, app_keys),
            now,
        );
        recipients.push(recipient_npub);
    }

    Ok(Some(json!({
        "event": "device_link_roster_sent",
        "topic": DEVICE_LINK_ROSTER_APP_TOPIC,
        "recipient_device_npubs": recipients,
        "dck_generation": app_keys.dck_generation,
        "created_at": app_keys.created_at,
        "app_keys_event_id": event_id,
        "sent_bytes": bytes.len(),
    })))
}

fn signed_roster_event_for_state(
    config_dir: &Path,
    state: &AccountState,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> Result<(String, String)> {
    if let Some(record) = state.app_keys_event.as_ref()
        && record.signer_pubkey == app_keys.signer_pubkey()
    {
        return Ok((record.event_id.clone(), record.event_json.clone()));
    }
    if app_keys.signer_pubkey() != state.device_pubkey {
        return Err(anyhow::anyhow!(
            "cannot send roster: signed event is missing and this device was not the roster signer"
        ));
    }
    let account = Account::load(state.clone(), config_dir).context("loading account")?;
    let event =
        iris_drive_core::nostr_events::build_app_keys_event(account.device.keys(), app_keys)
            .context("building AppKeys roster event")?;
    Ok((event.id.to_hex(), event.as_json()))
}

fn device_link_roster_fingerprint(
    device_pubkey: &str,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> String {
    format!(
        "{}:{}:{}",
        device_pubkey, app_keys.created_at, app_keys.dck_generation
    )
}

pub(crate) async fn handle_device_link_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    fips_blocks: Option<&FsFipsBlockSync>,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool> {
    match message.topic.as_str() {
        DEVICE_LINK_REQUEST_APP_TOPIC => {
            handle_device_link_request_app_message(config_dir, message).await
        }
        DEVICE_LINK_ROSTER_APP_TOPIC => {
            handle_device_link_roster_app_message(config_dir, message, fips_blocks).await
        }
        DEVICE_LINK_ROSTER_ACK_APP_TOPIC => {
            handle_device_link_roster_ack_app_message(config_dir, message, acked_rosters)
        }
        _ => Ok(false),
    }
}

async fn handle_device_link_request_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool> {
    let frame: DeviceLinkRequestFrame =
        serde_json::from_slice(&message.data).context("parsing device link request frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported device link request schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey).context("parsing link request owner")?;
    let device_hex =
        normalize_pubkey(&frame.device_pubkey).context("parsing link request device")?;
    let link_secret = if frame.link_secret.trim().is_empty() {
        decode_device_approval_request(&frame.url)?
            .map(|request| request.link_secret)
            .unwrap_or_default()
    } else {
        frame.link_secret
    };

    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_mut() else {
        return Ok(true);
    };
    let changed = state
        .record_inbound_device_link_request(
            &owner_hex,
            &device_hex,
            frame.label,
            &link_secret,
            frame.requested_at,
        )
        .context("recording inbound device link request")?;
    if changed {
        config.save(config_path_in(config_dir))?;
        println!(
            "{}",
            json!({
                "event": "device_link_request_received",
                "topic": DEVICE_LINK_REQUEST_APP_TOPIC,
                "peer": message.peer_id,
                "device_npub": account_npub(&device_hex),
                "requested_at": frame.requested_at,
            })
        );
    }
    Ok(true)
}

#[allow(clippy::too_many_lines)]
async fn handle_device_link_roster_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<bool> {
    let frame: DeviceLinkRosterFrame =
        serde_json::from_slice(&message.data).context("parsing device link roster frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported device link roster schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey).context("parsing roster owner")?;
    let admin_device_hex =
        normalize_pubkey(&frame.admin_device_pubkey).context("parsing roster admin device")?;
    let sender_hex = normalize_pubkey(&message.peer_id).ok();

    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_mut() else {
        return Ok(true);
    };
    if state.can_manage_devices() || state.owner_pubkey != owner_hex {
        return Ok(true);
    }
    if sender_hex.as_deref() != Some(admin_device_hex.as_str()) {
        return Ok(true);
    }
    if frame.app_keys_event_json.is_empty() || frame.app_keys_event_id.is_empty() {
        return Ok(true);
    }
    let roster_event = nostr_sdk::Event::from_json(&frame.app_keys_event_json)
        .context("parsing signed roster event")?;
    if roster_event.id.to_hex() != frame.app_keys_event_id {
        return Ok(true);
    }
    let parsed = iris_drive_core::nostr_events::parse_app_keys_event(&roster_event)
        .context("parsing signed roster AppKeys")?;
    if roster_event.pubkey.to_hex() != admin_device_hex
        || parsed.owner_pubkey != state.owner_pubkey
        || !parsed.contains(&state.device_pubkey)
        || !parsed.is_admin(&admin_device_hex)
    {
        return Ok(true);
    }

    let should_apply = state
        .outbound_device_link_request
        .as_ref()
        .is_some_and(|pending| pending.admin_device_pubkey == admin_device_hex);
    let already_current = state.app_keys.as_ref() == Some(&parsed);
    if !should_apply && !already_current {
        return Ok(true);
    }

    let outcome = if should_apply {
        iris_drive_core::relay_sync::apply_remote_app_keys_event(&mut config, &roster_event)
            .context("applying signed roster event")?
    } else {
        iris_drive_core::relay_sync::AppKeysApply::Applied(iris_drive_core::ApplyDecision::Rejected)
    };
    let decision = match outcome {
        iris_drive_core::relay_sync::AppKeysApply::Applied(decision) => decision,
        iris_drive_core::relay_sync::AppKeysApply::NotOurOwner
        | iris_drive_core::relay_sync::AppKeysApply::UnauthorizedSigner => {
            iris_drive_core::ApplyDecision::Rejected
        }
    };
    let state = config.account.as_ref().expect("account still present");
    let accepted = should_apply && decision != iris_drive_core::ApplyDecision::Rejected;
    let ack_data = if accepted || already_current {
        Some((
            state.device_pubkey.clone(),
            state
                .app_keys
                .as_ref()
                .expect("accepted/current app keys")
                .clone(),
            roster_event.id.to_hex(),
        ))
    } else {
        None
    };
    if accepted {
        let authorization_state = authorization_state_label(state);
        config.save(config_path_in(config_dir))?;
        println!(
            "{}",
            json!({
                "event": "device_link_roster_received",
                "topic": DEVICE_LINK_ROSTER_APP_TOPIC,
                "peer": message.peer_id,
                "admin_device_npub": account_npub(&admin_device_hex),
                "authorization_state": authorization_state,
                "apply_decision": format!("{decision:?}").to_ascii_lowercase(),
            })
        );
    }
    if let Some((device_pubkey, app_keys, app_keys_event_id)) = ack_data {
        send_device_link_roster_ack(
            fips_blocks,
            &admin_device_hex,
            &owner_hex,
            &device_pubkey,
            &app_keys_event_id,
            &app_keys,
        )
        .await?;
    }
    Ok(true)
}

fn handle_device_link_roster_ack_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool> {
    let frame: DeviceLinkRosterAckFrame =
        serde_json::from_slice(&message.data).context("parsing device link roster ack frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported device link roster ack schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey).context("parsing ack owner")?;
    let admin_device_hex =
        normalize_pubkey(&frame.admin_device_pubkey).context("parsing ack admin device")?;
    let device_hex = normalize_pubkey(&frame.device_pubkey).context("parsing ack device")?;
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(device_hex.as_str()) {
        return Ok(true);
    }

    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(true);
    };
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(true);
    };
    if !state.can_manage_devices()
        || state.owner_pubkey != owner_hex
        || state.device_pubkey != admin_device_hex
        || !app_keys.contains(&device_hex)
        || app_keys.created_at != frame.app_keys_created_at
        || app_keys.dck_generation != frame.dck_generation
    {
        return Ok(true);
    }

    let fingerprint = device_link_roster_fingerprint(&device_hex, app_keys);
    let changed = acked_rosters.insert(fingerprint);
    if changed {
        println!(
            "{}",
            json!({
                        "event": "device_link_roster_ack_received",
                        "topic": DEVICE_LINK_ROSTER_ACK_APP_TOPIC,
                        "device_npub": account_npub(&device_hex),
                "dck_generation": app_keys.dck_generation,
                "created_at": app_keys.created_at,
                "app_keys_event_id": frame.app_keys_event_id,
            })
        );
    }
    Ok(true)
}

async fn send_device_link_roster_ack(
    fips_blocks: Option<&FsFipsBlockSync>,
    admin_device_hex: &str,
    owner_hex: &str,
    device_hex: &str,
    app_keys_event_id: &str,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> Result<()> {
    let Some(sync) = fips_blocks else {
        return Ok(());
    };
    let frame = DeviceLinkRosterAckFrame {
        schema: 1,
        owner_pubkey: owner_hex.to_string(),
        admin_device_pubkey: admin_device_hex.to_string(),
        device_pubkey: device_hex.to_string(),
        app_keys_event_id: app_keys_event_id.to_string(),
        app_keys_created_at: app_keys.created_at,
        dck_generation: app_keys.dck_generation,
        acknowledged_at: unix_now_seconds(),
    };
    sync.send_app_message(
        &account_npub(admin_device_hex),
        DEVICE_LINK_ROSTER_ACK_APP_TOPIC,
        serde_json::to_vec(&frame)?,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

pub(crate) fn account_npub(hex: &str) -> String {
    use nostr_sdk::nips::nip19::ToBech32;
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
}

pub(crate) fn authorization_state_label(state: &AccountState) -> &'static str {
    use iris_drive_core::DeviceAuthorizationState as S;
    match state.authorization_state {
        S::Authorized => "authorized",
        S::AwaitingApproval => "awaiting_approval",
        S::Revoked => "revoked",
    }
}

pub(crate) fn device_role_label(role: iris_drive_core::DeviceRole) -> &'static str {
    match role {
        iris_drive_core::DeviceRole::Admin => "admin",
        iris_drive_core::DeviceRole::Member => "member",
    }
}

pub(crate) fn drive_role_label(role: DriveRole) -> &'static str {
    match role {
        DriveRole::Owner => "owner",
        DriveRole::Editor => "editor",
        DriveRole::Reader => "reader",
    }
}

pub(crate) fn short_pubkey(pk: &str) -> String {
    if pk.len() > 14 {
        format!("{}…{}", &pk[..6], &pk[pk.len() - 6..])
    } else {
        pk.to_string()
    }
}
