#[allow(clippy::wildcard_imports)]
use super::*;

mod app_key_link_urls;
pub(crate) use app_key_link_urls::*;
#[cfg(test)]
pub(crate) use iris_drive_core::app_key_link_transport::app_key_link_roster_fingerprint;
pub(crate) use iris_drive_core::app_key_link_transport::{
    APP_KEY_LINK_REQUEST_APP_TOPIC, APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
    APP_KEY_LINK_ROSTER_APP_TOPIC, AppKeyLinkRequestFrame, AppKeyLinkRosterAckFrame,
    AppKeyLinkRosterFrame, app_key_link_roster_ack_frame, app_key_link_roster_ack_matches_state,
    app_key_link_roster_frame, app_key_link_roster_recipients,
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
    let profile = Profile::create(config_dir, label).context("creating profile")?;
    finish_profile_init(
        config_dir,
        &profile,
        UserProfile::from_optional(username, profile_photo),
    )
}

pub(crate) fn cmd_restore(
    config_dir: &std::path::Path,
    recovery_secret: &str,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) && !force {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let profile =
        Profile::restore(config_dir, recovery_secret, label).context("restoring profile")?;
    finish_profile_init(config_dir, &profile, None)
}

pub(crate) fn cmd_recover_app_key(
    config_dir: &std::path::Path,
    recovery_phrase: Option<&str>,
    label: Option<String>,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive restore` first"))?;
    let loaded_from_disk = recovery_phrase.is_none_or(|phrase| phrase.trim().is_empty());
    let phrase = if loaded_from_disk {
        iris_drive_core::recovery_phrase::load_recovery_phrase(
            iris_drive_core::paths::recovery_phrase_path_in(config_dir),
        )
        .context("loading saved recovery phrase")?
    } else {
        recovery_phrase.expect("checked above").trim().to_string()
    };
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    profile
        .admit_current_app_key_with_recovery_phrase(&phrase, label)
        .context("recovering app key")?;
    let dck_generation = profile
        .state
        .app_keys
        .as_ref()
        .map_or(0, |snapshot| snapshot.dck_generation);
    let mut output = profile_identity_json_map(&profile.state);
    output.insert("dck_generation".to_string(), json!(dck_generation));
    output.insert(
        "profile_roster_op_count".to_string(),
        json!(profile.state.profile_roster_ops.len()),
    );
    output.insert(
        "loaded_recovery_phrase_from_disk".to_string(),
        json!(loaded_from_disk),
    );
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!("{}", Value::Object(output));
    Ok(())
}

pub(crate) fn cmd_link(
    config_dir: &std::path::Path,
    invite: &str,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    cmd_link_with_admin_app_key(config_dir, invite, None, force, label)
}

pub(crate) fn cmd_link_with_admin_app_key(
    config_dir: &std::path::Path,
    invite_or_profile: &str,
    admin_app_key: Option<&str>,
    force: bool,
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) && !force {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let target = resolve_app_key_link_target_with_admin(invite_or_profile, admin_app_key)?;
    let mut profile = Profile::link_to_profile(
        config_dir,
        target.profile_id,
        target.admin_app_key_hex.clone(),
        label,
    )
    .context("linking AppKey")?;
    let link_secret = if target.link_secret.trim().is_empty() {
        profile.state.app_key_link_secret.clone()
    } else {
        target.link_secret
    };
    profile
        .state
        .queue_outbound_app_key_link_request(
            target.admin_app_key_hex,
            &link_secret,
            unix_now_seconds(),
        )
        .context("queueing app-key link request")?;
    finish_profile_init(config_dir, &profile, None)
}

pub(crate) fn finish_profile_init(
    config_dir: &std::path::Path,
    profile: &Profile,
    user_profile: Option<UserProfile>,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    config.profile = Some(profile.state.clone());
    if user_profile.is_some() {
        config.user_profile = user_profile;
    }
    if config.drive(PRIMARY_DRIVE_ID).is_none() {
        config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    }
    config.save(config_path_in(config_dir))?;
    let mut output = profile_identity_json_map(&profile.state);
    output.insert(
        "config_dir".to_string(),
        json!(config_dir.display().to_string()),
    );
    output.insert(
        "app_key_link_request".to_string(),
        app_key_link_request_json(&profile.state),
    );
    output.insert(
        "app_key_link_invite".to_string(),
        app_key_link_invite_json(&profile.state),
    );
    output.insert(
        "drives".to_string(),
        json!(
            config
                .drives
                .iter()
                .map(|d| &d.drive_id)
                .collect::<Vec<_>>()
        ),
    );
    println!("{}", Value::Object(output));
    Ok(())
}

pub(crate) fn cmd_logout(config_dir: &std::path::Path) -> Result<()> {
    let report = iris_drive_core::logout_local_profile(config_dir).context("logging out")?;
    println!(
        "{}",
        json!({
            "logged_out": true,
            "changed": report.changed(),
            "config_dir": config_dir.display().to_string(),
            "removed_key": report.removed_key,
            "removed_recovery_phrase": report.removed_recovery_phrase,
            "removed_sync_cache": report.removed_sync_cache,
            "cleared_profile": report.cleared_profile,
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
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let (app_key_hex, label) = resolve_app_key_approval_input(device, state.profile_id, label)
        .context("parsing AppKey approval request")?;
    let approved_app_key_npub = pubkey_npub(&app_key_hex);
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let snap = profile
        .approve_app_key(&app_key_hex, label)
        .context("approving AppKey")?;
    let device_count = snap.app_actors.len();
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "approved_app_key_npub": approved_app_key_npub,
            "roster_size": device_count,
        })
    );
    Ok(())
}

pub(crate) fn cmd_reject(config_dir: &std::path::Path, device: &str) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if !state.can_admin_profile() {
        return Err(anyhow::anyhow!(
            "this AppKey is not an admin - only admin AppKeys can reject AppKey-link requests"
        ));
    }
    let (app_key_hex, _) = resolve_app_key_approval_input(device, state.profile_id, None)
        .context("parsing AppKey rejection request")?;
    let rejected = state
        .reject_inbound_app_key_link_request(&app_key_hex)
        .context("rejecting AppKey request")?;
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "rejected": rejected,
            "rejected_app_key_npub": pubkey_npub(&app_key_hex),
            "inbound_app_key_link_requests": inbound_app_key_link_requests_json(
                config.profile.as_ref().expect("profile still present")
            ),
        })
    );
    Ok(())
}

pub(crate) fn cmd_revoke(config_dir: &std::path::Path, device: &str) -> Result<()> {
    let app_key_hex = normalize_pubkey(device).context("parsing AppKey pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if state.app_key_pubkey == app_key_hex {
        return Err(anyhow::anyhow!("cannot revoke this AppKey from itself"));
    }
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let snap = profile
        .revoke_app_key(&app_key_hex)
        .context("revoking AppKey")?;
    let device_count = snap.app_actors.len();
    let dck_generation = snap.dck_generation;
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "revoked_app_key_npub": pubkey_npub(&app_key_hex),
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
    let app_key_hex = normalize_pubkey(device).context("parsing AppKey pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let snap = if make_admin {
        profile
            .appoint_admin(&app_key_hex)
            .context("promoting AppKey to admin")?
    } else {
        profile
            .demote_admin(&app_key_hex)
            .context("demoting AppKey admin")?
    };
    let role = snap
        .app_actor(&app_key_hex)
        .map_or(iris_drive_core::AppActorRole::Member, |actor| actor.role);
    let dck_generation = snap.dck_generation;
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "app_key_npub": pubkey_npub(&app_key_hex),
            "role": app_actor_role_label(role),
            "dck_generation": dck_generation,
        })
    );
    Ok(())
}

pub(crate) fn cmd_roster(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let snap = state.app_keys.as_ref();
    let mut output = profile_identity_json_map(&state);
    output.insert(
        "app_key_link_invite".to_string(),
        app_key_link_invite_json(&state),
    );
    output.insert(
        "inbound_app_key_link_requests".to_string(),
        json!(inbound_app_key_link_requests_json(&state)),
    );
    output.insert(
        "app_keys".to_string(),
        snap.map_or(Value::Null, |s| {
            json!({
                "created_at": s.created_at,
                "dck_generation": s.dck_generation,
                "app_actors": s.app_actors.iter().map(|actor| json!({
                    "pubkey": actor.pubkey,
                    "npub": pubkey_npub(&actor.pubkey),
                    "added_at": actor.added_at,
                    "label": actor.label,
                    "role": app_actor_role_label(actor.role),
                    "is_current_app_key": actor.pubkey == state.app_key_pubkey,
                    "has_dck_wrap": s.wrapped_dck.contains_key(&actor.pubkey),
                })).collect::<Vec<_>>(),
            })
        }),
    );
    println!("{}", Value::Object(output));
    Ok(())
}

pub(crate) fn cmd_rotate_dck(config_dir: &std::path::Path) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let snap = profile.rotate_dck().context("rotating DCK")?;
    let dck_gen = snap.dck_generation;
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "dck_generation": dck_gen,
            "device_wrap_count": profile
                .state
                .app_keys
                .as_ref()
                .map_or(0, |s| s.wrapped_dck.len()),
        })
    );
    Ok(())
}

pub(crate) fn cmd_repair_key_wraps(config_dir: &std::path::Path) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let repair = profile
        .repair_current_key_epoch_wraps()
        .context("repairing current key epoch wraps")?;
    let remaining_missing_key_wraps = profile
        .state
        .profile_projection()
        .active_key_recipients_missing_wraps(repair.epoch)
        .iter()
        .map(|pubkey| pubkey_npub(pubkey))
        .collect::<Vec<_>>();
    let repaired_key_wraps = repair
        .repaired_pubkeys
        .iter()
        .map(|pubkey| pubkey_npub(pubkey))
        .collect::<Vec<_>>();
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "epoch": repair.epoch,
            "dck_generation": repair.snapshot.dck_generation,
            "repaired_key_wrap_count": repair.repaired_pubkeys.len(),
            "repaired_key_wraps": repaired_key_wraps,
            "remaining_missing_key_wraps": remaining_missing_key_wraps,
        })
    );
    Ok(())
}

pub(crate) fn cmd_whoami(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut output = profile_identity_json_map(&state);
    output.insert(
        "app_key_link_request".to_string(),
        app_key_link_request_json(&state),
    );
    output.insert(
        "app_key_link_invite".to_string(),
        app_key_link_invite_json(&state),
    );
    output.insert(
        "inbound_app_key_link_requests".to_string(),
        json!(inbound_app_key_link_requests_json(&state)),
    );
    println!("{}", Value::Object(output));
    Ok(())
}

pub(crate) fn load_profile_state(config_dir: &std::path::Path) -> Result<ProfileState> {
    AppConfig::load_or_default(config_path_in(config_dir))?
        .profile
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))
}

pub(crate) fn already_initialized(config_dir: &std::path::Path) -> bool {
    // An install is "initialized" when both an AppKey and a non-empty
    // config with profile state exist.
    key_path_in(config_dir).exists()
        && config_path_in(config_dir).exists()
        && AppConfig::load_or_default(config_path_in(config_dir))
            .ok()
            .and_then(|c| c.profile)
            .is_some()
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn profile_identity_json_map(state: &ProfileState) -> serde_json::Map<String, Value> {
    let summary = iris_drive_core::device_summary::iris_profile_summary(state);
    let mut output = serde_json::Map::new();
    output.insert("profile".to_string(), iris_profile_summary_json(&summary));
    output.insert("profile_id".to_string(), json!(summary.profile_id));
    output.insert(
        "current_app_key_pubkey".to_string(),
        json!(summary.current_app_key_pubkey_hex),
    );
    output.insert(
        "current_app_key_npub".to_string(),
        json!(summary.current_app_key_npub),
    );
    output.insert(
        "current_app_key_label".to_string(),
        json!(summary.current_app_key_label),
    );
    output.insert(
        "authorization_state".to_string(),
        json!(summary.authorization_state),
    );
    output.insert(
        "can_write_roots".to_string(),
        json!(summary.can_write_roots),
    );
    output.insert(
        "can_admin_profile".to_string(),
        json!(summary.can_admin_profile),
    );
    output
}

fn iris_profile_summary_json(
    summary: &iris_drive_core::device_summary::IrisProfileSummary,
) -> Value {
    json!({
        "profile_id": &summary.profile_id,
        "current_app_key_pubkey": &summary.current_app_key_pubkey_hex,
        "current_app_key_npub": &summary.current_app_key_npub,
        "current_app_key_label": &summary.current_app_key_label,
        "authorization_state": &summary.authorization_state,
        "can_write_roots": summary.can_write_roots,
        "can_admin_profile": summary.can_admin_profile,
        "active_app_key_count": summary.active_app_key_count,
        "profile_roster_op_count": summary.profile_roster_op_count,
        "current_key_epoch": summary.current_key_epoch,
        "recovery_phrase_facet_count": summary.recovery_phrase_facet_count,
        "nip46_facet_count": summary.nip46_facet_count,
        "social_profile_facet_count": summary.social_profile_facet_count,
        "missing_key_wraps": summary.missing_key_wrap_npubs,
    })
}

pub(crate) const APP_KEY_LINK_REQUEST_RETRY_SECS: u64 = 10;
pub(crate) const APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS: u64 = 250;
pub(crate) const APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS: u8 = 40;
pub(crate) const APP_KEY_LINK_ROSTER_RETRY_SECS: u64 = 2;
pub(crate) const APP_KEY_LINK_TICK_MILLIS: u64 = 250;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SentAppKeyLinkRequest {
    last_sent: std::time::Instant,
    attempts: u8,
}

fn app_key_link_request_send_due(
    sent: Option<SentAppKeyLinkRequest>,
    now: std::time::Instant,
) -> bool {
    let Some(sent) = sent else {
        return true;
    };
    now.duration_since(sent.last_sent) >= app_key_link_request_retry_interval(sent.attempts)
}

fn app_key_link_request_retry_interval(attempts: u8) -> std::time::Duration {
    if attempts < APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS {
        std::time::Duration::from_millis(APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS)
    } else {
        std::time::Duration::from_secs(APP_KEY_LINK_REQUEST_RETRY_SECS)
    }
}

pub(crate) async fn send_pending_app_key_link_request(
    config_dir: &Path,
    client: &nostr_sdk::Client,
    fips_blocks: Option<&FsFipsBlockSync>,
    sent_cache: &mut BTreeMap<String, SentAppKeyLinkRequest>,
) -> Result<Option<Value>> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    if state.can_admin_profile()
        || state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(None);
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(None);
    };

    let admin_npub = pubkey_npub(&pending.admin_app_key_pubkey);
    let fingerprint = format!(
        "{}:{}:{}",
        pending.admin_app_key_pubkey, state.app_key_pubkey, pending.requested_at
    );
    let now = std::time::Instant::now();
    if !app_key_link_request_send_due(sent_cache.get(&fingerprint).copied(), now) {
        return Ok(None);
    }

    let Some(frame) =
        iris_drive_core::app_key_link_transport::pending_app_key_link_request_frame(state)
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&frame)?;
    let device =
        iris_drive_core::AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let relay_event_id =
        iris_drive_core::relay_sync::publish_app_key_link_request(client, device.keys(), &frame)
            .await?;
    let mut fips_sent = false;
    let mut fips_error = None;
    if let Some(sync) = fips_blocks {
        sync.refresh_authorized_peers(&config).await;
        match sync
            .send_app_message(&admin_npub, APP_KEY_LINK_REQUEST_APP_TOPIC, bytes.clone())
            .await
        {
            Ok(()) => {
                fips_sent = true;
            }
            Err(error) => {
                fips_error = Some(error.to_string());
            }
        }
    }
    let attempts = sent_cache
        .get(&fingerprint)
        .map_or(1, |sent| sent.attempts.saturating_add(1));
    sent_cache.insert(
        fingerprint,
        SentAppKeyLinkRequest {
            last_sent: now,
            attempts,
        },
    );

    Ok(Some(json!({
        "event": "app_key_link_request_sent",
        "topic": APP_KEY_LINK_REQUEST_APP_TOPIC,
        "admin_app_key_npub": admin_npub,
        "app_key_npub": pubkey_npub(&state.app_key_pubkey),
        "requested_at": pending.requested_at,
        "sent_bytes": bytes.len(),
        "relay_event_id": relay_event_id.to_hex(),
        "sent_over_relay": true,
        "sent_over_fips": fips_sent,
        "fips_error": fips_error,
    })))
}

pub(crate) async fn send_authorized_app_key_link_rosters(
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
    sent_cache: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<Option<Value>> {
    let Some(sync) = fips_blocks else {
        return Ok(None);
    };
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    if !state.can_admin_profile() {
        return Ok(None);
    }
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(None);
    };
    if !app_keys.contains(&state.app_key_pubkey) {
        return Ok(None);
    }

    let now = std::time::Instant::now();
    let due_devices = app_key_link_roster_recipients(state)
        .into_iter()
        .filter(|recipient| {
            if acked_rosters.contains(&recipient.roster_fingerprint) {
                return false;
            }
            !sent_cache
                .get(&recipient.roster_fingerprint)
                .is_some_and(|last_sent| {
                    now.duration_since(*last_sent)
                        < std::time::Duration::from_secs(APP_KEY_LINK_ROSTER_RETRY_SECS)
                })
        })
        .collect::<Vec<_>>();
    if due_devices.is_empty() {
        return Ok(None);
    }

    sync.refresh_authorized_peers(&config).await;
    let Some(frame) = app_key_link_roster_frame(state, unix_now_seconds()) else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&frame)?;
    let mut recipients = Vec::new();
    for recipient in due_devices {
        let recipient_npub = pubkey_npub(&recipient.app_key_pubkey);
        sync.send_app_message(
            &recipient_npub,
            APP_KEY_LINK_ROSTER_APP_TOPIC,
            bytes.clone(),
        )
        .await?;
        sent_cache.insert(recipient.roster_fingerprint, now);
        recipients.push(recipient_npub);
    }

    Ok(Some(json!({
        "event": "app_key_link_roster_sent",
        "topic": APP_KEY_LINK_ROSTER_APP_TOPIC,
        "recipient_app_key_npubs": recipients,
        "dck_generation": app_keys.dck_generation,
        "created_at": app_keys.created_at,
        "sent_bytes": bytes.len(),
    })))
}

pub(crate) async fn handle_app_key_link_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    fips_blocks: Option<&FsFipsBlockSync>,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool> {
    match message.topic.as_str() {
        APP_KEY_LINK_REQUEST_APP_TOPIC => {
            handle_app_key_link_request_app_message(config_dir, message).await
        }
        APP_KEY_LINK_ROSTER_APP_TOPIC => {
            handle_app_key_link_roster_app_message(config_dir, message, fips_blocks).await
        }
        APP_KEY_LINK_ROSTER_ACK_APP_TOPIC => {
            handle_app_key_link_roster_ack_app_message(config_dir, message, acked_rosters)
        }
        _ => Ok(false),
    }
}

async fn handle_app_key_link_request_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool> {
    let frame: AppKeyLinkRequestFrame =
        serde_json::from_slice(&message.data).context("parsing app-key link request frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported app-key link request schema {}",
            frame.schema
        ));
    }
    let app_key_hex =
        normalize_pubkey(&frame.app_key_pubkey).context("parsing link request device")?;
    let link_secret = if frame.link_secret.trim().is_empty() {
        decode_app_key_approval_request(&frame.url)?
            .map(|request| request.link_secret)
            .unwrap_or_default()
    } else {
        frame.link_secret
    };

    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_mut() else {
        return Ok(true);
    };
    let changed = state
        .record_inbound_app_key_link_request(
            frame.profile_id,
            &app_key_hex,
            frame.label,
            &link_secret,
            frame.requested_at,
        )
        .context("recording inbound app-key link request")?;
    if changed {
        config.save(config_path_in(config_dir))?;
        println!(
            "{}",
            json!({
                "event": "app_key_link_request_received",
                "topic": APP_KEY_LINK_REQUEST_APP_TOPIC,
                "peer": message.peer_id,
                "app_key_npub": pubkey_npub(&app_key_hex),
                "requested_at": frame.requested_at,
            })
        );
    }
    Ok(true)
}

#[allow(clippy::too_many_lines)]
async fn handle_app_key_link_roster_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<bool> {
    let frame: AppKeyLinkRosterFrame =
        serde_json::from_slice(&message.data).context("parsing app-key link roster frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported app-key link roster schema {}",
            frame.schema
        ));
    }
    let admin_app_key_hex =
        normalize_pubkey(&frame.admin_app_key_pubkey).context("parsing roster admin AppKey")?;
    let sender_hex = normalize_pubkey(&message.peer_id).ok();

    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_mut() else {
        return Ok(true);
    };
    if state.can_admin_profile() {
        return Ok(true);
    }
    if sender_hex.as_deref() != Some(admin_app_key_hex.as_str()) {
        return Ok(true);
    }

    let outcome = iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut config,
        &frame,
        &admin_app_key_hex,
    )
    .context("applying signed profile roster ops")?;
    let accepted = match outcome {
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Current => true,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision) => {
            decision != iris_drive_core::ApplyDecision::Rejected
        }
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Ignored => false,
    };
    let changed = matches!(
        outcome,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision)
            if decision != iris_drive_core::ApplyDecision::Rejected
    );
    let state = config.profile.as_ref().expect("profile still present");
    let ack_frame = if accepted {
        app_key_link_roster_ack_frame(state, &admin_app_key_hex, unix_now_seconds())
    } else {
        None
    };
    if changed {
        let authorization_state = authorization_state_label(state);
        config.save(config_path_in(config_dir))?;
        println!(
            "{}",
            json!({
                "event": "app_key_link_roster_received",
                "topic": APP_KEY_LINK_ROSTER_APP_TOPIC,
                "peer": message.peer_id,
                "admin_app_key_npub": pubkey_npub(&admin_app_key_hex),
                "authorization_state": authorization_state,
                "apply_decision": format!("{outcome:?}").to_ascii_lowercase(),
            })
        );
    }
    if let Some(frame) = ack_frame {
        send_app_key_link_roster_ack(fips_blocks, &frame).await?;
    }
    Ok(true)
}

fn handle_app_key_link_roster_ack_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool> {
    let frame: AppKeyLinkRosterAckFrame =
        serde_json::from_slice(&message.data).context("parsing app-key link roster ack frame")?;
    if frame.schema != 1 {
        return Err(anyhow::anyhow!(
            "unsupported app-key link roster ack schema {}",
            frame.schema
        ));
    }
    let admin_app_key_hex =
        normalize_pubkey(&frame.admin_app_key_pubkey).context("parsing ack admin AppKey")?;
    let app_key_hex = normalize_pubkey(&frame.app_key_pubkey).context("parsing ack device")?;
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(app_key_hex.as_str()) {
        return Ok(true);
    }

    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(true);
    };
    if admin_app_key_hex != frame.admin_app_key_pubkey
        || app_key_hex != frame.app_key_pubkey
        || !app_key_link_roster_ack_matches_state(state, &frame)
    {
        return Ok(true);
    }

    let changed = acked_rosters.insert(frame.roster_fingerprint.clone());
    if changed {
        let app_keys = state.app_keys.as_ref();
        println!(
            "{}",
            json!({
                        "event": "app_key_link_roster_ack_received",
                        "topic": APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
                        "app_key_npub": pubkey_npub(&app_key_hex),
                "roster_fingerprint": frame.roster_fingerprint,
                "dck_generation": app_keys.map(|app_keys| app_keys.dck_generation),
                "created_at": app_keys.map(|app_keys| app_keys.created_at),
            })
        );
    }
    Ok(true)
}

async fn send_app_key_link_roster_ack(
    fips_blocks: Option<&FsFipsBlockSync>,
    frame: &AppKeyLinkRosterAckFrame,
) -> Result<()> {
    let Some(sync) = fips_blocks else {
        return Ok(());
    };
    sync.send_app_message(
        &pubkey_npub(&frame.admin_app_key_pubkey),
        APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
        serde_json::to_vec(frame)?,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

pub(crate) fn pubkey_npub(hex: &str) -> String {
    use nostr_sdk::nips::nip19::ToBech32;
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
}

pub(crate) fn authorization_state_label(state: &ProfileState) -> &'static str {
    iris_drive_core::device_summary::authorization_state_key(state.authorization_state)
}

pub(crate) fn app_actor_role_label(role: iris_drive_core::AppActorRole) -> &'static str {
    iris_drive_core::device_summary::device_role_key(role)
}

pub(crate) fn drive_role_label(role: DriveRole) -> &'static str {
    match role {
        DriveRole::Owner => "owner",
        DriveRole::Editor => "editor",
        DriveRole::Reader => "reader",
    }
}
