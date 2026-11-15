mod content;
pub use content::{
    dispatch_share_action_with_content, dispatch_share_action_with_content_blocking,
};
pub(crate) use content::{publish_share_content, publish_share_content_to};
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::nostr_identity::NostrIdentityId;
use crate::paths::{config_path_in, key_path_in};
use crate::profile::Profile;
use crate::sharing::{
    ShareRecipient, ShareRecipientProfileEvidence, ShareRole, ShareShortcut, SharedFolder,
    SharedFolderView, create_shared_folder, default_share_shortcut_path,
    invite_shared_folder_member, invite_shared_folder_resolved_recipient,
    record_pending_share_invite, repair_shared_folder_key_epoch_wraps,
    resolve_share_recipient_from_evidence, revoke_shared_folder_member,
    set_shared_folder_member_role, shared_folder_from_invite_for_profile,
    shared_folder_missing_key_wrap_pubkeys, shared_folder_views,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareAction {
    CreateShare {
        source_path: String,
        #[serde(default)]
        display_name: Option<String>,
    },
    DeleteShare {
        share_id: NostrIdentityId,
    },
    InviteShareMember {
        share_id: NostrIdentityId,
        profile_id: NostrIdentityId,
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
        share_id: NostrIdentityId,
        evidence_json: String,
        role: ShareRole,
        #[serde(default)]
        display_name: Option<String>,
    },
    RecordPendingShareInvite {
        share_id: NostrIdentityId,
        representative_npub_hint: String,
        role: ShareRole,
        #[serde(default)]
        display_name: Option<String>,
    },
    AcceptShareInvite {
        invite: String,
    },
    RevokeShareMember {
        share_id: NostrIdentityId,
        profile_id: NostrIdentityId,
        #[serde(default)]
        reason: Option<String>,
    },
    SetShareMemberRole {
        share_id: NostrIdentityId,
        profile_id: NostrIdentityId,
        role: ShareRole,
    },
    AddShareShortcut {
        share_id: NostrIdentityId,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        target_path: Option<String>,
    },
    RepairShareWraps {
        share_id: NostrIdentityId,
    },
    /// Take the folder out of My Drive on this device. Membership stays; it
    /// can be added back with `AddShareShortcut`.
    RemoveShareShortcuts {
        share_id: NostrIdentityId,
    },
    /// The link that lets anyone who opens it ask to join with `role`.
    CreateShareJoinLink {
        share_id: NostrIdentityId,
        role: ShareRole,
    },
    /// Stop the folder's links from letting anyone else in.
    DisableShareJoinLinks {
        share_id: NostrIdentityId,
    },
    /// Ask to join a folder from its link.
    JoinShareLink {
        link: String,
    },
    /// Stop waiting to be let into a folder this device asked to join.
    CancelShareJoin {
        share_id: NostrIdentityId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShareActionResult {
    pub shares: Vec<SharedFolderView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ShareRole>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_join_link: Option<String>,
    /// Folders this device asked to join and is waiting to be let into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_joins: Vec<PendingShareJoinView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingShareJoinView {
    pub share_id: NostrIdentityId,
    pub display_name: String,
    pub role: ShareRole,
    pub requested_at: i64,
}

fn pending_join_views(config_dir: &Path, config: &AppConfig) -> Vec<PendingShareJoinView> {
    crate::share_join::ShareJoinStore::load(config_dir)
        .map(|store| {
            store
                .pending
                .iter()
                .filter(|pending| config.shared_folder(pending.link.share_id).is_none())
                .map(|pending| PendingShareJoinView {
                    share_id: pending.link.share_id,
                    display_name: pending.link.display_name(),
                    role: pending.link.role,
                    requested_at: pending.requested_at,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ShareActionMetadata {
    share_id: Option<NostrIdentityId>,
    profile_id: Option<NostrIdentityId>,
    role: Option<ShareRole>,
    epoch: Option<u64>,
    last_share_invite: Option<String>,
    shortcut: Option<ShareShortcut>,
    repaired_key_wrap_count: Option<usize>,
    remaining_missing_key_wrap_count: Option<usize>,
    revoked_app_pubkeys: Vec<String>,
    repaired_key_wrap_pubkeys: Vec<String>,
    remaining_missing_key_wrap_pubkeys: Vec<String>,
    share_join_link: Option<String>,
}

impl ShareAction {
    /// A join link pasted where an invite goes is joined like any other link.
    #[must_use]
    pub fn normalized(self) -> Self {
        match self {
            Self::AcceptShareInvite { invite }
                if crate::share_join::ShareJoinLink::is_link(&invite) =>
            {
                Self::JoinShareLink { link: invite }
            }
            action => action,
        }
    }
}

pub fn dispatch_share_action(
    config_dir: &Path,
    action: ShareAction,
    now_seconds: i64,
) -> Result<ShareActionResult> {
    let action = action.normalized();
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
        role: metadata.role,
        epoch: metadata.epoch,
        last_share_invite: metadata.last_share_invite,
        shortcut: metadata.shortcut,
        repaired_key_wrap_count: metadata.repaired_key_wrap_count,
        remaining_missing_key_wrap_count: metadata.remaining_missing_key_wrap_count,
        revoked_app_pubkeys: metadata.revoked_app_pubkeys,
        repaired_key_wrap_pubkeys: metadata.repaired_key_wrap_pubkeys,
        remaining_missing_key_wrap_pubkeys: metadata.remaining_missing_key_wrap_pubkeys,
        share_join_link: metadata.share_join_link,
        pending_joins: pending_join_views(config_dir, &config),
    })
}

pub fn share_action_state(config_dir: &Path) -> Result<ShareActionResult> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .context("profile is required before reading share state")?
        .app_key_pubkey
        .clone();
    let shares = shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &current_app_pubkey,
    );
    Ok(ShareActionResult {
        shares,
        share_id: None,
        profile_id: None,
        role: None,
        epoch: None,
        last_share_invite: None,
        shortcut: None,
        repaired_key_wrap_count: None,
        remaining_missing_key_wrap_count: None,
        revoked_app_pubkeys: Vec::new(),
        repaired_key_wrap_pubkeys: Vec::new(),
        remaining_missing_key_wrap_pubkeys: Vec::new(),
        share_join_link: None,
        pending_joins: pending_join_views(config_dir, &config),
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
        ShareAction::DeleteShare { share_id } => delete_share(config, share_id),
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
        ShareAction::RecordPendingShareInvite {
            share_id,
            representative_npub_hint,
            role,
            display_name,
        } => record_pending_invite(
            config_dir,
            config,
            share_id,
            &representative_npub_hint,
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
        } => revoke_share_member_action(
            config_dir,
            config,
            share_id,
            profile_id,
            reason,
            now_seconds,
        ),
        ShareAction::SetShareMemberRole {
            share_id,
            profile_id,
            role,
        } => set_share_member_role(config_dir, config, share_id, profile_id, role, now_seconds),
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
        ShareAction::RemoveShareShortcuts { share_id } => {
            anyhow::ensure!(config.shared_folder(share_id).is_some(), "Share missing");
            config.remove_share_shortcuts_for_share(share_id);
            Ok(ShareActionMetadata {
                share_id: Some(share_id),
                ..ShareActionMetadata::default()
            })
        }
        ShareAction::CreateShareJoinLink { share_id, role } => {
            create_share_join_link(config_dir, config, share_id, role)
        }
        ShareAction::DisableShareJoinLinks { share_id } => {
            let mut store = crate::share_join::ShareJoinStore::load(config_dir)?;
            if store.disable_links(share_id) {
                store.save(config_dir)?;
            }
            Ok(ShareActionMetadata {
                share_id: Some(share_id),
                ..ShareActionMetadata::default()
            })
        }
        ShareAction::CancelShareJoin { share_id } => {
            let mut store = crate::share_join::ShareJoinStore::load(config_dir)?;
            if store.finish_pending(share_id) {
                store.save(config_dir)?;
            }
            Ok(ShareActionMetadata {
                share_id: Some(share_id),
                ..ShareActionMetadata::default()
            })
        }
        ShareAction::JoinShareLink { link } => {
            let link = crate::share_join::ShareJoinLink::parse(&link)?;
            let share_id = link.share_id;
            crate::share_join::record_share_join(config_dir, config, link, now_seconds)?;
            Ok(ShareActionMetadata {
                share_id: Some(share_id),
                ..ShareActionMetadata::default()
            })
        }
    }
}

fn create_share_join_link(
    config_dir: &Path,
    config: &AppConfig,
    share_id: NostrIdentityId,
    role: ShareRole,
) -> Result<ShareActionMetadata> {
    let admin = config
        .profile
        .as_ref()
        .context("profile is required before sharing")?
        .app_key_pubkey
        .clone();
    let folder = config.shared_folder(share_id).context("Share missing")?;
    anyhow::ensure!(
        crate::shared_folder_app_key_can_admin(folder, &admin),
        "Only people who manage this folder can make links to it"
    );
    let mut store = crate::share_join::ShareJoinStore::load(config_dir)?;
    let (link, created) = store.link_for(folder, &admin, role)?;
    if created {
        store.save(config_dir)?;
    }
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        role: Some(role),
        share_join_link: Some(link.url()),
        ..ShareActionMetadata::default()
    })
}

fn revoke_share_member_action(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    profile_id: NostrIdentityId,
    reason: Option<String>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
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

fn create_share(
    config_dir: &Path,
    config: &mut AppConfig,
    source_path: &str,
    display_name: Option<&str>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    // Sharing a folder this profile already shares opens that share again.
    let source = crate::provider::normalize_provider_path(source_path)?;
    let shared_here = |folder: &&SharedFolder| {
        folder.owner_profile_id == account.state.profile_id && folder.source_path == source
    };
    if let Some(existing) = config.shared_folders.iter().find(shared_here) {
        return Ok(ShareActionMetadata {
            share_id: Some(existing.share_id),
            ..ShareActionMetadata::default()
        });
    }
    anyhow::ensure!(
        crate::share_folders::can_start_share(config, &source),
        "This folder is inside or contains a shared folder"
    );
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
    // The folder stays where its owner keeps it; sharing adds no second copy.
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder);
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        ..ShareActionMetadata::default()
    })
}

fn delete_share(config: &mut AppConfig, share_id: NostrIdentityId) -> Result<ShareActionMetadata> {
    config
        .remove_shared_folder(share_id)
        .with_context(|| format!("share not found: {share_id}"))?;
    config.remove_share_shortcuts_for_share(share_id);
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        ..ShareActionMetadata::default()
    })
}

fn invite_share_member(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    recipient: ShareRecipient,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = shared_folder_mut(config, share_id)?;
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
    share_id: NostrIdentityId,
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
    let folder = shared_folder_mut(config, share_id)?;
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

fn record_pending_invite(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    representative_npub_hint: &str,
    role: ShareRole,
    display_name: Option<String>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = shared_folder_mut(config, share_id)?;
    let pending = record_pending_share_invite(
        folder,
        account.app_key.keys(),
        representative_npub_hint,
        role,
        display_name,
        now_seconds,
    )?;
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        role: Some(pending.role),
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
    let mut folder = shared_folder_from_invite_for_profile(invite, local_profile_id)?;
    // Invite metadata is not a signed writer announcement. Only verified relay
    // or direct-root events may supply file roots.
    folder.app_key_roots.clear();
    let share_id = folder.share_id;
    if let Some(existing) = config.shared_folder(share_id) {
        anyhow::ensure!(
            existing.owner_profile_id == folder.owner_profile_id,
            "Share owner changed"
        );
        folder.app_key_roots = existing.app_key_roots.clone();
        if existing.access.updated_at >= folder.access.updated_at {
            folder = existing.clone();
        }
    }
    let display_name = folder.display_name.clone();
    config.upsert_shared_folder(folder);
    let shortcut = ensure_default_share_shortcut(config, share_id, &display_name)?;
    Ok(ShareActionMetadata {
        share_id: Some(share_id),
        shortcut,
        ..ShareActionMetadata::default()
    })
}

fn revoke_share_member(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    profile_id: NostrIdentityId,
    reason: Option<&str>,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = shared_folder_mut(config, share_id)?;
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

fn set_share_member_role(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    profile_id: NostrIdentityId,
    role: ShareRole,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = shared_folder_mut(config, share_id)?;
    let outcome = set_shared_folder_member_role(
        folder,
        account.app_key.keys(),
        profile_id,
        role,
        now_seconds,
    )?;
    Ok(ShareActionMetadata {
        share_id: Some(outcome.share_id),
        profile_id: Some(outcome.profile_id),
        role: Some(outcome.role),
        ..ShareActionMetadata::default()
    })
}

fn add_share_shortcut(
    config: &mut AppConfig,
    share_id: NostrIdentityId,
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

pub(crate) fn ensure_default_share_shortcut(
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    display_name: &str,
) -> Result<Option<ShareShortcut>> {
    if config
        .share_shortcuts
        .iter()
        .any(|shortcut| shortcut.share_id == share_id)
    {
        return Ok(None);
    }
    let shortcut_path = default_share_shortcut_path(&config.share_shortcuts, display_name, "")?;
    let shortcut = ShareShortcut::new(share_id, &shortcut_path, "")?;
    config.upsert_share_shortcut(shortcut.clone());
    Ok(Some(shortcut))
}

fn repair_share_wraps(
    config_dir: &Path,
    config: &mut AppConfig,
    share_id: NostrIdentityId,
    now_seconds: i64,
) -> Result<ShareActionMetadata> {
    let account = load_profile(config_dir, config)?;
    let folder = shared_folder_mut(config, share_id)?;
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

fn shared_folder_mut(
    config: &mut AppConfig,
    share_id: NostrIdentityId,
) -> Result<&mut SharedFolder> {
    config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .with_context(|| format!("share not found: {share_id}"))
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
mod tests;
