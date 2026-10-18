#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_shares(config_dir: &Path, command: Option<SharesCmd>) -> Result<()> {
    match command.unwrap_or(SharesCmd::List { diagnostics: false }) {
        SharesCmd::Create { source_path, name } => {
            cmd_shares_create(config_dir, &source_path, name.as_deref())
        }
        SharesCmd::Delete { share_id } => cmd_shares_delete(config_dir, &share_id),
        SharesCmd::List { diagnostics } => cmd_shares_list(config_dir, diagnostics),
        SharesCmd::Members { share_id } => cmd_shares_members(config_dir, &share_id),
        SharesCmd::RecipientEvidence { display_name } => {
            cmd_shares_recipient_evidence(config_dir, display_name.as_deref())
        }
        SharesCmd::Invite {
            share_id,
            profile,
            app_key,
            recipient_evidence,
            role,
            npub,
            display_name,
            label,
        } => cmd_shares_invite(
            config_dir,
            &share_id,
            profile.as_deref(),
            app_key.as_deref(),
            recipient_evidence.as_deref(),
            &role,
            npub,
            display_name,
            label,
        ),
        SharesCmd::Accept { invite } => cmd_shares_accept(config_dir, &invite),
        SharesCmd::Revoke {
            share_id,
            profile_id,
            reason,
            diagnostics,
        } => cmd_shares_revoke(
            config_dir,
            &share_id,
            &profile_id,
            reason.as_deref(),
            diagnostics,
        ),
        SharesCmd::Role {
            share_id,
            profile_id,
            role,
        } => cmd_shares_role(config_dir, &share_id, &profile_id, &role),
        SharesCmd::Shortcut {
            share_id,
            path,
            parent,
            target_path,
        } => cmd_shares_shortcut(
            config_dir,
            &share_id,
            path.as_deref(),
            parent.as_deref(),
            &target_path,
        ),
        SharesCmd::RepairWraps {
            share_id,
            diagnostics,
        } => cmd_shares_repair_wraps(config_dir, &share_id, diagnostics),
    }
}

fn cmd_shares_create(config_dir: &Path, source_path: &str, name: Option<&str>) -> Result<()> {
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::CreateShare {
            source_path: source_path.to_owned(),
            display_name: name.map(str::to_owned),
        },
    )
    .context("creating shared folder")?;
    let share_id = result
        .share_id
        .ok_or_else(|| anyhow::anyhow!("created share action did not return a share id"))?;
    let created = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("created share was not projected"))?;
    println!(
        "{}",
        json!({
            "share": share_view_json(created),
            "shares": share_views_json(&result.shares),
        })
    );
    Ok(())
}

fn cmd_shares_delete(config_dir: &Path, share_id: &str) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::DeleteShare { share_id },
    )
    .context("deleting shared folder")?;
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "shares": share_views_json(&result.shares),
        })
    );
    Ok(())
}

fn cmd_shares_list(config_dir: &Path, diagnostics: bool) -> Result<()> {
    let result = iris_drive_core::share_action_state(config_dir).context("reading share state")?;
    println!(
        "{}",
        json!({ "shares": share_views_json_with_diagnostics(&result.shares, diagnostics) })
    );
    Ok(())
}

fn cmd_shares_members(config_dir: &Path, share_id: &str) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let result = iris_drive_core::share_action_state(config_dir).context("reading share state")?;
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?;
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_shares_recipient_evidence(config_dir: &Path, display_name: Option<&str>) -> Result<()> {
    println!(
        "{}",
        share_recipient_evidence_json(config_dir, display_name, share_timestamp())?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_shares_invite(
    config_dir: &Path,
    share_id: &str,
    profile_id: Option<&str>,
    app_key: Option<&str>,
    recipient_evidence_path: Option<&Path>,
    role: &str,
    representative_npub_hint: Option<String>,
    display_name: Option<String>,
    label: Option<String>,
) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let role = parse_share_role(role)?;
    let action = if let Some(path) = recipient_evidence_path {
        if profile_id.is_some()
            || app_key.is_some()
            || representative_npub_hint.is_some()
            || label.is_some()
        {
            return Err(anyhow::anyhow!(
                "--recipient-evidence cannot be combined with --profile, --app-key, --npub, or --label"
            ));
        }
        iris_drive_core::ShareAction::InviteShareMemberFromEvidence {
            share_id,
            evidence_json: load_share_recipient_evidence_json(path)?,
            role,
            display_name,
        }
    } else {
        match (profile_id, app_key, representative_npub_hint) {
            (None, None, Some(representative_npub_hint)) => {
                if label.is_some() {
                    return Err(anyhow::anyhow!(
                        "--label is only valid when inviting a concrete AppKey"
                    ));
                }
                iris_drive_core::ShareAction::RecordPendingShareInvite {
                    share_id,
                    representative_npub_hint,
                    role,
                    display_name,
                }
            }
            (Some(profile_id), Some(app_key), representative_npub_hint) => {
                let profile_id = profile_id
                    .parse::<iris_drive_core::NostrIdentityId>()
                    .context("parsing recipient NostrIdentity id")?;
                iris_drive_core::ShareAction::InviteShareMember {
                    share_id,
                    profile_id,
                    app_key: app_key.to_owned(),
                    role,
                    representative_npub_hint,
                    display_name,
                    label,
                }
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "--profile and --app-key, --recipient-evidence, or --npub is required"
                ));
            }
        }
    };
    let result = dispatch_cli_share_action(config_dir, action).context("inviting share member")?;
    let share_id = result.share_id.unwrap_or(share_id);
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("invited share was not projected"))?;
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "profile_id": result.profile_id.map(|profile_id| profile_id.to_string()).unwrap_or_default(),
            "epoch": result.epoch,
            "invite": result.last_share_invite.unwrap_or_default(),
            "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
            "pending_invites": view.pending_invites.iter().map(pending_share_invite_json).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_shares_accept(config_dir: &Path, invite: &str) -> Result<()> {
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::AcceptShareInvite {
            invite: invite.to_owned(),
        },
    )
    .context("accepting share invite")?;
    let share_id = result
        .share_id
        .ok_or_else(|| anyhow::anyhow!("accepted share action did not return a share id"))?;
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("accepted share was not projected"))?;
    println!(
        "{}",
        json!({
            "share": share_view_json(view),
            "share_id": share_id.to_string(),
        })
    );
    Ok(())
}

fn cmd_shares_revoke(
    config_dir: &Path,
    share_id: &str,
    profile_id: &str,
    reason: Option<&str>,
    diagnostics: bool,
) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let profile_id = profile_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing member NostrIdentity id")?;
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::RevokeShareMember {
            share_id,
            profile_id,
            reason: reason.map(str::to_owned),
        },
    )
    .context("revoking share member")?;
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("revoked share was not projected"))?;
    println!(
        "{}",
        share_revoke_result_json(&result, view, share_id, profile_id, diagnostics)
    );
    Ok(())
}

fn cmd_shares_role(config_dir: &Path, share_id: &str, profile_id: &str, role: &str) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let profile_id = profile_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing member NostrIdentity id")?;
    let role = parse_share_role(role)?;
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::SetShareMemberRole {
            share_id,
            profile_id,
            role,
        },
    )
    .context("updating share member role")?;
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("updated share was not projected"))?;
    println!(
        "{}",
        json!({
            "share_id": result.share_id.unwrap_or(share_id).to_string(),
            "profile_id": result.profile_id.unwrap_or(profile_id).to_string(),
            "role": result.role.unwrap_or(role).as_str(),
            "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_shares_shortcut(
    config_dir: &Path,
    share_id: &str,
    path: Option<&str>,
    parent: Option<&str>,
    target_path: &str,
) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::AddShareShortcut {
            share_id,
            path: path.map(str::to_owned),
            parent: parent.map(str::to_owned),
            target_path: Some(target_path.to_owned()),
        },
    )
    .context("creating share shortcut")?;
    let shortcut = result
        .shortcut
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("share shortcut action did not return a shortcut"))?;
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "shortcut_path": shortcut.path.clone(),
            "target_path": shortcut.target_path.clone(),
            "shares": share_views_json(&result.shares),
        })
    );
    Ok(())
}

fn cmd_shares_repair_wraps(config_dir: &Path, share_id: &str, diagnostics: bool) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::NostrIdentityId>()
        .context("parsing share id")?;
    let result = dispatch_cli_share_action(
        config_dir,
        iris_drive_core::ShareAction::RepairShareWraps { share_id },
    )
    .context("repairing share key epoch wraps")?;
    println!(
        "{}",
        share_repair_result_json(&result, share_id, diagnostics)
    );
    Ok(())
}

fn share_views_json(views: &[iris_drive_core::SharedFolderView]) -> Vec<Value> {
    share_views_json_with_diagnostics(views, false)
}

fn share_views_json_with_diagnostics(
    views: &[iris_drive_core::SharedFolderView],
    diagnostics: bool,
) -> Vec<Value> {
    views
        .iter()
        .map(|view| share_view_json_with_diagnostics(view, diagnostics))
        .collect()
}

fn share_view_json(view: &iris_drive_core::SharedFolderView) -> Value {
    share_view_json_with_diagnostics(view, false)
}

fn share_view_json_with_diagnostics(
    view: &iris_drive_core::SharedFolderView,
    diagnostics: bool,
) -> Value {
    let mut value = json!({
        "share_id": view.share_id.to_string(),
        "display_name": view.display_name.clone(),
        "source_path": view.source_path.clone(),
        "shared_with_me_path": view.shared_with_me_path.clone(),
        "local_role": view.local_role.as_str(),
        "local_role_label": view.local_role.label(),
        "key_status": view.key_status.as_str(),
        "key_status_label": view.key_status.label(),
        "write_authorization": view.write_authorization.as_str(),
        "write_authorization_label": view.write_authorization.label(),
        "can_write": view.can_write,
        "can_admin": view.can_admin,
        "current_key_epoch": view.current_key_epoch,
        "has_current_key_wrap": view.has_current_key_wrap,
        "key_unavailable": view.key_unavailable,
        "repair_needed": view.repair_needed,
        "missing_key_wrap_count": view.missing_key_wrap_count,
        "participant_count": view.participant_count,
        "app_key_count": view.app_key_count,
        "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        "pending_invites": view.pending_invites.iter().map(pending_share_invite_json).collect::<Vec<_>>(),
        "shortcut_paths": view.shortcut_paths.clone(),
    });
    if diagnostics {
        insert_json_field(
            &mut value,
            "missing_key_wraps".to_string(),
            json!(app_key_npubs(&view.missing_key_wrap_pubkeys)),
        );
    }
    value
}

fn pending_share_invite_json(invite: &iris_drive_core::PendingShareInviteView) -> Value {
    json!({
        "representative_npub_hint": invite.representative_npub_hint.clone(),
        "display_name": invite.display_name.clone(),
        "role": invite.role.as_str(),
        "role_label": invite.role.label(),
        "status": invite.status.as_str(),
        "status_label": invite.status.label(),
        "created_at": invite.created_at,
    })
}

fn share_member_json(member: &iris_drive_core::SharedFolderMemberView) -> Value {
    json!({
        "profile_id": member.profile_id.to_string(),
        "display_name": member.display_name.clone(),
        "representative_npub_hint": member.representative_npub_hint.clone(),
        "role": member.role.as_str(),
        "role_label": member.role.label(),
        "status": member.status.as_str(),
        "status_label": member.status.label(),
        "app_key_count": member.app_key_count,
        "can_revoke": member.can_revoke,
        "can_change_role": member.can_change_role,
    })
}

fn share_revoke_result_json(
    result: &iris_drive_core::ShareActionResult,
    view: &iris_drive_core::SharedFolderView,
    share_id: iris_drive_core::NostrIdentityId,
    profile_id: iris_drive_core::NostrIdentityId,
    diagnostics: bool,
) -> Value {
    let mut value = json!({
        "share_id": result.share_id.unwrap_or(share_id).to_string(),
        "profile_id": result.profile_id.unwrap_or(profile_id).to_string(),
        "epoch": result.epoch,
        "revoked_app_key_count": result.revoked_app_pubkeys.len(),
        "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
    });
    if diagnostics {
        insert_json_field(
            &mut value,
            "revoked_app_keys".to_string(),
            json!(app_key_npubs(&result.revoked_app_pubkeys)),
        );
    }
    value
}

fn share_repair_result_json(
    result: &iris_drive_core::ShareActionResult,
    share_id: iris_drive_core::NostrIdentityId,
    diagnostics: bool,
) -> Value {
    let mut value = json!({
        "share_id": result.share_id.unwrap_or(share_id).to_string(),
        "epoch": result.epoch,
        "repaired_key_wrap_count": result
            .repaired_key_wrap_count
            .unwrap_or(result.repaired_key_wrap_pubkeys.len()),
        "remaining_missing_key_wrap_count": result
            .remaining_missing_key_wrap_count
            .unwrap_or(result.remaining_missing_key_wrap_pubkeys.len()),
    });
    if diagnostics {
        insert_json_field(
            &mut value,
            "repaired_key_wraps".to_string(),
            json!(app_key_npubs(&result.repaired_key_wrap_pubkeys)),
        );
        insert_json_field(
            &mut value,
            "remaining_missing_key_wraps".to_string(),
            json!(app_key_npubs(&result.remaining_missing_key_wrap_pubkeys)),
        );
    }
    value
}

fn insert_json_field(value: &mut Value, key: String, field: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key, field);
    }
}

fn app_key_npubs(pubkeys: &[String]) -> Vec<String> {
    pubkeys
        .iter()
        .map(|pubkey| iris_drive_core::app_key_summary::pubkey_npub(pubkey))
        .collect()
}

fn parse_share_role(value: &str) -> Result<iris_drive_core::ShareRole> {
    iris_drive_core::ShareRole::parse_user_input(value).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid share role {}; expected reader, editor, or admin",
            value.trim()
        )
    })
}

fn load_share_recipient_evidence_json(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("reading recipient evidence {}", path.display()))
}

fn share_recipient_evidence_json(
    config_dir: &Path,
    display_name: Option<&str>,
    accepted_at: i64,
) -> Result<String> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .context("profile is required before exporting recipient evidence")?;
    let profile = Profile::load(state, config_dir)?;
    let evidence = iris_drive_core::share_recipient_profile_evidence_for_app_key(
        profile.state.profile_id,
        &profile.state.profile_roster_ops,
        profile.app_key.keys(),
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        accepted_at,
    )
    .context("exporting share recipient evidence")?;
    serde_json::to_string(&evidence).context("encoding recipient evidence")
}

fn dispatch_cli_share_action(
    config_dir: &Path,
    action: iris_drive_core::ShareAction,
) -> Result<iris_drive_core::ShareActionResult> {
    iris_drive_core::share_actions::dispatch_share_action_with_content_blocking(
        config_dir,
        action,
        share_timestamp(),
    )
}

fn share_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
#[allow(dead_code)]
mod sharing_test_network;

#[cfg(test)]
mod tests;
