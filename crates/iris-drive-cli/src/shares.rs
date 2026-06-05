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
    }
}

fn cmd_shares_list(config_dir: &Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_app_pubkey = config
        .account
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
        .account
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
                "can_write": view.can_write,
                "can_admin": view.can_admin,
                "current_key_epoch": view.current_key_epoch,
                "has_current_key_wrap": view.has_current_key_wrap,
                "key_unavailable": view.key_unavailable,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn shares_shortcut_command_adds_unique_my_drive_shortcut() {
        let config_dir = tempdir().unwrap();
        let account = Account::create(config_dir.path(), Some("Mac".into())).unwrap();
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
            account: Some(account.state.clone()),
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
}
