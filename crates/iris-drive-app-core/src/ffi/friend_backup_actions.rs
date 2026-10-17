use std::path::Path;

use iris_drive_core::friend_backup::{self, status::friend_backup_status};

use super::{NativeAppRuntime, label_option};
use crate::state::{UiBackupFriend, UiFriendBackups};

impl NativeAppRuntime {
    pub(super) fn set_friend_backup_capacity(&mut self, capacity_bytes: u64) {
        if let Err(error) = friend_backup::set_capacity(Path::new(&self.data_dir), capacity_bytes) {
            self.state.error = format!("setting space shared with friends: {error:#}");
        }
    }

    pub(super) fn add_backup_friend(&mut self, contact: &str, label: &str, quota_bytes: u64) {
        if let Err(error) = friend_backup::upsert_friend(
            Path::new(&self.data_dir),
            contact,
            label_option(label),
            quota_bytes,
            true,
        ) {
            self.state.error = format!("adding backup friend: {error:#}");
        }
    }

    pub(super) fn remove_backup_friend(&mut self, npub: &str) {
        if let Err(error) = friend_backup::remove_friend(Path::new(&self.data_dir), npub) {
            self.state.error = format!("removing backup friend: {error:#}");
        }
    }

    pub(super) fn export_friend_backup_recovery(&mut self, path: &str) {
        if let Err(error) =
            friend_backup::recovery::export_recovery(Path::new(&self.data_dir), Path::new(path))
        {
            self.state.error = format!("saving backup recovery file: {error:#}");
        }
    }
}

pub(super) fn friend_backup_ui(config_dir: &Path) -> UiFriendBackups {
    match friend_backup_status(config_dir) {
        Ok(status) => UiFriendBackups {
            backup_npub: status.backup_npub,
            invite: status.invite,
            capacity_bytes: status.capacity_bytes,
            used_bytes: status.used_bytes,
            friends: status
                .friends
                .into_iter()
                .map(|friend| UiBackupFriend {
                    npub: friend.npub,
                    label: friend.label.unwrap_or_default(),
                    quota_bytes: friend.quota_bytes,
                    used_bytes: friend.used_bytes,
                    state: friend.state,
                    state_label: friend.state_label,
                    detail: friend.detail,
                })
                .collect(),
            error: String::new(),
        },
        Err(error) => UiFriendBackups {
            error: format!("loading friend backups: {error:#}"),
            ..UiFriendBackups::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::{FfiApp, NativeAppAction};
    use iris_drive_core::friend_backup::{backup_npub, encode_backup_invite};
    use iris_drive_core::paths::config_path_in;

    fn app(dir: &std::path::Path) -> std::sync::Arc<FfiApp> {
        let app = FfiApp::new(dir.display().to_string(), "test".to_owned());
        let state = app.dispatch(NativeAppAction::CreateProfile {
            app_key_label: "Friend backup test".to_owned(),
        });
        assert!(state.error.is_empty(), "{}", state.error);
        app
    }

    #[test]
    fn friend_backup_actions_keep_contacts_private_and_require_local_capacity() {
        let local_dir = tempfile::tempdir().unwrap();
        let remote_dir = tempfile::tempdir().unwrap();
        let local = app(local_dir.path());
        let _remote = app(remote_dir.path());
        let remote_npub = backup_npub(remote_dir.path()).unwrap();
        let contact = encode_backup_invite(&remote_npub).unwrap();
        let main_config = std::fs::read(config_path_in(local_dir.path())).unwrap();

        let added = local.dispatch(NativeAppAction::AddBackupFriend {
            contact: contact.clone(),
            label: "Friend".to_owned(),
            quota_bytes: 0,
        });
        assert!(added.error.is_empty(), "{}", added.error);
        assert_eq!(added.ui.friend_backups.capacity_bytes, 0);
        assert_eq!(added.ui.friend_backups.friends[0].quota_bytes, 0);
        assert_eq!(added.ui.friend_backups.friends[0].npub, remote_npub);
        assert_ne!(
            added.ui.friend_backups.backup_npub,
            added.ui.profile.as_ref().unwrap().current_app_key_npub
        );

        let rejected = local.dispatch(NativeAppAction::AddBackupFriend {
            contact: contact.clone(),
            label: "Friend".to_owned(),
            quota_bytes: 1024,
        });
        assert_ne!(rejected.error.len(), 0);
        assert_eq!(rejected.ui.friend_backups.friends[0].quota_bytes, 0);

        let capacity = local.dispatch(NativeAppAction::SetFriendBackupCapacity {
            capacity_bytes: 2048,
        });
        assert!(capacity.error.is_empty(), "{}", capacity.error);
        let offered = local.dispatch(NativeAppAction::AddBackupFriend {
            contact,
            label: " Archive buddy ".to_owned(),
            quota_bytes: 1024,
        });
        assert!(offered.error.is_empty(), "{}", offered.error);
        assert_eq!(offered.ui.friend_backups.friends.len(), 1);
        assert_eq!(offered.ui.friend_backups.friends[0].label, "Archive buddy");
        assert_eq!(offered.ui.friend_backups.friends[0].quota_bytes, 1024);
        assert_eq!(
            std::fs::read(config_path_in(local_dir.path())).unwrap(),
            main_config,
            "friend contacts must not modify the profile, relays, or public identity config"
        );

        let removed = local.dispatch(NativeAppAction::RemoveBackupFriend { npub: remote_npub });
        assert!(removed.error.is_empty(), "{}", removed.error);
        assert_eq!(removed.ui.friend_backups.friends.len(), 0);
    }

    #[test]
    fn invalid_friend_invite_does_not_add_contact_or_grant_storage() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let result = app.dispatch(NativeAppAction::AddBackupFriend {
            contact: "iris-drive://backup?npub=not-a-key&bytes=999999".to_owned(),
            label: String::new(),
            quota_bytes: 0,
        });
        assert_ne!(result.error.len(), 0);
        assert_eq!(result.ui.friend_backups.friends.len(), 0);
        assert_eq!(result.ui.friend_backups.capacity_bytes, 0);
    }

    #[test]
    fn friend_backup_recovery_export_writes_only_the_chosen_file() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let output = dir.path().join("backup-recovery.json");
        assert!(!output.exists());
        let state = app.dispatch(NativeAppAction::ExportFriendBackupRecovery {
            path: output.display().to_string(),
        });
        assert!(state.error.is_empty(), "{}", state.error);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        let backup_secret = saved["backup_secret"].as_str().unwrap();
        assert_ne!(backup_secret.len(), 0);
        assert!(
            !serde_json::to_string(&state)
                .unwrap()
                .contains(backup_secret)
        );
        assert_eq!(state.ui.friend_backups.capacity_bytes, 0);
        assert_eq!(state.ui.friend_backups.friends.len(), 0);
    }

    #[test]
    fn friend_backup_actions_preserve_retained_data_and_report_physical_usage() {
        use hashtree_core::{DirEntry, HashTree, HashTreeConfig, LinkType};
        use hashtree_fs::FsBlobStore;
        use iris_drive_core::friend_backup::storage::{FriendBackupStore, prepare_backup};
        use nostr_sdk::nips::nip44;

        let local_dir = tempfile::tempdir().unwrap();
        let remote_dir = tempfile::tempdir().unwrap();
        let local = app(local_dir.path());
        let _remote = app(remote_dir.path());
        let friend = backup_npub(remote_dir.path()).unwrap();
        let keys = iris_drive_core::friend_backup::backup_keys(
            &iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(remote_dir.path()))
                .unwrap(),
        )
        .unwrap();
        let capacity = local.dispatch(NativeAppAction::SetFriendBackupCapacity {
            capacity_bytes: 1_000_000,
        });
        assert!(capacity.error.is_empty(), "{}", capacity.error);
        let offered = local.dispatch(NativeAppAction::AddBackupFriend {
            contact: friend.clone(),
            label: "Friend".to_owned(),
            quota_bytes: 1_000_000,
        });
        assert!(offered.error.is_empty(), "{}", offered.error);
        let store = FriendBackupStore::open(local_dir.path().join("friend-backups")).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let report = runtime.block_on(async {
            let tree = HashTree::new(HashTreeConfig::new(std::sync::Arc::new(
                FsBlobStore::new(remote_dir.path().join("own")).unwrap(),
            )));
            let contents = b"private backup contents";
            let (file, _) = tree.put(contents).await.unwrap();
            let root = tree
                .put_directory(vec![DirEntry {
                    name: "private.txt".into(),
                    hash: file.hash,
                    key: file.key,
                    link_type: LinkType::File,
                    size: contents.len() as u64,
                    meta: None,
                }])
                .await
                .unwrap();
            let capsule = nip44::encrypt(
                keys.secret_key(),
                &keys.public_key(),
                root.to_string(),
                nip44::Version::V2,
            )
            .unwrap();
            let prepared = prepare_backup(&tree, &root, capsule, remote_dir.path().join("export"))
                .await
                .unwrap();
            store
                .retain_authorized(
                    local_dir.path(),
                    &friend,
                    &prepared.manifest,
                    prepared.route().as_ref(),
                )
                .await
                .unwrap()
        });
        let refreshed = local.refresh();
        assert_eq!(
            refreshed.ui.friend_backups.used_bytes,
            report.retained_bytes
        );
        assert_eq!(
            refreshed.ui.friend_backups.friends[0].used_bytes,
            report.retained_bytes
        );

        let reduced = local.dispatch(NativeAppAction::AddBackupFriend {
            contact: friend.clone(),
            label: "Friend".to_owned(),
            quota_bytes: 0,
        });
        assert!(reduced.error.contains("retained"), "{}", reduced.error);
        let removed = local.dispatch(NativeAppAction::RemoveBackupFriend {
            npub: friend.clone(),
        });
        assert!(removed.error.contains("retained"), "{}", removed.error);
        assert_eq!(removed.ui.friend_backups.friends.len(), 1);
        assert!(store.current_manifest(&friend).unwrap().is_some());
        assert_eq!(store.usage_bytes(&friend).unwrap(), report.retained_bytes);
    }
}
