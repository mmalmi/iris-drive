#[allow(clippy::wildcard_imports)]
use super::*;

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
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) {
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
    label: Option<String>,
) -> Result<()> {
    cmd_link_with_admin_device(config_dir, owner, None, label)
}

pub(crate) fn cmd_link_with_admin_device(
    config_dir: &std::path::Path,
    owner: &str,
    admin_device: Option<&str>,
    label: Option<String>,
) -> Result<()> {
    if already_initialized(config_dir) {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let target = resolve_device_link_target_with_admin(owner, admin_device)?;
    let mut account =
        Account::link(config_dir, target.owner_hex, label).context("linking device")?;
    if let Some(admin_device_hex) = target.admin_device_hex {
        account
            .state
            .queue_outbound_device_link_request(admin_device_hex, unix_now_seconds())
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

pub(crate) fn normalize_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pk = PublicKey::from_bech32(trimmed).context("parsing npub")?;
        Ok(pk.to_hex())
    } else if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected npub1... or 64-char hex pubkey, got {trimmed}"
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceApprovalRequest {
    owner_hex: String,
    device_hex: String,
    label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceLinkTarget {
    owner_hex: String,
    admin_device_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceLinkInvite {
    owner_hex: String,
    admin_device_hex: String,
}

pub(crate) fn resolve_device_approval_input(
    input: &str,
    expected_owner_hex: &str,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(request) = decode_device_approval_request(input)? {
        if request.owner_hex != expected_owner_hex {
            return Err(anyhow::anyhow!(
                "device request belongs to a different owner"
            ));
        }
        let label = explicit_label.or(request.label);
        return Ok((request.device_hex, label));
    }

    Ok((
        normalize_pubkey(input).context("parsing device pubkey")?,
        explicit_label,
    ))
}

pub(crate) fn resolve_device_link_target_with_admin(
    input: &str,
    admin_device: Option<&str>,
) -> Result<DeviceLinkTarget> {
    if let Some(invite) = decode_device_link_invite(input)? {
        if admin_device.is_some() {
            return Err(anyhow::anyhow!(
                "--admin-device is only valid with a manual owner pubkey, not an invite URL"
            ));
        }
        return Ok(DeviceLinkTarget {
            owner_hex: invite.owner_hex,
            admin_device_hex: Some(invite.admin_device_hex),
        });
    }

    let admin_device_hex = admin_device
        .map(|admin| normalize_pubkey(admin).context("parsing admin device pubkey"))
        .transpose()?;
    Ok(DeviceLinkTarget {
        owner_hex: normalize_pubkey(input).context("parsing owner pubkey")?,
        admin_device_hex,
    })
}

pub(crate) fn device_link_request_json(state: &AccountState) -> Value {
    if state.has_owner_signing_authority
        || state.authorization_state != iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let url = encode_device_approval_request(
        &state.owner_pubkey,
        &state.device_pubkey,
        state.device_label.as_deref(),
    );

    json!({
        "url": url,
        "owner_npub": account_npub(&state.owner_pubkey),
        "device_npub": account_npub(&state.device_pubkey),
        "label": state.device_label.as_deref(),
        "admin_device_npub": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| account_npub(&request.admin_device_pubkey)),
        "requested_at": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| request.requested_at),
        "sent_over_fips": state.outbound_device_link_request.is_some(),
    })
}

pub(crate) fn device_link_invite_json(state: &AccountState) -> Value {
    if !state.has_owner_signing_authority {
        return Value::Null;
    }
    let url = encode_device_link_invite(&state.owner_pubkey, &state.device_pubkey);
    json!({
        "url": url,
        "web_url": device_link_web_url(&url),
        "owner_npub": account_npub(&state.owner_pubkey),
        "admin_device_npub": account_npub(&state.device_pubkey),
    })
}

pub(crate) fn inbound_device_link_requests_json(state: &AccountState) -> Vec<Value> {
    state
        .inbound_device_link_requests
        .iter()
        .map(|request| {
            json!({
                "url": encode_device_approval_request(
                    &state.owner_pubkey,
                    &request.device_pubkey,
                    request.label.as_deref(),
                ),
                "owner_npub": account_npub(&state.owner_pubkey),
                "device_npub": account_npub(&request.device_pubkey),
                "label": request.label.as_deref(),
                "requested_at": request.requested_at,
            })
        })
        .collect()
}

pub(crate) fn encode_device_approval_request(
    owner_hex: &str,
    device_hex: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "iris-drive://device-link?owner={}&device={}",
        account_npub(owner_hex),
        account_npub(device_hex)
    );
    if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
        url.push_str("&label=");
        url.push_str(&percent_encode_component(label));
    }
    url
}

pub(crate) fn encode_device_link_invite(owner_hex: &str, admin_device_hex: &str) -> String {
    format!(
        "iris-drive://link-device?owner={}&admin={}",
        account_npub(owner_hex),
        account_npub(admin_device_hex)
    )
}

pub(crate) fn device_link_web_url(invite_url: &str) -> String {
    invite_url.replacen(
        "iris-drive://link-device",
        "https://drive.iris.to/link-device",
        1,
    )
}

pub(crate) fn decode_device_link_invite(input: &str) -> Result<Option<DeviceLinkInvite>> {
    let trimmed = input.trim();
    let Some(query) = device_link_invite_query(trimmed) else {
        return Ok(None);
    };

    let mut owner = None;
    let mut admin = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "owner" if !value.trim().is_empty() => owner = Some(value),
            "admin" | "admin_device" if !value.trim().is_empty() => admin = Some(value),
            _ => {}
        }
    }

    let owner = owner.ok_or_else(|| anyhow::anyhow!("device link invite is missing owner"))?;
    let admin = admin.ok_or_else(|| anyhow::anyhow!("device link invite is missing admin"))?;

    Ok(Some(DeviceLinkInvite {
        owner_hex: normalize_pubkey(&owner).context("parsing invite owner")?,
        admin_device_hex: normalize_pubkey(&admin).context("parsing invite admin device")?,
    }))
}

pub(crate) fn decode_device_approval_request(input: &str) -> Result<Option<DeviceApprovalRequest>> {
    let trimmed = input.trim();
    let Some(query) = device_approval_query(trimmed) else {
        return Ok(None);
    };

    let mut owner = None;
    let mut device = None;
    let mut label = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "owner" if !value.trim().is_empty() => owner = Some(value),
            "device" if !value.trim().is_empty() => device = Some(value),
            "label" if !value.trim().is_empty() => label = Some(value),
            _ => {}
        }
    }

    let owner = owner.ok_or_else(|| anyhow::anyhow!("device request is missing owner"))?;
    let device = device.ok_or_else(|| anyhow::anyhow!("device request is missing device"))?;

    Ok(Some(DeviceApprovalRequest {
        owner_hex: normalize_pubkey(&owner).context("parsing request owner")?,
        device_hex: normalize_pubkey(&device).context("parsing request device")?,
        label,
    }))
}

pub(crate) fn device_link_invite_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix("iris-drive://link-device") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("https://drive.iris.to/link-device") {
        return rest.strip_prefix('?');
    }
    None
}

pub(crate) fn device_approval_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix("iris-drive://device-link") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("https://drive.iris.to/device-link") {
        return rest.strip_prefix('?');
    }
    None
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) const DEVICE_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/device-link/v1/request";
pub(crate) const DEVICE_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/device-link/v1/roster";
pub(crate) const DEVICE_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/device-link/v1/roster-ack";
pub(crate) const DEVICE_LINK_REQUEST_RETRY_SECS: u64 = 10;
pub(crate) const DEVICE_LINK_ROSTER_RETRY_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceLinkRequestFrame {
    schema: u32,
    owner_pubkey: String,
    device_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    requested_at: u64,
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceLinkRosterFrame {
    schema: u32,
    owner_pubkey: String,
    admin_device_pubkey: String,
    app_keys: iris_drive_core::AppKeysSnapshot,
    sent_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceLinkRosterAckFrame {
    schema: u32,
    owner_pubkey: String,
    admin_device_pubkey: String,
    device_pubkey: String,
    app_keys_created_at: i64,
    dck_generation: u64,
    acknowledged_at: u64,
}

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
    if state.has_owner_signing_authority
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
    let frame = DeviceLinkRequestFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        device_pubkey: state.device_pubkey.clone(),
        label: state.device_label.clone(),
        requested_at: pending.requested_at,
        url: encode_device_approval_request(
            &state.owner_pubkey,
            &state.device_pubkey,
            state.device_label.as_deref(),
        ),
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
    if !state.has_owner_signing_authority {
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
    let frame = DeviceLinkRosterFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        admin_device_pubkey: state.device_pubkey.clone(),
        app_keys: app_keys.clone(),
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
        "sent_bytes": bytes.len(),
    })))
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
    if state.has_owner_signing_authority || state.owner_pubkey != owner_hex {
        return Ok(true);
    }
    if sender_hex.as_deref() != Some(admin_device_hex.as_str()) {
        return Ok(true);
    }
    if frame.app_keys.owner_pubkey != state.owner_pubkey
        || !frame.app_keys.contains(&state.device_pubkey)
        || !frame.app_keys.contains(&admin_device_hex)
    {
        return Ok(true);
    }

    let should_apply = state
        .outbound_device_link_request
        .as_ref()
        .is_some_and(|pending| pending.admin_device_pubkey == admin_device_hex);
    let already_current = state.app_keys.as_ref() == Some(&frame.app_keys);
    if !should_apply && !already_current {
        return Ok(true);
    }

    let decision = if should_apply {
        state.apply_app_keys(frame.app_keys)
    } else {
        iris_drive_core::ApplyDecision::Rejected
    };
    let accepted = should_apply && decision != iris_drive_core::ApplyDecision::Rejected;
    let ack_data = if accepted || already_current {
        Some((
            state.device_pubkey.clone(),
            state
                .app_keys
                .as_ref()
                .expect("accepted/current app keys")
                .clone(),
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
    if let Some((device_pubkey, app_keys)) = ack_data {
        send_device_link_roster_ack(
            fips_blocks,
            &admin_device_hex,
            &owner_hex,
            &device_pubkey,
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
    if !state.has_owner_signing_authority
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
