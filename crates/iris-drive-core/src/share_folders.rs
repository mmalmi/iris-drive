//! Which folders of My Drive sharing can start from: the shells' folder
//! pickers and the "Share" action the OS file browser offers on a folder.

use std::collections::BTreeSet;

use crate::AppConfig;

fn within(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Paths this profile shares from, and the shortcuts it keeps to joined folders.
fn share_mounts(config: &AppConfig) -> (BTreeSet<&str>, Vec<&str>) {
    let profile = config.profile.as_ref().map(|profile| profile.profile_id);
    let owned = config
        .shared_folders
        .iter()
        .filter(|folder| Some(folder.owner_profile_id) == profile)
        .map(|folder| folder.source_path.as_str())
        .collect::<BTreeSet<_>>();
    let mounts = owned
        .iter()
        .copied()
        .chain(config.share_shortcuts.iter().map(|s| s.path.as_str()))
        .collect();
    (owned, mounts)
}

fn clear_of(path: &str, mounts: &[&str]) -> bool {
    !mounts
        .iter()
        .any(|mount| within(path, mount) || within(mount, path))
}

fn startable(dir: &str, mounts: &[&str]) -> bool {
    !dir.is_empty() && !dir.split('/').any(|part| part.starts_with('.')) && clear_of(dir, mounts)
}

/// Whether a new share could start at `path`: it is neither inside nor around
/// a folder that is already shared.
#[must_use]
pub fn can_start_share(config: &AppConfig, path: &str) -> bool {
    clear_of(path, &share_mounts(config).1)
}

/// Folders of My Drive a share could start from: not hidden, and neither
/// inside nor around a folder that is already shared.
#[must_use]
pub fn shareable_folders(config: &AppConfig, directories: &[String]) -> Vec<String> {
    let (_, mounts) = share_mounts(config);
    directories
        .iter()
        .filter(|dir| startable(dir, &mounts))
        .cloned()
        .collect()
}

/// Folders the OS file browser offers "Share" on: ones a share could start
/// from, and the ones this profile already shares.
#[must_use]
pub fn share_action_folders(config: &AppConfig, directories: &[String]) -> Vec<String> {
    let (owned, mounts) = share_mounts(config);
    directories
        .iter()
        .filter(|dir| owned.contains(dir.as_str()) || startable(dir, &mounts))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::sharing::{ShareShortcut, create_shared_folder};

    fn config_with_shares() -> (tempfile::TempDir, AppConfig) {
        let dir = tempfile::tempdir().unwrap();
        let me = Profile::create(dir.path(), None).unwrap();
        let mine = create_shared_folder(
            me.app_key.keys(),
            me.state.profile_id,
            "Projects/Trip",
            "Trip",
            None,
            Vec::new(),
            1,
        )
        .unwrap();
        let theirs = create_shared_folder(
            &nostr_sdk::Keys::generate(),
            crate::NostrIdentityId::new_v4(),
            "Team/Plans",
            "Plans",
            None,
            Vec::new(),
            1,
        )
        .unwrap();
        let mut config = AppConfig {
            profile: Some(me.state.clone()),
            ..AppConfig::default()
        };
        let joined = theirs.share_id;
        config.upsert_shared_folder(mine);
        config.upsert_shared_folder(theirs);
        config.upsert_share_shortcut(ShareShortcut::new(joined, "Joined/Plans", "").unwrap());
        (dir, config)
    }

    fn directories() -> Vec<String> {
        [
            ".Trash",
            "Documents",
            "Documents/.cache",
            "Joined",
            "Joined/Plans",
            "Joined/Plans/Drafts",
            "Photos",
            "Projects",
            "Projects/Trip",
            "Projects/Trip/Day 1",
            "Team",
            "Team/Plans",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn shareable_folders_skip_hidden_and_folders_in_or_around_a_share() {
        let (_dir, config) = config_with_shares();

        assert_eq!(
            shareable_folders(&config, &directories()),
            ["Documents", "Photos", "Team", "Team/Plans"]
        );
    }

    #[test]
    fn share_action_folders_add_the_folders_this_profile_shares() {
        let (_dir, config) = config_with_shares();

        assert_eq!(
            share_action_folders(&config, &directories()),
            ["Documents", "Photos", "Projects/Trip", "Team", "Team/Plans"]
        );
    }
}
