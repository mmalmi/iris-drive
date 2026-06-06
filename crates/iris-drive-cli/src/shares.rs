#[allow(clippy::wildcard_imports)]
use super::*;
use nostr_sdk::nips::nip19::FromBech32;

pub(crate) fn cmd_shares(config_dir: &Path, command: Option<SharesCmd>) -> Result<()> {
    match command.unwrap_or(SharesCmd::List) {
        SharesCmd::Create { source_path, name } => {
            cmd_shares_create(config_dir, &source_path, name.as_deref())
        }
        SharesCmd::List => cmd_shares_list(config_dir),
        SharesCmd::Members { share_id } => cmd_shares_members(config_dir, &share_id),
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
        } => cmd_shares_revoke(config_dir, &share_id, &profile_id, reason.as_deref()),
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
        SharesCmd::RepairWraps { share_id } => cmd_shares_repair_wraps(config_dir, &share_id),
    }
}

fn cmd_shares_create(config_dir: &Path, source_path: &str, name: Option<&str>) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let account = Profile::load(state, config_dir).context("loading profile")?;
    let display_name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(
            || default_display_name_for_share_path(source_path),
            str::to_owned,
        );
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        source_path,
        &display_name,
        account.state.app_key_label.clone(),
        Vec::new(),
        share_timestamp(),
    )
    .context("creating shared folder")?;
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder);
    let views = iris_drive_core::shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &account.state.app_key_pubkey,
    );
    let created = views
        .iter()
        .find(|view| view.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("created share was not projected"))?;
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share": share_view_json(created),
            "shares": share_views_json(&views),
        })
    );
    Ok(())
}

fn cmd_shares_list(config_dir: &Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?
        .app_key_pubkey
        .clone();
    let views = iris_drive_core::shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &current_app_pubkey,
    );
    println!("{}", json!({ "shares": share_views_json(&views) }));
    Ok(())
}

fn cmd_shares_members(config_dir: &Path, share_id: &str) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing share id")?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?
        .app_key_pubkey
        .clone();
    let folder = config
        .shared_folder(share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?;
    let view =
        iris_drive_core::shared_folder_view(folder, &config.share_shortcuts, &current_app_pubkey);
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        })
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
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let account = Profile::load(state, config_dir).context("loading profile")?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?;
    let outcome = if let Some(path) = recipient_evidence_path {
        if profile_id.is_some()
            || app_key.is_some()
            || representative_npub_hint.is_some()
            || label.is_some()
        {
            return Err(anyhow::anyhow!(
                "--recipient-evidence cannot be combined with --profile, --app-key, --npub, or --label"
            ));
        }
        let evidence = load_share_recipient_evidence(path)?;
        let resolved =
            iris_drive_core::resolve_share_recipient_from_evidence(&evidence, display_name)
                .context("resolving share recipient evidence")?;
        iris_drive_core::invite_shared_folder_resolved_recipient(
            folder,
            account.app_key.keys(),
            &resolved,
            role,
            share_timestamp(),
        )
    } else {
        let profile_id = profile_id
            .ok_or_else(|| anyhow::anyhow!("--profile is required without --recipient-evidence"))?
            .parse::<iris_drive_core::IrisProfileId>()
            .context("parsing recipient IrisProfile id")?;
        let app_pubkey = normalize_pubkey_hex(app_key.ok_or_else(|| {
            anyhow::anyhow!("--app-key is required without --recipient-evidence")
        })?)
        .context("parsing recipient AppKey")?;
        iris_drive_core::invite_shared_folder_member(
            folder,
            account.app_key.keys(),
            iris_drive_core::ShareRecipient {
                profile_id,
                app_pubkey,
                role,
                label,
                representative_npub_hint,
                display_name,
            },
            share_timestamp(),
        )
    }
    .context("inviting share member")?;
    let view = iris_drive_core::shared_folder_view(
        folder,
        &config.share_shortcuts,
        &account.state.app_key_pubkey,
    );
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share_id": outcome.share_id.to_string(),
            "profile_id": outcome.profile_id.to_string(),
            "epoch": outcome.epoch,
            "invite": outcome.invite_url,
            "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_shares_accept(config_dir: &Path, invite: &str) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let folder = iris_drive_core::shared_folder_from_invite_for_profile(invite, state.profile_id)
        .context("accepting share invite")?;
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder.clone());
    let view = iris_drive_core::shared_folder_view(
        &folder,
        &config.share_shortcuts,
        &state.app_key_pubkey,
    );
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share": share_view_json(&view),
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
) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing share id")?;
    let profile_id = profile_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing member IrisProfile id")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let account = Profile::load(state, config_dir).context("loading profile")?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?;
    let outcome = iris_drive_core::revoke_shared_folder_member(
        folder,
        account.app_key.keys(),
        profile_id,
        reason,
        share_timestamp(),
    )
    .context("revoking share member")?;
    let view = iris_drive_core::shared_folder_view(
        folder,
        &config.share_shortcuts,
        &account.state.app_key_pubkey,
    );
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share_id": outcome.share_id.to_string(),
            "profile_id": outcome.profile_id.to_string(),
            "epoch": outcome.epoch,
            "revoked_app_keys": outcome.revoked_app_pubkeys
                .iter()
                .map(|pubkey| iris_drive_core::app_key_summary::pubkey_npub(pubkey))
                .collect::<Vec<_>>(),
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
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?
        .app_key_pubkey
        .clone();
    let folder = config
        .shared_folder(share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?
        .clone();
    let shortcut_path = match path {
        Some(path) if !path.trim().is_empty() => path.trim().to_owned(),
        _ => iris_drive_core::default_share_shortcut_path(
            &config.share_shortcuts,
            &folder.display_name,
            parent.unwrap_or_default(),
        )?,
    };
    let shortcut = iris_drive_core::ShareShortcut::new(share_id, &shortcut_path, target_path)
        .context("creating share shortcut")?;
    config.upsert_share_shortcut(shortcut.clone());
    let views = iris_drive_core::shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &current_app_pubkey,
    );
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share_id": share_id.to_string(),
            "shortcut_path": shortcut.path,
            "target_path": shortcut.target_path,
            "shares": share_views_json(&views),
        })
    );
    Ok(())
}

fn cmd_shares_repair_wraps(config_dir: &Path, share_id: &str) -> Result<()> {
    let share_id = share_id
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing share id")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let account = Profile::load(state, config_dir).context("loading profile")?;
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
        .ok_or_else(|| anyhow::anyhow!("share not found: {share_id}"))?;
    let repair = iris_drive_core::repair_shared_folder_key_epoch_wraps(
        folder,
        account.app_key.keys(),
        share_timestamp(),
    )
    .context("repairing share key epoch wraps")?;
    let remaining_missing_key_wraps =
        iris_drive_core::shared_folder_missing_key_wrap_pubkeys(folder, repair.epoch)
            .iter()
            .map(|pubkey| iris_drive_core::app_key_summary::pubkey_npub(pubkey))
            .collect::<Vec<_>>();
    let repaired_key_wraps = repair
        .repaired_pubkeys
        .iter()
        .map(|pubkey| iris_drive_core::app_key_summary::pubkey_npub(pubkey))
        .collect::<Vec<_>>();
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "share_id": repair.share_id.to_string(),
            "epoch": repair.epoch,
            "repaired_key_wrap_count": repair.repaired_pubkeys.len(),
            "repaired_key_wraps": repaired_key_wraps,
            "remaining_missing_key_wraps": remaining_missing_key_wraps,
        })
    );
    Ok(())
}

fn share_views_json(views: &[iris_drive_core::SharedFolderView]) -> Vec<Value> {
    views.iter().map(share_view_json).collect()
}

fn share_view_json(view: &iris_drive_core::SharedFolderView) -> Value {
    json!({
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
        "missing_key_wraps": view
            .missing_key_wrap_pubkeys
            .iter()
            .map(|pubkey| iris_drive_core::app_key_summary::pubkey_npub(pubkey))
            .collect::<Vec<_>>(),
        "participant_count": view.participant_count,
        "app_key_count": view.app_key_count,
        "members": view.members.iter().map(share_member_json).collect::<Vec<_>>(),
        "shortcut_paths": view.shortcut_paths.clone(),
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
    })
}

fn default_display_name_for_share_path(source_path: &str) -> String {
    source_path
        .trim_matches('/')
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or("Shared folder")
        .to_owned()
}

fn parse_share_role(value: &str) -> Result<iris_drive_core::ShareRole> {
    iris_drive_core::ShareRole::parse_user_input(value).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid share role {}; expected reader, editor, or admin",
            value.trim()
        )
    })
}

fn load_share_recipient_evidence(
    path: &Path,
) -> Result<iris_drive_core::ShareRecipientProfileEvidence> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading recipient evidence {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing recipient evidence {}", path.display()))
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
    Err(anyhow::anyhow!(
        "expected npub1... or 64-char hex pubkey, got {trimmed}"
    ))
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

        cmd_shares_repair_wraps(config_dir.path(), &folder.share_id.to_string()).unwrap();

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
}
