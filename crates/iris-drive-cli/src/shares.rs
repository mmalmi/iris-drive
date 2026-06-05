#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_shares(config_dir: &Path, command: Option<SharesCmd>) -> Result<()> {
    match command.unwrap_or(SharesCmd::List) {
        SharesCmd::List => cmd_shares_list(config_dir),
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

fn cmd_shares_list(config_dir: &Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?
        .device_pubkey
        .clone();
    let views = iris_drive_core::shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        &current_app_pubkey,
    );
    println!("{}", json!({ "shares": share_views_json(views) }));
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
        .device_pubkey
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
            "shares": share_views_json(views),
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
        account.device.keys(),
        share_repair_timestamp(),
    )
    .context("repairing share key epoch wraps")?;
    let remaining_missing_key_wraps = folder
        .projection()
        .active_key_recipients_missing_wraps(repair.epoch)
        .iter()
        .map(|pubkey| iris_drive_core::device_summary::pubkey_npub(pubkey))
        .collect::<Vec<_>>();
    let repaired_key_wraps = repair
        .repaired_pubkeys
        .iter()
        .map(|pubkey| iris_drive_core::device_summary::pubkey_npub(pubkey))
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

fn share_views_json(views: Vec<iris_drive_core::SharedFolderView>) -> Vec<Value> {
    views
        .into_iter()
        .map(|view| {
            json!({
                "share_id": view.share_id.to_string(),
                "display_name": view.display_name,
                "source_path": view.source_path,
                "shared_with_me_path": view.shared_with_me_path,
                "local_role": share_role_label(view.local_role),
                "key_status": view.key_status.as_str(),
                "key_status_label": view.key_status.label(),
                "can_write": view.can_write,
                "can_admin": view.can_admin,
                "current_key_epoch": view.current_key_epoch,
                "has_current_key_wrap": view.has_current_key_wrap,
                "key_unavailable": view.key_unavailable,
                "repair_needed": view.repair_needed,
                "missing_key_wraps": view
                    .missing_key_wrap_pubkeys
                    .iter()
                    .map(|pubkey| iris_drive_core::device_summary::pubkey_npub(pubkey))
                    .collect::<Vec<_>>(),
                "participant_count": view.participant_count,
                "shortcut_paths": view.shortcut_paths,
            })
        })
        .collect()
}

fn share_role_label(role: iris_drive_core::ShareRole) -> &'static str {
    match role {
        iris_drive_core::ShareRole::Admin => "admin",
        iris_drive_core::ShareRole::Editor => "editor",
        iris_drive_core::ShareRole::Reader => "reader",
    }
}

fn share_repair_timestamp() -> i64 {
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

    #[test]
    fn shares_shortcut_command_adds_unique_my_drive_shortcut() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let folder = iris_drive_core::create_shared_folder(
            account.device.keys(),
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
    fn shares_repair_wraps_command_repairs_missing_share_wraps() {
        let config_dir = tempdir().unwrap();
        let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
        let recipient_keys = nostr_sdk::Keys::generate();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        let mut folder = iris_drive_core::create_shared_folder(
            account.device.keys(),
            account.state.profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Mac".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        let add_recipient_event = iris_drive_core::build_iris_profile_roster_op_event(
            account.device.keys(),
            folder.share_id,
            iris_drive_core::iris_profile_roster_parent_ids(&folder.roster_ops),
            None,
            iris_drive_core::IrisProfileRosterOp::AddFacet {
                facet: iris_drive_core::IrisProfileFacet::app_key(
                    recipient_pubkey.clone(),
                    12,
                    Some("Phone".into()),
                    iris_drive_core::ShareRole::Editor.capabilities(),
                ),
            },
            12,
        )
        .unwrap();
        folder.roster_ops.push(
            iris_drive_core::parse_iris_profile_roster_op_event(&add_recipient_event).unwrap(),
        );
        assert_eq!(
            folder.projection().active_key_recipients_missing_wraps(1),
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
            repaired
                .projection()
                .active_key_recipients_missing_wraps(1)
                .is_empty()
        );
        assert_eq!(
            iris_drive_core::current_shared_folder_key(repaired, &recipient_keys).unwrap(),
            iris_drive_core::current_shared_folder_key(repaired, account.device.keys()).unwrap()
        );
    }
}
