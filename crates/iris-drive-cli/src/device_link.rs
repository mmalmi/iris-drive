#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_app_keys(config_dir: &std::path::Path, command: AppKeysCmd) -> Result<()> {
    match command {
        AppKeysCmd::Invite => cmd_app_keys_invite(config_dir),
        AppKeysCmd::ResetInvite => cmd_app_keys_reset_invite(config_dir),
        AppKeysCmd::Request {
            invite_or_profile,
            admin_device,
            label,
        } => cmd_link_with_admin_device(
            config_dir,
            &invite_or_profile,
            admin_device.as_deref(),
            false,
            label,
        ),
        AppKeysCmd::Requests => cmd_app_keys_requests(config_dir),
        AppKeysCmd::Approve { request, label } => cmd_approve(config_dir, &request, label),
        AppKeysCmd::Reject { request } => cmd_reject(config_dir, &request),
        AppKeysCmd::List => cmd_roster(config_dir),
        AppKeysCmd::RepairWraps => cmd_repair_key_wraps(config_dir),
        AppKeysCmd::Revoke { app_key } => cmd_revoke(config_dir, &app_key),
        AppKeysCmd::AppointAdmin { app_key } => cmd_appoint_admin(config_dir, &app_key),
        AppKeysCmd::DemoteAdmin { app_key } => cmd_demote_admin(config_dir, &app_key),
    }
}

pub(crate) fn cmd_app_keys_invite(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let invite = device_link_invite_json(state);
    if invite.is_null() {
        return Err(anyhow::anyhow!(
            "AppKey link invites require an admin AppKey"
        ));
    }
    println!("{invite}");
    Ok(())
}

pub(crate) fn cmd_app_keys_reset_invite(config_dir: &std::path::Path) -> Result<()> {
    let config_path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&config_path)?;
    let (invite, inbound_requests) = {
        let state = config
            .profile
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        if !state.can_manage_devices() {
            return Err(anyhow::anyhow!(
                "AppKey link invites require an admin AppKey"
            ));
        }
        state.reset_device_link_secret();
        (
            device_link_invite_json(state),
            inbound_device_link_requests_json(state),
        )
    };
    config.save(&config_path)?;
    println!(
        "{}",
        json!({
            "reset": true,
            "device_link_invite": invite,
            "inbound_device_link_requests": inbound_requests,
        })
    );
    Ok(())
}

pub(crate) fn cmd_app_keys_requests(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    println!(
        "{}",
        json!({
            "outbound": device_link_request_json(state),
            "inbound": inbound_device_link_requests_json(state),
        })
    );
    Ok(())
}
