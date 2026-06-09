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
        .parse::<iris_drive_core::IrisProfileId>()
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
        .parse::<iris_drive_core::IrisProfileId>()
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
        .parse::<iris_drive_core::IrisProfileId>()
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
                    .parse::<iris_drive_core::IrisProfileId>()
                    .context("parsing recipient IrisProfile id")?;
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
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing share id")?;
    let profile_id = profile_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing member IrisProfile id")?;
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
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing share id")?;
    let profile_id = profile_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing member IrisProfile id")?;
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
        .parse::<iris_drive_core::IrisProfileId>()
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
        .parse::<iris_drive_core::IrisProfileId>()
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
        value.as_object_mut().unwrap().insert(
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
    share_id: iris_drive_core::IrisProfileId,
    profile_id: iris_drive_core::IrisProfileId,
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
        value.as_object_mut().unwrap().insert(
            "revoked_app_keys".to_string(),
            json!(app_key_npubs(&result.revoked_app_pubkeys)),
        );
    }
    value
}

fn share_repair_result_json(
    result: &iris_drive_core::ShareActionResult,
    share_id: iris_drive_core::IrisProfileId,
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
        value.as_object_mut().unwrap().insert(
            "repaired_key_wraps".to_string(),
            json!(app_key_npubs(&result.repaired_key_wrap_pubkeys)),
        );
        value.as_object_mut().unwrap().insert(
            "remaining_missing_key_wraps".to_string(),
            json!(app_key_npubs(&result.remaining_missing_key_wrap_pubkeys)),
        );
    }
    value
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
    iris_drive_core::dispatch_share_action(config_dir, action, share_timestamp())
}

fn share_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn recipient_evidence_file(dir: &Path, recipient: &Profile, display_name: &str) -> PathBuf {
        let acceptance_event = iris_drive_core::build_iris_profile_facet_acceptance_event(
            recipient.app_key.keys(),
            recipient.state.profile_id,
            [iris_drive_core::IrisProfileKeyPurpose::AppKey],
            recipient
                .state
                .profile_roster_ops
                .first()
                .map(|op| op.op_id.clone()),
            20,
        )
        .unwrap();
        let evidence = iris_drive_core::ShareRecipientProfileEvidence {
            profile_id: recipient.state.profile_id,
            representative_pubkey: Some(recipient.state.app_key_pubkey.clone()),
            representative_npub: None,
            display_name: Some(display_name.to_string()),
            roster_ops: recipient.state.profile_roster_ops.clone(),
            acceptances: vec![
                iris_drive_core::parse_iris_profile_facet_acceptance_event(&acceptance_event)
                    .unwrap(),
            ],
        };
        let path = dir.join("recipient-evidence.json");
        std::fs::write(&path, serde_json::to_vec(&evidence).unwrap()).unwrap();
        path
    }

    #[test]
    fn shares_shortcut_command_adds_unique_my_drive_shortcut() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.upsert_share_shortcut(
            iris_drive_core::ShareShortcut::new(folder.share_id, "Projects/Alpha", "").unwrap(),
        );
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_shortcut(
            config_dir.path(),
            &folder.share_id.to_string(),
            None,
            Some("Projects"),
            "",
        )
        .unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let shortcuts = saved
            .share_shortcuts
            .iter()
            .map(|shortcut| shortcut.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(shortcuts, vec!["Projects/Alpha", "Projects/Alpha (2)"]);
    }

    #[test]
    fn shares_create_command_creates_entity_member_share() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_create(config_dir.path(), "Projects/Alpha", None).unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        assert_eq!(saved.shared_folders.len(), 1);
        let folder = &saved.shared_folders[0];
        assert_eq!(folder.source_path, "Projects/Alpha");
        assert_eq!(folder.display_name, "Alpha");
        assert_eq!(
            folder
                .members
                .get(&account.state.profile_id.to_string())
                .unwrap()
                .role,
            iris_drive_core::ShareRole::Admin
        );
        assert_eq!(
            folder
                .participant_profiles
                .get(&account.state.app_key_pubkey),
            Some(&account.state.profile_id)
        );
        assert_eq!(saved.share_shortcuts.len(), 1);
        let shortcut = &saved.share_shortcuts[0];
        assert_eq!(shortcut.share_id, folder.share_id);
        assert_eq!(shortcut.path, "Alpha");
        assert_eq!(shortcut.target_path, "");
    }

    #[test]
    fn shares_delete_command_removes_share_and_shortcuts() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        let share_id = folder.share_id;
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder);
        config.upsert_share_shortcut(
            iris_drive_core::ShareShortcut::new(share_id, "Projects/Alpha", "").unwrap(),
        );
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_delete(config_dir.path(), &share_id.to_string()).unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        assert!(saved.shared_folders.is_empty());
        assert!(saved.share_shortcuts.is_empty());
    }

    #[test]
    fn shares_invite_command_records_pending_npub_hint_without_authority() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        let share_id = folder.share_id;
        config.upsert_shared_folder(folder);
        config.save(config_path_in(config_dir.path())).unwrap();
        let representative_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
        let representative_npub =
            iris_drive_core::app_key_summary::pubkey_npub(&representative_pubkey);

        cmd_shares_invite(
            config_dir.path(),
            &share_id.to_string(),
            None,
            None,
            None,
            "reader",
            Some(representative_npub.clone()),
            Some("Alice".into()),
            None,
        )
        .unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let share = saved.shared_folder(share_id).unwrap();
        assert_eq!(share.pending_invites.len(), 1);
        assert_eq!(share.members.len(), 1);
        assert_eq!(share.participant_profiles.len(), 1);
        let pending = share.pending_invites.get(&representative_npub).unwrap();
        assert_eq!(pending.display_name.as_deref(), Some("Alice"));
        assert_eq!(pending.status, iris_drive_core::ShareMemberStatus::Pending);
    }

    #[test]
    fn shares_recipient_evidence_command_exports_resolvable_profile_bundle() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Phone".into())).unwrap();
        let config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.save(config_path_in(config_dir.path())).unwrap();

        let evidence_json = share_recipient_evidence_json(config_dir.path(), Some("Alice"), 30)
            .expect("recipient evidence export succeeds");
        let evidence: iris_drive_core::ShareRecipientProfileEvidence =
            serde_json::from_str(&evidence_json).unwrap();
        let resolved = iris_drive_core::resolve_share_recipient_from_evidence(&evidence, None)
            .expect("exported evidence resolves");

        assert_eq!(resolved.profile_id, account.state.profile_id);
        assert_eq!(resolved.representative_pubkey, account.state.app_key_pubkey);
        assert_eq!(resolved.display_name.as_deref(), Some("Alice"));
        assert_eq!(resolved.app_pubkeys, vec![account.state.app_key_pubkey]);
    }

    #[test]
    fn share_json_includes_core_projection_keys_and_labels() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            vec![iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: iris_drive_core::ShareRole::Reader,
                label: Some("Phone".into()),
                representative_npub_hint: Some("npub1alice".into()),
                display_name: Some("Alice".into()),
            }],
            10,
        )
        .unwrap();

        let view = iris_drive_core::shared_folder_view(&folder, &[], &account.state.app_key_pubkey);
        let share = share_view_json(&view);
        assert_eq!(share["local_role"], "admin");
        assert_eq!(share["local_role_label"], "Admin");
        assert_eq!(share["key_status"], "available");
        assert_eq!(share["key_status_label"], "Available");
        assert_eq!(share["write_authorization"], "authorized");
        assert_eq!(share["write_authorization_label"], "Authorized");
        assert!(share.get("missing_key_wraps").is_none());
        assert!(
            share_view_json_with_diagnostics(&view, true)
                .get("missing_key_wraps")
                .is_some()
        );

        let member = share["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["profile_id"] == recipient_profile_id.to_string())
            .unwrap();
        assert_eq!(member["role"], "reader");
        assert_eq!(member["role_label"], "Reader");
        assert_eq!(member["status"], "active");
        assert_eq!(member["status_label"], "Active");
    }

    #[test]
    fn shares_members_command_projects_entity_members() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            vec![iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: iris_drive_core::ShareRole::Reader,
                label: Some("Phone".into()),
                representative_npub_hint: Some("npub1alice".into()),
                display_name: Some("Alice".into()),
            }],
            10,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_members(config_dir.path(), &folder.share_id.to_string()).unwrap();

        let view = iris_drive_core::shared_folder_view(&folder, &[], &account.state.app_key_pubkey);
        assert_eq!(view.members.len(), 2);
        assert!(view.members.iter().any(|member| {
            member.profile_id == recipient_profile_id
                && member.display_name == "Alice"
                && member.role == iris_drive_core::ShareRole::Reader
        }));
    }

    #[test]
    fn shares_revoke_command_revokes_profile_member_and_rotates_epoch() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            vec![iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: iris_drive_core::ShareRole::Editor,
                label: Some("Phone".into()),
                representative_npub_hint: None,
                display_name: Some("Alice".into()),
            }],
            10,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_revoke(
            config_dir.path(),
            &folder.share_id.to_string(),
            &recipient_profile_id.to_string(),
            Some("removed"),
            false,
        )
        .unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let revoked = saved.shared_folder(folder.share_id).unwrap();
        let projection = revoked.projection();
        assert_eq!(
            revoked
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .status,
            iris_drive_core::ShareMemberStatus::Revoked
        );
        assert!(projection.key_epochs.contains_key(&2));
        assert!(
            projection
                .key_epochs
                .get(&2)
                .unwrap()
                .wrapped_dck
                .contains_key(&account.state.app_key_pubkey)
        );
        assert!(
            !projection
                .key_epochs
                .get(&2)
                .unwrap()
                .wrapped_dck
                .contains_key(&recipient_keys.public_key().to_hex())
        );
    }

    #[test]
    fn shares_revoke_json_hides_app_keys_without_diagnostics() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            vec![iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: iris_drive_core::ShareRole::Editor,
                label: Some("Phone".into()),
                representative_npub_hint: None,
                display_name: Some("Alice".into()),
            }],
            10,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.save(config_path_in(config_dir.path())).unwrap();

        let result = dispatch_cli_share_action(
            config_dir.path(),
            iris_drive_core::ShareAction::RevokeShareMember {
                share_id: folder.share_id,
                profile_id: recipient_profile_id,
                reason: Some("removed".to_string()),
            },
        )
        .unwrap();
        let view = result
            .shares
            .iter()
            .find(|view| view.share_id == folder.share_id)
            .unwrap();

        let normal =
            share_revoke_result_json(&result, view, folder.share_id, recipient_profile_id, false);
        assert_eq!(normal["revoked_app_key_count"], 1);
        assert!(normal.get("revoked_app_keys").is_none());

        let diagnostics =
            share_revoke_result_json(&result, view, folder.share_id, recipient_profile_id, true);
        assert_eq!(diagnostics["revoked_app_keys"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn shares_role_command_updates_profile_member_role() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            vec![iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: iris_drive_core::ShareRole::Reader,
                label: Some("Phone".into()),
                representative_npub_hint: None,
                display_name: Some("Alice".into()),
            }],
            10,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_role(
            config_dir.path(),
            &folder.share_id.to_string(),
            &recipient_profile_id.to_string(),
            "editor",
        )
        .unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let updated = saved.shared_folder(folder.share_id).unwrap();
        assert_eq!(
            updated
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .role,
            iris_drive_core::ShareRole::Editor
        );
        assert_eq!(
            iris_drive_core::shared_folder_app_key_write_authorization(
                updated,
                &recipient_keys.public_key().to_hex()
            ),
            iris_drive_core::ShareRootWriteAuthorization::Authorized
        );
    }

    #[test]
    fn shares_invite_and_accept_commands_import_recipient_share() {
        let owner_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
        let mut owner_config = AppConfig {
            profile: Some(owner.state.clone()),
            ..AppConfig::default()
        };
        let folder = iris_drive_core::create_shared_folder(
            owner.app_key.keys(),
            owner.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        owner_config.upsert_shared_folder(folder.clone());
        owner_config.save(config_path_in(owner_dir.path())).unwrap();
        let recipient_dir = tempdir().unwrap();
        let recipient = Profile::create(recipient_dir.path(), Some("Recipient".into())).unwrap();
        let recipient_config = AppConfig {
            profile: Some(recipient.state.clone()),
            ..AppConfig::default()
        };
        recipient_config
            .save(config_path_in(recipient_dir.path()))
            .unwrap();

        cmd_shares_invite(
            owner_dir.path(),
            &folder.share_id.to_string(),
            Some(&recipient.state.profile_id.to_string()),
            Some(&recipient.state.app_key_pubkey),
            None,
            "reader",
            Some("npub1alice".into()),
            Some("Alice".into()),
            Some("Recipient phone".into()),
        )
        .unwrap();

        let owner_saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
        let invited = owner_saved.shared_folder(folder.share_id).unwrap();
        let bundle = iris_drive_core::ShareInviteBundle {
            schema: iris_drive_core::SHARE_INVITE_SCHEMA,
            shared_folder: invited.clone(),
            recipient_profile_id: recipient.state.profile_id,
            role: iris_drive_core::ShareRole::Reader,
            representative_npub_hint: Some("npub1alice".into()),
            roster_checkpoint: None,
            created_at: 20,
        };
        let invite = iris_drive_core::encode_share_invite(&bundle).unwrap();

        cmd_shares_accept(recipient_dir.path(), &invite).unwrap();

        let recipient_saved =
            AppConfig::load_or_default(config_path_in(recipient_dir.path())).unwrap();
        let accepted = recipient_saved.shared_folder(folder.share_id).unwrap();
        assert_eq!(accepted.display_name, "Alpha");
        assert_eq!(
            accepted
                .members
                .get(&recipient.state.profile_id.to_string())
                .unwrap()
                .display_name
                .as_deref(),
            Some("Alice")
        );
        assert_eq!(
            iris_drive_core::current_shared_folder_key(accepted, recipient.app_key.keys()).unwrap(),
            iris_drive_core::current_shared_folder_key(accepted, owner.app_key.keys()).unwrap()
        );
    }

    #[test]
    fn shares_invite_command_resolves_recipient_evidence_file() {
        let owner_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
        let mut owner_config = AppConfig {
            profile: Some(owner.state.clone()),
            ..AppConfig::default()
        };
        let folder = iris_drive_core::create_shared_folder(
            owner.app_key.keys(),
            owner.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        owner_config.upsert_shared_folder(folder.clone());
        owner_config.save(config_path_in(owner_dir.path())).unwrap();
        let recipient_dir = tempdir().unwrap();
        let recipient = Profile::create(recipient_dir.path(), Some("Recipient".into())).unwrap();
        let evidence_path = recipient_evidence_file(recipient_dir.path(), &recipient, "Alice");

        cmd_shares_invite(
            owner_dir.path(),
            &folder.share_id.to_string(),
            None,
            None,
            Some(&evidence_path),
            "editor",
            None,
            None,
            None,
        )
        .unwrap();

        let owner_saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
        let invited = owner_saved.shared_folder(folder.share_id).unwrap();
        let member = invited
            .members
            .get(&recipient.state.profile_id.to_string())
            .unwrap();
        assert_eq!(member.display_name.as_deref(), Some("Alice"));
        assert_eq!(member.role, iris_drive_core::ShareRole::Editor);
        assert_eq!(
            invited
                .participant_profiles
                .get(&recipient.state.app_key_pubkey),
            Some(&recipient.state.profile_id)
        );
        assert_eq!(
            iris_drive_core::current_shared_folder_key(invited, recipient.app_key.keys()).unwrap(),
            iris_drive_core::current_shared_folder_key(invited, owner.app_key.keys()).unwrap()
        );
    }

    #[test]
    fn shares_repair_wraps_command_repairs_missing_share_wraps() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        let recipient_profile_id = iris_drive_core::IrisProfileId::new_v4();
        let mut folder = iris_drive_core::create_shared_folder(
            account.app_key.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        iris_drive_core::invite_shared_folder_member(
            &mut folder,
            account.app_key.keys(),
            iris_drive_core::ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_pubkey.clone(),
                role: iris_drive_core::ShareRole::Editor,
                label: Some("Phone".into()),
                representative_npub_hint: None,
                display_name: Some("Phone".into()),
            },
            12,
        )
        .unwrap();
        let current_epoch = folder
            .projection()
            .key_epochs
            .keys()
            .next_back()
            .copied()
            .unwrap();
        for op in &mut folder.roster_ops {
            if let iris_drive_core::IrisProfileRosterOp::RotateKeyEpoch { epoch, wrapped_dck } =
                &mut op.content.op
                && *epoch == current_epoch
            {
                let mut missing_recipient_wraps = wrapped_dck.clone();
                missing_recipient_wraps.remove(&recipient_pubkey);
                let event = iris_drive_core::build_iris_profile_roster_op_event(
                    account.app_key.keys(),
                    folder.share_id,
                    op.content.parents.clone(),
                    None,
                    iris_drive_core::IrisProfileRosterOp::RotateKeyEpoch {
                        epoch: *epoch,
                        wrapped_dck: missing_recipient_wraps,
                    },
                    op.content.created_at,
                )
                .unwrap();
                *op = iris_drive_core::parse_iris_profile_roster_op_event(&event).unwrap();
            }
        }
        assert_eq!(
            iris_drive_core::shared_folder_missing_key_wrap_pubkeys(&folder, current_epoch),
            vec![recipient_pubkey.clone()]
        );
        let mut config = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        config.upsert_shared_folder(folder.clone());
        config.save(config_path_in(config_dir.path())).unwrap();

        cmd_shares_repair_wraps(config_dir.path(), &folder.share_id.to_string(), false).unwrap();

        let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let repaired = saved.shared_folder(folder.share_id).unwrap();
        assert!(
            iris_drive_core::shared_folder_missing_key_wrap_pubkeys(repaired, current_epoch)
                .is_empty()
        );
        assert_eq!(
            iris_drive_core::current_shared_folder_key(repaired, &recipient_keys).unwrap(),
            iris_drive_core::current_shared_folder_key(repaired, account.app_key.keys()).unwrap()
        );
    }

    #[test]
    fn shares_repair_json_hides_app_key_wrap_lists_without_diagnostics() {
        let share_id = iris_drive_core::IrisProfileId::new_v4();
        let missing_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
        let repaired_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
        let result = iris_drive_core::ShareActionResult {
            shares: Vec::new(),
            share_id: Some(share_id),
            profile_id: None,
            role: None,
            epoch: Some(2),
            last_share_invite: None,
            shortcut: None,
            repaired_key_wrap_count: Some(1),
            remaining_missing_key_wrap_count: Some(1),
            revoked_app_pubkeys: Vec::new(),
            repaired_key_wrap_pubkeys: vec![repaired_pubkey],
            remaining_missing_key_wrap_pubkeys: vec![missing_pubkey],
        };

        let normal = share_repair_result_json(&result, share_id, false);
        assert_eq!(normal["repaired_key_wrap_count"], 1);
        assert_eq!(normal["remaining_missing_key_wrap_count"], 1);
        assert!(normal.get("repaired_key_wraps").is_none());
        assert!(normal.get("remaining_missing_key_wraps").is_none());

        let diagnostics = share_repair_result_json(&result, share_id, true);
        assert_eq!(
            diagnostics["repaired_key_wraps"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            diagnostics["remaining_missing_key_wraps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
