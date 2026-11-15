//! A profile's shared folders, the same on all of its devices.
//!
//! Google Drive keeps sharing with the account. Drive has no account server,
//! so each device keeps the list and sends it to the profile's other devices,
//! encrypted for them. Folders merge one by one, the newest change winning; a
//! folder the profile stopped sharing or left stays in the list as removed.
//!
//! The list says which folders to have and where they sit in My Drive. It
//! never grants access: a device adds a folder only from the signed access
//! list of the admin the entry names, once that list includes the device.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::share_join::{ShareAnchor, ShareJoinLink};
use crate::sharing::{ShareRole, ShareShortcut, SharedFolder};
use crate::{NostrIdentityId, ProfileState};

pub mod admit;
pub mod event;
pub mod sync;

pub use event::{
    SHARE_INDEX_LABEL, current_share_index_event, is_share_index_event, share_index_filter,
};
pub use sync::{
    ShareIndexApply, ShareIndexReconcile, apply_share_index_event, apply_share_index_event_locked,
    publish_share_index, reconcile_share_index, sync_share_index_once,
};

pub const SHARE_INDEX_FILE: &str = "share-index.json";

/// Where a member added a shared folder to My Drive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ShareIndexPlace {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_path: String,
}

/// One shared folder of the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareIndexEntry {
    pub share_id: NostrIdentityId,
    pub owner_profile_id: NostrIdentityId,
    /// The admin device whose signed access list a device trusts when it
    /// first adds the folder.
    pub admin: String,
    pub name: String,
    pub role: ShareRole,
    /// The key of the join link the profile used, so a device the admin has
    /// not let in yet can ask with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_key: Option<String>,
    /// Where the owner keeps the folder. Empty for folders others own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<ShareIndexPlace>,
    /// The profile stopped sharing the folder or left it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub removed: bool,
    /// How many times the profile came back to the folder after removing it.
    /// Within one, removal wins: a device that moved the folder before it
    /// heard the profile left does not bring it back.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub generation: u32,
    pub updated_at: i64,
    pub updated_by: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl ShareIndexEntry {
    fn newer_than(&self, other: &Self) -> bool {
        (
            self.generation,
            self.removed,
            self.updated_at,
            &self.updated_by,
        ) > (
            other.generation,
            other.removed,
            other.updated_at,
            &other.updated_by,
        )
    }

    #[must_use]
    pub fn anchor(&self) -> ShareAnchor {
        ShareAnchor {
            share_id: self.share_id,
            owner_profile_id: self.owner_profile_id,
            admin: self.admin.clone(),
            name: self.name.clone(),
        }
    }

    /// The join link the profile used, rebuilt for a device that never saw it.
    #[must_use]
    pub fn link(&self) -> Option<ShareJoinLink> {
        Some(ShareJoinLink {
            v: 1,
            share_id: self.share_id,
            owner_profile_id: self.owner_profile_id,
            admin: self.admin.clone(),
            name: self.name.clone(),
            role: self.role,
            key: self.link_key.clone()?,
            doc: None,
        })
    }

    /// Entries from another device are data: keep only well-formed ones.
    fn is_well_formed(&self) -> bool {
        let hex32 = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        };
        hex32(&self.admin)
            && hex32(&self.updated_by)
            && self.link_key.as_deref().is_none_or(hex32)
            && self.name.chars().count() <= 200
            && self.shortcuts.len() <= 64
            && (self.source_path.is_empty()
                || crate::provider::normalize_provider_path(&self.source_path)
                    .is_ok_and(|path| path == self.source_path))
            && self.shortcuts.iter().all(|place| {
                ShareShortcut::new(self.share_id, &place.path, &place.target_path).is_ok_and(
                    |shortcut| {
                        shortcut.path == place.path && shortcut.target_path == place.target_path
                    },
                )
            })
    }
}

/// The list as this device keeps it, in a private file beside the config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareIndex {
    #[serde(default)]
    pub entries: BTreeMap<String, ShareIndexEntry>,
    /// Folders this device has. One that then leaves its config was removed
    /// here; a listed folder it never had is still to be added.
    #[serde(default)]
    pub adopted: BTreeSet<String>,
}

/// The most folders a list from another device may carry.
pub const MAX_SHARE_INDEX_ENTRIES: usize = 4096;

impl ShareIndex {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join(SHARE_INDEX_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("reading {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let path = config_dir.join(SHARE_INDEX_FILE);
        let bytes = serde_json::to_vec_pretty(self)?;
        crate::atomic_file::atomic_write_private(&path, &bytes)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// A time after every change this device has seen, so what it does next
    /// wins over them whatever the clocks say.
    fn next_stamp(&self, now: i64) -> i64 {
        self.entries
            .values()
            .map(|entry| entry.updated_at.saturating_add(1))
            .fold(now, i64::max)
    }

    /// The generation of a folder the profile comes back to: one past the
    /// removal it knows of.
    fn generation_after_removal(&self, key: &str) -> u32 {
        self.entries
            .get(key)
            .map_or(0, |removed| removed.generation.saturating_add(1))
    }

    #[must_use]
    pub fn entry(&self, share_id: NostrIdentityId) -> Option<&ShareIndexEntry> {
        self.entries
            .get(&share_id.to_string())
            .filter(|entry| !entry.removed)
    }

    /// Folders the profile has that this device has not added yet.
    pub fn wanted(&self) -> impl Iterator<Item = &ShareIndexEntry> {
        self.entries
            .iter()
            .filter(|(key, entry)| !entry.removed && !self.adopted.contains(*key))
            .map(|(_, entry)| entry)
    }

    /// Fold what this device did to its shared folders into the list: folders
    /// it shared or joined, where they sit now, and folders it removed.
    pub fn capture_local(&mut self, config: &AppConfig, now: i64) -> bool {
        let Some(state) = config.profile.as_ref() else {
            return false;
        };
        let stamp = self.next_stamp(now);
        let mut changed = false;
        let mut present = BTreeSet::new();
        let mut revoked = None;
        for folder in &config.shared_folders {
            let key = folder.share_id.to_string();
            present.insert(key.clone());
            let (source_path, shortcuts) = local_place(config, state, folder);
            if let Some(entry) = self.entries.get_mut(&key).filter(|entry| !entry.removed) {
                let admin = reanchored_admin(state, folder, &entry.admin, &mut revoked);
                if entry.source_path != source_path
                    || entry.shortcuts != shortcuts
                    || entry.admin != admin
                {
                    entry.source_path = source_path;
                    entry.shortcuts = shortcuts;
                    entry.admin = admin;
                    entry.updated_at = stamp;
                    entry.updated_by.clone_from(&state.app_key_pubkey);
                    changed = true;
                }
            } else {
                let generation = self.generation_after_removal(&key);
                self.entries.insert(
                    key.clone(),
                    ShareIndexEntry {
                        share_id: folder.share_id,
                        owner_profile_id: folder.owner_profile_id,
                        admin: default_admin(folder, &state.app_key_pubkey),
                        name: folder.display_name.clone(),
                        role: folder
                            .active_member_for_app_key(&state.app_key_pubkey)
                            .map_or(folder.local_role, |member| member.role),
                        link_key: None,
                        source_path,
                        shortcuts,
                        removed: false,
                        generation,
                        updated_at: stamp,
                        updated_by: state.app_key_pubkey.clone(),
                    },
                );
                changed = true;
            }
            changed |= self.adopted.insert(key);
        }
        let gone = self
            .adopted
            .iter()
            .filter(|key| !present.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in gone {
            self.adopted.remove(&key);
            if let Some(entry) = self.entries.get_mut(&key).filter(|entry| !entry.removed) {
                entry.removed = true;
                entry.link_key = None;
                entry.source_path.clear();
                entry.shortcuts.clear();
                entry.updated_at = stamp;
                entry.updated_by.clone_from(&state.app_key_pubkey);
            }
            changed = true;
        }
        changed
    }

    /// Remember the link a folder was joined with and the admin who answers
    /// it, so the profile's other devices can join the same way.
    pub fn note_link(&mut self, link: &ShareJoinLink, device: &str, now: i64) -> bool {
        let stamp = self.next_stamp(now);
        let key = link.share_id.to_string();
        if let Some(entry) = self.entries.get_mut(&key).filter(|entry| !entry.removed) {
            if entry.link_key.as_deref() == Some(link.key.as_str()) && entry.admin == link.admin {
                return false;
            }
            entry.link_key = Some(link.key.clone());
            entry.admin.clone_from(&link.admin);
            entry.updated_at = stamp;
            device.clone_into(&mut entry.updated_by);
            return true;
        }
        let generation = self.generation_after_removal(&key);
        self.entries.insert(
            key,
            ShareIndexEntry {
                share_id: link.share_id,
                owner_profile_id: link.owner_profile_id,
                admin: link.admin.clone(),
                name: link.display_name(),
                role: link.role,
                link_key: Some(link.key.clone()),
                source_path: String::new(),
                shortcuts: Vec::new(),
                removed: false,
                generation,
                updated_at: stamp,
                updated_by: device.to_owned(),
            },
        );
        true
    }

    /// Take in another device's list. Each folder keeps its newest change.
    pub fn merge(&mut self, incoming: impl IntoIterator<Item = ShareIndexEntry>) -> bool {
        let mut changed = false;
        for entry in incoming.into_iter().take(MAX_SHARE_INDEX_ENTRIES) {
            if !entry.is_well_formed() {
                continue;
            }
            let key = entry.share_id.to_string();
            if self
                .entries
                .get(&key)
                .is_none_or(|current| entry.newer_than(current))
            {
                self.entries.insert(key, entry);
                changed = true;
            }
        }
        changed
    }

    /// Make this device's folders match the list: drop the ones the profile
    /// removed and put the rest where the list says they sit.
    pub fn apply_to_config(&mut self, config: &mut AppConfig) -> bool {
        let mut changed = false;
        for (key, entry) in &self.entries {
            if config.shared_folder(entry.share_id).is_none() {
                continue;
            }
            if entry.removed {
                config.remove_shared_folder(entry.share_id);
                config.remove_share_shortcuts_for_share(entry.share_id);
                self.adopted.remove(key);
                changed = true;
            } else {
                changed |= place(config, entry);
            }
        }
        changed
    }
}

/// Put a folder this device has where `entry` says it sits in My Drive.
pub(crate) fn place(config: &mut AppConfig, entry: &ShareIndexEntry) -> bool {
    let Some(profile_id) = config.profile.as_ref().map(|state| state.profile_id) else {
        return false;
    };
    let mut changed = false;
    if entry.owner_profile_id == profile_id
        && !entry.source_path.is_empty()
        && let Some(folder) = config
            .shared_folders
            .iter_mut()
            .find(|folder| folder.share_id == entry.share_id)
        && folder.source_path != entry.source_path
    {
        folder.source_path.clone_from(&entry.source_path);
        changed = true;
    }
    let mut wanted = entry
        .shortcuts
        .iter()
        .filter_map(|place| {
            ShareShortcut::new(entry.share_id, &place.path, &place.target_path).ok()
        })
        .collect::<Vec<_>>();
    wanted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut current = config
        .share_shortcuts
        .iter()
        .filter(|shortcut| shortcut.share_id == entry.share_id)
        .cloned()
        .collect::<Vec<_>>();
    current.sort_by(|a, b| a.path.cmp(&b.path));
    if current != wanted {
        config.remove_share_shortcuts_for_share(entry.share_id);
        for shortcut in wanted {
            config.upsert_share_shortcut(shortcut);
        }
        changed = true;
    }
    changed
}

fn local_place(
    config: &AppConfig,
    state: &ProfileState,
    folder: &SharedFolder,
) -> (String, Vec<ShareIndexPlace>) {
    let source_path = if folder.owner_profile_id == state.profile_id {
        folder.source_path.clone()
    } else {
        String::new()
    };
    let mut shortcuts = config
        .share_shortcuts
        .iter()
        .filter(|shortcut| shortcut.share_id == folder.share_id)
        .map(|shortcut| ShareIndexPlace {
            path: shortcut.path.clone(),
            target_path: shortcut.target_path.clone(),
        })
        .collect::<Vec<_>>();
    shortcuts.sort();
    (source_path, shortcuts)
}

/// The admin whose signed access list this device holds: itself when it
/// manages the folder, else whoever last changed who has access.
fn default_admin(folder: &SharedFolder, device: &str) -> String {
    if crate::shared_folder_app_key_can_admin(folder, device) {
        return device.to_owned();
    }
    let admins = folder
        .projection()
        .active_facets
        .keys()
        .filter(|pubkey| crate::shared_folder_app_key_can_admin(folder, pubkey))
        .cloned()
        .collect::<Vec<_>>();
    folder
        .access
        .key_epochs
        .values()
        .next_back()
        .map(|epoch| epoch.signed_by_pubkey.clone())
        .filter(|signer| admins.contains(signer))
        .or_else(|| admins.into_iter().next())
        .unwrap_or_else(|| device.to_owned())
}

/// A device that manages the folder takes over from an admin device the
/// profile no longer has, so devices linked later can still be let in.
/// `revoked` holds the profile's removed devices once they were looked up.
fn reanchored_admin(
    state: &ProfileState,
    folder: &SharedFolder,
    admin: &str,
    revoked: &mut Option<BTreeSet<String>>,
) -> String {
    let device = &state.app_key_pubkey;
    if admin == device || !crate::shared_folder_app_key_can_admin(folder, device) {
        return admin.to_owned();
    }
    let revoked =
        revoked.get_or_insert_with(|| state.profile_projection().tombstones.into_keys().collect());
    if revoked.contains(admin) {
        device.clone()
    } else {
        admin.to_owned()
    }
}

#[cfg(test)]
mod tests;
