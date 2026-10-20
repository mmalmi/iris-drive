use std::path::Path;

use anyhow::Result;
use iris_drive_core::friend_backup::{self, status::friend_backup_status};
use serde_json::json;

use crate::commands::BackupFriendsCmd;

pub(crate) fn run(config_dir: &Path, command: BackupFriendsCmd) -> Result<()> {
    match command {
        BackupFriendsCmd::ExportRecovery { output } => {
            friend_backup::recovery::export_recovery(config_dir, &output)?;
            println!("{}", json!({"saved": true}));
            return Ok(());
        }
        BackupFriendsCmd::Restore {
            recovery_file,
            friend,
        } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let restored = runtime.block_on(friend_backup::recovery::restore_backup(
                config_dir,
                &recovery_file,
                &friend,
            ))?;
            println!("{}", json!({"restored_folder": restored}));
            return Ok(());
        }
        BackupFriendsCmd::Identity => {
            let npub = friend_backup::backup_npub(config_dir)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "backup_npub": npub,
                    "invite": friend_backup::encode_backup_invite(&npub)?,
                }))?
            );
            return Ok(());
        }
        BackupFriendsCmd::List => {}
        BackupFriendsCmd::Capacity { bytes } => {
            friend_backup::set_capacity(config_dir, bytes)?;
        }
        BackupFriendsCmd::Add {
            contact,
            bytes,
            label,
        } => {
            friend_backup::upsert_friend(config_dir, &contact, label, bytes, true)?;
        }
        BackupFriendsCmd::Remove { npub } => {
            friend_backup::remove_friend(config_dir, &npub)?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&friend_backup_status(config_dir)?)?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::{BackupFriendsCmd, BackupsCmd, Cli, Command};
    use clap::Parser;

    #[test]
    fn adding_a_backup_friend_requires_explicit_bytes_even_for_an_invite() {
        assert!(
            Cli::try_parse_from([
                "idrive",
                "backups",
                "friends",
                "add",
                "iris-drive://backup?npub=example"
            ])
            .is_err()
        );
        let command = Cli::try_parse_from([
            "idrive", "backups", "friends", "add", "example", "--bytes", "0",
        ])
        .unwrap();
        assert!(matches!(
            command.command,
            Command::Backups(BackupsCmd::Friends(BackupFriendsCmd::Add { bytes: 0, .. }))
        ));
    }
}
