//! Owner-only, cross-client Trash metadata and account root registration.
use anyhow::{Result, ensure};

mod operations;
mod tree;
pub use operations::{list, restore, trash};
pub(crate) use operations::{materialize_for_admission, stage_share_return, verify_share_return};

#[cfg(test)]
mod tests;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, Version};

use crate::{
    AppConfig, AppKeyRootRef, Drive, DriveRole, NostrIdentityId, PRIMARY_DRIVE_ID, ProfileState,
};

pub(crate) fn can_write(state: &ProfileState, pubkey: &str) -> bool {
    state
        .profile_projection()
        .active_facets
        .get(pubkey)
        .is_some_and(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
}

pub(crate) fn can_open(state: &ProfileState, pubkey: &str) -> bool {
    state
        .profile_projection()
        .active_facets
        .get(pubkey)
        .is_some_and(|facet| {
            facet.is_app_key() || facet.has_purpose(crate::NostrIdentityKeyPurpose::RecoveryPhrase)
        })
}

pub(crate) fn recipients(state: &ProfileState, drive: &Drive) -> Vec<String> {
    if drive.root_scope_id != state.root_scope_id() || !can_write(state, &state.app_key_pubkey) {
        return Vec::new();
    }
    let mut recipients = state
        .profile_projection()
        .active_facets
        .values()
        .filter(|facet| {
            facet.is_app_key() || facet.has_purpose(crate::NostrIdentityKeyPurpose::RecoveryPhrase)
        })
        .map(|facet| facet.pubkey.clone())
        .collect::<Vec<_>>();
    recipients.sort();
    recipients.dedup();
    recipients
}

pub(crate) fn root_was_accepted(
    config: &AppConfig,
    author: &str,
    scope: &str,
    root: &AppKeyRootRef,
) -> bool {
    let Some(state) = config.profile.as_ref() else {
        return false;
    };
    scope == state.root_scope_id()
        && can_write(state, author)
        && config
            .drive(DRIVE_ID)
            .filter(|drive| drive.root_scope_id == scope)
            .and_then(|drive| drive.app_key_roots.get(author))
            .is_some_and(|current| current.root_cid == root.root_cid)
}

pub const DRIVE_ID: &str = "trash";
pub const RETENTION_SECS: i64 = 30 * 24 * 60 * 60;
pub const ITEM_FILE: &str = "item.json";
pub const CONTENT_DIR: &str = "content";
pub const REMOVED_DIR: &str = "removed";

pub(crate) fn owner_drive(root_scope_id: String) -> Drive {
    Drive {
        root_scope_id,
        drive_id: DRIVE_ID.into(),
        display_name: "Trash".into(),
        role: DriveRole::Owner,
        app_key_roots: std::collections::BTreeMap::new(),
        last_root_cid: None,
        key_hex: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashItem {
    pub v: u32,
    pub id: String,
    pub account: NostrIdentityId,
    pub source_scope: NostrIdentityId,
    pub tree_name: String,
    pub path: Vec<String>,
    pub deleted_at: i64,
}

impl TrashItem {
    pub fn validate(&self, account: NostrIdentityId, slot: &str) -> Result<()> {
        ensure!(self.v == 1, "Unsupported Trash item version");
        ensure!(self.id == slot && valid_id(slot), "Invalid Trash item ID");
        ensure!(
            self.account == account
                && self.source_scope == account
                && self.tree_name == PRIMARY_DRIVE_ID,
            "Trash item belongs to a different account or drive"
        );
        ensure!(
            !self.path.is_empty() && self.path.iter().all(|part| valid_component(part)),
            "Invalid Trash item path"
        );
        ensure!(self.deleted_at >= 0, "Invalid Trash deletion time");
        Ok(())
    }

    #[must_use]
    pub fn expired(&self, now: i64) -> bool {
        now >= self.deleted_at.saturating_add(RETENTION_SECS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashRemoval {
    pub v: u32,
    pub id: String,
    pub removed_at: i64,
}

impl TrashRemoval {
    pub fn validate(&self, slot: &str) -> Result<()> {
        ensure!(
            self.v == 1 && self.id == slot && valid_id(slot) && self.removed_at >= 0,
            "Invalid Trash removal marker"
        );
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version() == Some(Version::Random) && id.hyphenated().to_string() == value
    })
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | ".." | crate::merge::META_DIR)
        && !value.contains(['/', '\\', ':', '\0'])
}
