use anyhow::{Context, Result};
use iris_drive_core::{
    Profile, config::AppConfig, config_lock::ConfigMutationLock, paths::config_path_in,
};
use nostr_sdk::PublicKey;

use super::app_key_link_urls::decode_app_key_approval_bootstrap;
use super::device_approval_publish::publish_device_approval_with_error;
use super::pubkey_npub;

pub(crate) fn cmd_approve(
    config_dir: &std::path::Path,
    device: &str,
    label: Option<String>,
) -> Result<()> {
    let PersistedDeviceApproval {
        config,
        state,
        pending,
        approved_app_key_npub,
        device_count,
    } = persist_device_approval(config_dir, device, label)?;
    let (publish_report, approval_publish_error) =
        publish_device_approval_with_error(config_dir, &config, &state, &pending);
    println!(
        "{}",
        publish_report.into_output_json(
            &approved_app_key_npub,
            device_count,
            approval_publish_error.as_deref(),
        )
    );
    Ok(())
}

pub(super) struct PersistedDeviceApproval {
    config: AppConfig,
    pub(super) state: iris_drive_core::ProfileState,
    pending: iris_drive_core::profile::PendingDeviceApprovalReceipt,
    approved_app_key_npub: String,
    device_count: usize,
}

pub(super) fn persist_device_approval(
    config_dir: &std::path::Path,
    device: &str,
    label: Option<String>,
) -> Result<PersistedDeviceApproval> {
    let config_lock = ConfigMutationLock::acquire_blocking(config_dir)
        .context("locking config for AppKey approval")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let bootstrap = decode_app_key_approval_bootstrap(&config, device)?;
    let app_key_hex = PublicKey::parse(&bootstrap.device_app_key_npub)
        .context("parsing approval device AppKey")?
        .to_hex();
    let request_pubkey = PublicKey::parse(&bootstrap.request_npub)
        .context("parsing approval request key")?
        .to_hex();
    let label = label.or_else(|| bootstrap.label.clone()).or_else(|| {
        state
            .inbound_app_key_link_requests
            .iter()
            .find(|pending| pending.app_key_pubkey == app_key_hex)
            .and_then(|pending| pending.label.clone())
    });
    let approved_app_key_npub = pubkey_npub(&app_key_hex);
    let mut profile = Profile::load(state, config_dir).context("loading profile")?;
    let snap = profile
        .approve_device_bootstrap(&bootstrap, label)
        .context("approving AppKey")?;
    let device_count = snap.app_actors.len();
    let pending = profile
        .state
        .pending_device_approval_receipts
        .iter()
        .find(|pending| pending.request_pubkey == request_pubkey)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("approval did not retain its encrypted receipt"))?;
    config.profile = Some(profile.state.clone());
    config.save(config_path_in(config_dir))?;
    drop(config_lock);
    Ok(PersistedDeviceApproval {
        config,
        state: profile.state,
        pending,
        approved_app_key_npub,
        device_count,
    })
}
