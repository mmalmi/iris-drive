use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::iris_profile::IrisProfileId;
use crate::paths::{config_path_in, key_path_in};
use crate::profile::Profile;
use crate::sharing::{
    ShareRecipient, ShareRecipientProfileEvidence, ShareRole, ShareShortcut, SharedFolderView,
    create_shared_folder, default_share_shortcut_path, invite_shared_folder_member,
    invite_shared_folder_resolved_recipient, repair_shared_folder_key_epoch_wraps,
    resolve_share_recipient_from_evidence, revoke_shared_folder_member,
    shared_folder_from_invite_for_profile, shared_folder_missing_key_wrap_pubkeys,
    shared_folder_views,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareAction {
    CreateShare {
        source_path: String,
        #[serde(default)]
        display_name: Option<String>,
    },
    InviteShareMember {
        share_id: IrisProfileId,
        profile_id: IrisProfileId,
        app_key: String,
        role: ShareRole,
        #[serde(default)]
        representative_npub_hint: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        label: Option<String>,
    },
    InviteShareMemberFromEvidence {
        share_id: IrisProfileId,
        evidence_json: String,
        role: ShareRole,
        #[serde(default)]
        display_name: Option<String>,
    },
    AcceptShareInvite {
        invite: String,
    },
    RevokeShareMember {
        share_id: IrisProfileId,
        profile_id: IrisProfileId,
        #[serde(default)]
        reason: Option<String>,
    },
    AddShareShortcut {
        share_id: IrisProfileId,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        target_path: Option<String>,
    },
    RepairShareWraps {
        share_id: IrisProfileId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShareActionResult {
    pub shares: Vec<SharedFolderView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_id: Option<IrisProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<IrisProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_share_invite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<ShareShortcut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repaired_key_wrap_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_missing_key_wrap_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ShareActionMetadata {
    share_id: Option<IrisProfileId>,
    profile_id: Option<IrisProfileId>,
    last_share_invite: Option<String>,
    shortcut: Option<ShareShortcut>,
    repaired_key_wrap_count: Option<usize>,
    remaining_missing_key_wrap_count: Option<usize>,
}

pub fn dispatch_share_action(
    config_dir: &Path,
    action: ShareAction,
    now_seconds: i64,
) -> Result<ShareActionResult> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let metadata = apply_share_action(config_dir, &mut config, action, now_seconds)?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .context("profile is required before running share actions")?
        .app_key_pubkey
        .clone();
    let shares = shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &current_app_pubkey,
    );
    config.save(config_path_in(config_dir))?;
    Ok(ShareActionResult {
        shares,
        share_id: metadata.share_id,
        profile_id: metadata.profile_id,
        last_share_invite: metadata.last_share_invite,
        shortcut: metadata.shortcut,
        repaired_key_wrap_count: metadata.repaired_key_wrap_count,
        remaining_missing_key_wrap_count: metadata.remaining_missing_key_wrap_count,
    })
}

fn apply_share_action(
    config_dir: &Path,
    config: &mut AppConfig,
    action: ShareAction,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    match action {
        ShareAction::CreateShare {
            source_path,
            display_name,
        } => create_share(
            config_dir,
            config,
            &source_path,
            display_name.as_deref(),
            now_seconds,
        ),
        ShareAction::InviteShareMember {
            share_id,
            profile_id,
            app_key,
            role,
            representative_npub_hint,
            display_name,
            label,
        } => invite_share_member(
            config_dir,
            config,
            share_id,
            ShareRecipient {
                profile_id,
                app_pubkey: normalize_pubkey_hex(&app_key)?,
                role,
                label: trimmed_option(label),
                representative_npub_hint: trimmed_option(representative_npub_hint),
                display_name: trimmed_option(display_name),
            },
            now_seconds,
        ),
        ShareAction::InviteShareMemberFromEvidence {
            share_id,
            evidence_json,
            role,
            display_name,
        } => invite_share_member_from_evidence(
            config_dir,
            config,
            share_id,
            &evidence_json,
            role,
            trimmed_option(display_name),
            now_seconds,
        ),
        ShareAction::AcceptShareInvite { invite } => {
            accept_share_invite(config, &invite, now_seconds)
        }
        ShareAction::RevokeShareMember {
            share_id,
            profile_id,
            reason,
        } => {
            let reason = trimmed_option(reason);
            revoke_share_member(
                config_dir,
                config,
                share_id,
                profile_id,
                reason.as_deref(),
                now_seconds,
            )
        }
        ShareAction::AddShareShortcut {
            share_id,
            path,
            parent,
            target_path,
        } => add_share_shortcut(
            config,
            share_id,
            path.as_deref(),
            parent.as_deref(),
            target_path.as_deref(),
        ),
        ShareAction::RepairShareWraps { share_id } => {
            repair_share_wraps(config_dir, config, share_id, now_seconds)
        }
    }
}

fn create_share(
    config_dir: &Path,
    config: &mut AppConfig,
    source_path: &str,
    display_name: Option<&str>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let display_name = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| default_share_display_name(source_path), str::to_owned);
    let folder = create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        source_path,
        &display_name,
        account.state.app_key_label,
        Vec::new(),
        now_seconds,
    )?;
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder);
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        ..ShareActionMetadata::default()
    })
}

fn invite_share_member(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: IrisProfileId,
    recipient: ShareRecipient,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .with_context(|| format!("share not found: {share_id}"))?;
    let outcome =
        invite_shared_folder_member(folder, account.app_key.keys(), recipient, now_seconds)?;
    Ok(ShareActionMetadata {
        share_id: Some(outcome.share_id),
        profile_id: Some(outcome.profile_id),
        last_share_invite: Some(outcome.invite_url),
        ..ShareActionMetadata::default()
    })
}

fn invite_share_member_from_evidence(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: IrisProfileId,
    evidence_json: &str,
    role: ShareRole,
    display_name: Option<String>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let evidence: ShareRecipientProfileEvidence =
        serde_json::from_str(evidence_json).context("parsing recipient evidence")?;
    let resolved = resolve_share_recipient_from_evidence(&evidence, display_name)
        .context("resolving share recipient evidence")?;
    let account = load_profile(config_dir, config)?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .with_context(|| format!("share not found: {share_id}"))?;
    let outcome = invite_shared_folder_resolved_recipient(
        folder,
        account.app_key.keys(),
        &resolved,
        role,
        now_seconds,
    )?;
    Ok(ShareActionMetadata {
        share_id: Some(outcome.share_id),
        profile_id: Some(outcome.profile_id),
        last_share_invite: Some(outcome.invite_url),
        ..ShareActionMetadata::default()
    })
}

fn accept_share_invite(
    config: &mut AppConfig,
    invite: &str,
    _now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let local_profile_id = config
        .profile
        .as_ref()
        .context("profile is required before accepting share invites")?
        .profile_id;
    let folder = shared_folder_from_invite_for_profile(invite, local_profile_id)?;
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder);
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        ..ShareActionMetadata::default()
    })
}

fn revoke_share_member(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: IrisProfileId,
    profile_id: IrisProfileId,
    reason: Option<&str>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .with_context(|| format!("share not found: {share_id}"))?;
    let outcome = revoke_shared_folder_member(
        folder,
        account.app_key.keys(),
        profile_id,
        reason,
        now_seconds,
    )?;
    Ok(ShareActionMetadata {
        share_id: Some(outcome.share_id),
        profile_id: Some(outcome.profile_id),
        ..ShareActionMetadata::default()
    })
}

fn add_share_shortcut(
    config: &mut AppConfig,
    share_id: IrisProfileId,
    path: Option<&str>,
    parent: Option<&str>,
    target_path: Option<&str>,
) -> Result<ShareActionMetadata> {
    let folder = config
        .shared_folder(share_id)
        .with_context(|| format!("share not found: {share_id}"))?
        .clone();
    let shortcut_path = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                default_share_shortcut_path(
                    &config.share_shortcuts,
                    &folder.display_name,
                    parent.unwrap_or_default(),
                )
            },
            |value| Ok(value.to_owned()),
        )?;
    let shortcut = ShareShortcut::new(share_id, &shortcut_path, target_path.unwrap_or_default())?;
    config.upsert_share_shortcut(shortcut.clone());
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        shortcut: Some(shortcut),
        ..ShareActionMetadata::default()
    })
}

fn repair_share_wraps(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: IrisProfileId,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .with_context(|| format!("share not found: {share_id}"))?;
    let repair = repair_shared_folder_key_epoch_wraps(folder, account.app_key.keys(), now_seconds)?;
    let remaining_missing_key_wrap_count =
        shared_folder_missing_key_wrap_pubkeys(folder, repair.epoch).len();
    Ok(ShareActionMetadata {
        share_id: Some(repair.share_id),
        repaired_key_wrap_count: Some(repair.repaired_pubkeys.len()),
        remaining_missing_key_wrap_count: Some(remaining_missing_key_wrap_count),
        ..ShareActionMetadata::default()
    })
}

fn load_profile(config_dir: &Path, config: &AppConfig) -> Result<Profile> {
    if !key_path_in(config_dir).exists() {
        anyhow::bail!("iris-drive is not initialized");
    }
    let state = config
        .profile
        .clone()
        .context("profile is required before running share actions")?;
    Profile::load(state, config_dir).context("loading profile")
}

fn normalize_pubkey_hex(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        return Ok(PublicKey::from_bech32(trimmed)
            .context("parsing npub")?
            .to_hex());
    }
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    anyhow::bail!("expected npub1... or 64-char hex pubkey, got {trimmed}")
}

fn default_share_display_name(source_path: &str) -> String {
    source_path
        .trim_matches('/')
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or("Shared folder")
        .to_owned()
}

fn trimmed_option(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
