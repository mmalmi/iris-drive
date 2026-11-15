//! Share actions from native shells, routed through Rust core so every
//! platform creates, links, joins and manages folders the same way.

use std::path::Path;

use anyhow::Context;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, Profile};

use super::NativeAppRuntime;
use super::action_inputs::{optional_trimmed, parse_share_role, share_now_seconds};
use crate::NativeAppAction;
use crate::state::UiPendingShareJoin;

/// Folders this device asked to join and is still waiting to be let into.
pub(super) fn pending_share_joins(data_dir: &str, config: &AppConfig) -> Vec<UiPendingShareJoin> {
    let Ok(store) = iris_drive_core::share_join::ShareJoinStore::load(Path::new(data_dir)) else {
        return Vec::new();
    };
    store
        .pending
        .iter()
        .filter(|pending| config.shared_folder(pending.link.share_id).is_none())
        .map(|pending| UiPendingShareJoin {
            share_id: pending.link.share_id.to_string(),
            display_name: pending.link.display_name(),
            role: pending.link.role.as_str().to_owned(),
            role_label: pending.link.role.label().to_owned(),
        })
        .collect()
}

fn pending_share_join_views(
    pending: &[iris_drive_core::share_actions::PendingShareJoinView],
) -> Vec<UiPendingShareJoin> {
    pending
        .iter()
        .map(|pending| UiPendingShareJoin {
            share_id: pending.share_id.to_string(),
            display_name: pending.display_name.clone(),
            role: pending.role.as_str().to_owned(),
            role_label: pending.role.label().to_owned(),
        })
        .collect()
}

impl NativeAppRuntime {
    pub(super) fn dispatch_share_action(&mut self, action: NativeAppAction) {
        match self.try_dispatch_share_action(action) {
            Ok(result) => {
                // The latest thing to send someone: an invite, or a link to the folder.
                if let Some(invite) = result.last_share_invite.or(result.share_join_link) {
                    self.state.ui.last_share_invite = invite;
                }
                self.state.ui.pending_share_joins = pending_share_join_views(&result.pending_joins);
            }
            Err(error) => {
                self.state.error = format!("running share action: {error:#}");
            }
        }
    }

    pub(super) fn export_share_recipient_evidence(&mut self, display_name: &str) {
        self.state.ui.last_share_recipient_evidence.clear();
        match self.try_export_share_recipient_evidence(display_name) {
            Ok(evidence_json) => {
                self.state.ui.last_share_recipient_evidence = evidence_json;
            }
            Err(error) => {
                self.state.error = format!("exporting share recipient evidence: {error:#}");
            }
        }
    }

    fn try_export_share_recipient_evidence(&self, display_name: &str) -> anyhow::Result<String> {
        let config_dir = Path::new(&self.data_dir);
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .profile
            .context("profile is required before exporting recipient evidence")?;
        let profile = Profile::load(state, config_dir)?;
        let evidence = iris_drive_core::share_recipient_profile_evidence_for_app_key(
            profile.state.profile_id,
            &profile.state.profile_roster_ops,
            profile.app_key.keys(),
            optional_trimmed(display_name),
            share_now_seconds(),
        )
        .context("exporting share recipient evidence")?;
        serde_json::to_string(&evidence).context("encoding recipient evidence")
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one flat mapping from shell actions to core share actions"
    )]
    fn try_dispatch_share_action(
        &self,
        action: NativeAppAction,
    ) -> anyhow::Result<iris_drive_core::ShareActionResult> {
        let action = match action {
            NativeAppAction::CreateShare {
                source_path,
                display_name,
            } => iris_drive_core::ShareAction::CreateShare {
                source_path,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::DeleteShare { share_id } => {
                iris_drive_core::ShareAction::DeleteShare {
                    share_id: share_id.parse()?,
                }
            }
            NativeAppAction::InviteShareMember {
                share_id,
                profile_id,
                app_key,
                role,
                representative_npub_hint,
                display_name,
                label,
            } => iris_drive_core::ShareAction::InviteShareMember {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                app_key,
                role: parse_share_role(&role)?,
                representative_npub_hint: optional_trimmed(&representative_npub_hint),
                display_name: optional_trimmed(&display_name),
                label: optional_trimmed(&label),
            },
            NativeAppAction::InviteShareMemberFromEvidence {
                share_id,
                evidence_json,
                role,
                display_name,
            } => iris_drive_core::ShareAction::InviteShareMemberFromEvidence {
                share_id: share_id.parse()?,
                evidence_json,
                role: parse_share_role(&role)?,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::RecordPendingShareInvite {
                share_id,
                representative_npub_hint,
                role,
                display_name,
            } => iris_drive_core::ShareAction::RecordPendingShareInvite {
                share_id: share_id.parse()?,
                representative_npub_hint,
                role: parse_share_role(&role)?,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::AcceptShareInvite { invite } => {
                iris_drive_core::ShareAction::AcceptShareInvite { invite }
            }
            NativeAppAction::RevokeShareMember {
                share_id,
                profile_id,
                reason,
            } => iris_drive_core::ShareAction::RevokeShareMember {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                reason: optional_trimmed(&reason),
            },
            NativeAppAction::SetShareMemberRole {
                share_id,
                profile_id,
                role,
            } => iris_drive_core::ShareAction::SetShareMemberRole {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                role: parse_share_role(&role)?,
            },
            NativeAppAction::AddShareShortcut {
                share_id,
                path,
                parent,
                target_path,
            } => iris_drive_core::ShareAction::AddShareShortcut {
                share_id: share_id.parse()?,
                path: optional_trimmed(&path),
                parent: optional_trimmed(&parent),
                target_path: optional_trimmed(&target_path),
            },
            NativeAppAction::RepairShareWraps { share_id } => {
                iris_drive_core::ShareAction::RepairShareWraps {
                    share_id: share_id.parse()?,
                }
            }
            NativeAppAction::RemoveShareShortcuts { share_id } => {
                iris_drive_core::ShareAction::RemoveShareShortcuts {
                    share_id: share_id.parse()?,
                }
            }
            NativeAppAction::CreateShareJoinLink { share_id, role } => {
                iris_drive_core::ShareAction::CreateShareJoinLink {
                    share_id: share_id.parse()?,
                    role: parse_share_role(&role)?,
                }
            }
            NativeAppAction::CancelShareJoin { share_id } => {
                iris_drive_core::ShareAction::CancelShareJoin {
                    share_id: share_id.parse()?,
                }
            }
            NativeAppAction::DisableShareJoinLinks { share_id } => {
                iris_drive_core::ShareAction::DisableShareJoinLinks {
                    share_id: share_id.parse()?,
                }
            }
            _ => unreachable!("non-share action dispatched to share action handler"),
        };
        iris_drive_core::share_actions::dispatch_share_action_with_content_blocking(
            Path::new(&self.data_dir),
            action,
            share_now_seconds(),
        )
    }
}
