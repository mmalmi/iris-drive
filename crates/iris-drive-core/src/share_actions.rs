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
    pub epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_share_invite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<ShareShortcut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repaired_key_wrap_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_missing_key_wrap_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_app_pubkeys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repaired_key_wrap_pubkeys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_missing_key_wrap_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ShareActionMetadata {
    share_id: Option<IrisProfileId>,
    profile_id: Option<IrisProfileId>,
    epoch: Option<u64>,
    last_share_invite: Option<String>,
    shortcut: Option<ShareShortcut>,
    repaired_key_wrap_count: Option<usize>,
    remaining_missing_key_wrap_count: Option<usize>,
    revoked_app_pubkeys: Vec<String>,
    repaired_key_wrap_pubkeys: Vec<String>,
    remaining_missing_key_wrap_pubkeys: Vec<String>,
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
        epoch: metadata.epoch,
        last_share_invite: metadata.last_share_invite,
        shortcut: metadata.shortcut,
        repaired_key_wrap_count: metadata.repaired_key_wrap_count,
        remaining_missing_key_wrap_count: metadata.remaining_missing_key_wrap_count,
        revoked_app_pubkeys: metadata.revoked_app_pubkeys,
        repaired_key_wrap_pubkeys: metadata.repaired_key_wrap_pubkeys,
        remaining_missing_key_wrap_pubkeys: metadata.remaining_missing_key_wrap_pubkeys,
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
        epoch: Some(outcome.epoch),
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
        epoch: Some(outcome.epoch),
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
        epoch: Some(outcome.epoch),
        revoked_app_pubkeys: outcome.revoked_app_pubkeys,
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
    let remaining_missing_key_wrap_pubkeys =
        shared_folder_missing_key_wrap_pubkeys(folder, repair.epoch);
    Ok(ShareActionMetadata {
        share_id: Some(repair.share_id),
        epoch: Some(repair.epoch),
        repaired_key_wrap_count: Some(repair.repaired_pubkeys.len()),
        remaining_missing_key_wrap_count: Some(remaining_missing_key_wrap_pubkeys.len()),
        repaired_key_wrap_pubkeys: repair.repaired_pubkeys,
        remaining_missing_key_wrap_pubkeys,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iris_profile::IrisProfileRosterOp;
    use crate::paths::config_path_in;
    use crate::profile::Profile;
    use crate::sharing::{current_shared_folder_key, shared_folder_missing_key_wrap_pubkeys};
    use crate::{
        AppConfig, SHARE_INVITE_PREFIX, build_iris_profile_roster_op_event,
        parse_iris_profile_roster_op_event,
    };
    use tempfile::tempdir;

    fn init_config(dir: &Path, profile: &Profile) {
        let config = AppConfig {
            profile: Some(profile.state.clone()),
            ..AppConfig::default()
        };
        config.save(config_path_in(dir)).unwrap();
    }

    #[test]
    fn share_action_result_carries_cli_parity_metadata() {
        let owner_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
        init_config(owner_dir.path(), &owner);
        let recipient = nostr_sdk::Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let recipient_pubkey = recipient.public_key().to_hex();

        let created = dispatch_share_action(
            owner_dir.path(),
            ShareAction::CreateShare {
                source_path: "Projects/Alpha".to_owned(),
                display_name: Some("Alpha".to_owned()),
            },
            10,
        )
        .unwrap();
        let share_id = created.share_id.unwrap();

        let invited = dispatch_share_action(
            owner_dir.path(),
            ShareAction::InviteShareMember {
                share_id,
                profile_id: recipient_profile_id,
                app_key: recipient_pubkey.clone(),
                role: ShareRole::Editor,
                representative_npub_hint: None,
                display_name: Some("Alice".to_owned()),
                label: Some("Phone".to_owned()),
            },
            20,
        )
        .unwrap();

        assert_eq!(invited.share_id, Some(share_id));
        assert_eq!(invited.profile_id, Some(recipient_profile_id));
        assert!(
            invited
                .last_share_invite
                .unwrap()
                .starts_with(SHARE_INVITE_PREFIX)
        );
        assert_eq!(invited.epoch, Some(2));
        assert!(invited.revoked_app_pubkeys.is_empty());

        let revoked = dispatch_share_action(
            owner_dir.path(),
            ShareAction::RevokeShareMember {
                share_id,
                profile_id: recipient_profile_id,
                reason: Some("removed".to_owned()),
            },
            30,
        )
        .unwrap();

        assert_eq!(revoked.share_id, Some(share_id));
        assert_eq!(revoked.profile_id, Some(recipient_profile_id));
        assert_eq!(revoked.epoch, Some(3));
        assert_eq!(revoked.revoked_app_pubkeys, vec![recipient_pubkey]);
    }

    #[test]
    fn repair_share_action_reports_repaired_and_remaining_wrap_pubkeys() {
        let owner_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
        init_config(owner_dir.path(), &owner);
        let recipient = nostr_sdk::Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let recipient_pubkey = recipient.public_key().to_hex();

        let share_id = dispatch_share_action(
            owner_dir.path(),
            ShareAction::CreateShare {
                source_path: "Projects/Alpha".to_owned(),
                display_name: Some("Alpha".to_owned()),
            },
            10,
        )
        .unwrap()
        .share_id
        .unwrap();
        dispatch_share_action(
            owner_dir.path(),
            ShareAction::InviteShareMember {
                share_id,
                profile_id: recipient_profile_id,
                app_key: recipient_pubkey.clone(),
                role: ShareRole::Editor,
                representative_npub_hint: None,
                display_name: Some("Alice".to_owned()),
                label: Some("Phone".to_owned()),
            },
            20,
        )
        .unwrap();

        let mut config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
        let folder = config
            .shared_folders
            .iter_mut()
            .find(|folder| folder.share_id == share_id)
            .unwrap();
        let current_epoch = folder
            .projection()
            .key_epochs
            .keys()
            .next_back()
            .copied()
            .unwrap();
        for op in &mut folder.roster_ops {
            if let IrisProfileRosterOp::RotateKeyEpoch { epoch, wrapped_dck } = &mut op.content.op
                && *epoch == current_epoch
            {
                let mut incomplete_wraps = wrapped_dck.clone();
                incomplete_wraps.remove(&recipient_pubkey);
                let event = build_iris_profile_roster_op_event(
                    owner.app_key.keys(),
                    folder.share_id,
                    op.content.parents.clone(),
                    None,
                    IrisProfileRosterOp::RotateKeyEpoch {
                        epoch: *epoch,
                        wrapped_dck: incomplete_wraps,
                    },
                    op.content.created_at,
                )
                .unwrap();
                *op = parse_iris_profile_roster_op_event(&event).unwrap();
            }
        }
        assert_eq!(
            shared_folder_missing_key_wrap_pubkeys(folder, current_epoch),
            vec![recipient_pubkey.clone()]
        );
        config.save(config_path_in(owner_dir.path())).unwrap();

        let repaired = dispatch_share_action(
            owner_dir.path(),
            ShareAction::RepairShareWraps { share_id },
            30,
        )
        .unwrap();

        assert_eq!(repaired.share_id, Some(share_id));
        assert_eq!(repaired.epoch, Some(current_epoch));
        assert_eq!(repaired.repaired_key_wrap_count, Some(1));
        assert_eq!(repaired.repaired_key_wrap_pubkeys, vec![recipient_pubkey]);
        assert!(repaired.remaining_missing_key_wrap_pubkeys.is_empty());
        let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
        let folder = saved.shared_folder(share_id).unwrap();
        assert_eq!(
            current_shared_folder_key(folder, &recipient).unwrap(),
            current_shared_folder_key(folder, owner.app_key.keys()).unwrap()
        );
    }
}
