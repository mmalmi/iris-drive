fn merged_drive_roots_key(config: &AppConfig) -> Option<String> {
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    let mut key = format!("drive:{}", drive.drive_id);
    for (app_key_pubkey, root) in &drive.app_key_roots {
        key.push('|');
        key.push_str(app_key_pubkey);
        key.push(':');
        key.push_str(&root.root_cid);
        key.push(':');
        key.push_str(&root.app_key_seq.to_string());
    }
    Some(key)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootApplyFollowupKey {
    AppKeyRoots(Vec<(String, String, u64)>),
    SharedFolder(Vec<SharedRootVersion>),
    MergedDriveRoots(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootApplyFollowupQueueKey {
    AppKeyRoots(Vec<String>),
    SharedFolder(Vec<SharedRootSource>),
    MergedDriveRoots(String),
}

fn update_last_provider_root_key(config_dir: &Path, last_root_key: &mut Option<String>) {
    if let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) {
        *last_root_key = current_app_key_root_key(&config);
    }
}

fn root_apply_followup_key(
    config: &AppConfig,
    root_cid_to_pull: Option<&str>,
    should_refresh_projection: bool,
) -> Option<RootApplyFollowupKey> {
    if let Some(root_cid) = root_cid_to_pull
        && let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
    {
        let roots = drive
            .app_key_roots
            .iter()
            .filter(|(_, root)| root.root_cid == root_cid)
            .map(|(device, root)| (device.clone(), root.root_cid.clone(), root.app_key_seq))
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            return Some(RootApplyFollowupKey::AppKeyRoots(roots));
        }
    }
    if let Some(cid) = root_cid_to_pull {
        let roots = matching_share_root_versions(config, cid);
        if !roots.is_empty() {
            return Some(RootApplyFollowupKey::SharedFolder(roots));
        }
    }
    if should_refresh_projection {
        return merged_drive_roots_key(config).map(RootApplyFollowupKey::MergedDriveRoots);
    }
    None
}

fn root_apply_followup_queue_key(
    config: &AppConfig,
    root_cid_to_pull: Option<&str>,
    should_refresh_projection: bool,
) -> Option<RootApplyFollowupQueueKey> {
    if let Some(root_cid) = root_cid_to_pull
        && let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
    {
        let roots = drive
            .app_key_roots
            .iter()
            .filter(|(_, root)| root.root_cid == root_cid)
            .map(|(device, _)| device.clone())
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            return Some(RootApplyFollowupQueueKey::AppKeyRoots(roots));
        }
    }
    if let Some(cid) = root_cid_to_pull {
        let roots = matching_share_root_versions(config, cid);
        if !roots.is_empty() {
            return Some(RootApplyFollowupQueueKey::SharedFolder(
                roots.into_iter().map(|version| version.source).collect(),
            ));
        }
    }
    if should_refresh_projection {
        return merged_drive_roots_key(config).map(RootApplyFollowupQueueKey::MergedDriveRoots);
    }
    None
}

fn root_apply_followup_is_stale(
    config_dir: &Path,
    expected_root_key: Option<&RootApplyFollowupKey>,
) -> bool {
    let Some(expected_root_key) = expected_root_key else {
        return false;
    };
    let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) else {
        return false;
    };
    match expected_root_key {
        RootApplyFollowupKey::AppKeyRoots(roots) => {
            let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
                return true;
            };
            roots.iter().any(|(device, root_cid, app_key_seq)| {
                drive.app_key_roots.get(device).is_none_or(|root| {
                    root.root_cid != *root_cid || root.app_key_seq != *app_key_seq
                })
            })
        }
        RootApplyFollowupKey::SharedFolder(expected) => expected.iter().any(|version| {
            !accepted_shared_roots(&config).any(|(folder, key, root)| {
                folder == version.source.folder
                    && key == version.source.writer
                    && root.root_cid == version.cid
                    && root.app_key_seq == version.seq
            })
        }),
        RootApplyFollowupKey::MergedDriveRoots(expected) => {
            merged_drive_roots_key(&config).as_deref() != Some(expected.as_str())
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SharedRootSource {
    folder: iris_drive_core::NostrIdentityId,
    writer: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SharedRootVersion {
    source: SharedRootSource,
    cid: String,
    seq: u64,
}

fn matching_share_root_versions(config: &AppConfig, cid: &str) -> Vec<SharedRootVersion> {
    accepted_shared_roots(config)
        .filter(|(_, _, root)| root.root_cid == cid)
        .map(|(folder, writer, root)| SharedRootVersion {
            source: SharedRootSource {
                folder,
                writer: writer.to_owned(),
            },
            cid: root.root_cid.clone(),
            seq: root.app_key_seq,
        })
        .collect()
}

fn accepted_shared_roots(
    config: &AppConfig,
) -> impl Iterator<
    Item = (
        iris_drive_core::NostrIdentityId,
        &str,
        &iris_drive_core::AppKeyRootRef,
    ),
> {
    config
        .shared_folders
        .iter()
        .filter(|folder| {
            config.profile.as_ref().is_some_and(|profile| {
                folder
                    .active_member_for_app_key(&profile.app_key_pubkey)
                    .is_some()
            })
        })
        .flat_map(|folder| {
            let writers = iris_drive_core::shared_folder_authorized_writer_pubkeys(folder);
            folder
                .app_key_roots
                .iter()
                .filter(move |(key, _)| writers.contains(key))
                .map(move |(key, root)| (folder.share_id, key.as_str(), root))
        })
}

fn accepted_current_drive_root(
    config: &AppConfig,
    (author, scope, drive_id, root_ref): &(String, String, String, iris_drive_core::AppKeyRootRef),
) -> bool {
    config
        .drive(drive_id)
        .filter(|drive| drive.root_scope_id == *scope)
        .and_then(|drive| drive.app_key_roots.get(author))
        .is_some_and(|root| root.root_cid == root_ref.root_cid)
        || (drive_id == iris_drive_core::PRIMARY_DRIVE_ID
            && scope
                .parse()
                .ok()
                .and_then(|id| config.shared_folder(id))
                .and_then(|folder| folder.app_key_roots.get(author))
                .is_some_and(|root| root.root_cid == root_ref.root_cid))
}
