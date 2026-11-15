//! Letting a profile's own devices into its shared folders.
//!
//! Access to a folder is granted to device keys, so a device linked to the
//! profile later cannot open the profile's folders by itself. The device the
//! list names as a folder's admin adds it; one device answers for a folder so
//! two never rewrite its access list at once.

use anyhow::{Context, Result};
use nostr_sdk::Keys;

use super::ShareIndex;
use crate::config::AppConfig;
use crate::sharing::add_shared_folder_member_devices;
use crate::{NostrIdentityId, shared_folder_app_key_can_admin};

/// Add the profile's devices that a folder does not have yet, in every
/// folder this device answers for. Returns the folders whose access changed;
/// the caller publishes them.
pub(crate) fn admit_profile_devices(
    config: &mut AppConfig,
    index: &ShareIndex,
    keys: &Keys,
    now: i64,
) -> Result<Vec<NostrIdentityId>> {
    let state = config.profile.clone().context("Profile required")?;
    let device = &state.app_key_pubkey;
    let devices = state.active_root_writer_app_key_pubkeys();
    let mut admitted = Vec::new();
    for folder in &mut config.shared_folders {
        if index
            .entry(folder.share_id)
            .is_none_or(|entry| entry.admin != *device)
            || !shared_folder_app_key_can_admin(folder, device)
        {
            continue;
        }
        let Some(member) = folder
            .active_member_for_app_key(device)
            .filter(|member| member.profile_id == state.profile_id)
        else {
            continue;
        };
        let projection = folder.projection();
        let missing = devices
            .iter()
            .filter(|pubkey| {
                !projection.active_facets.contains_key(*pubkey)
                    && !folder.access.tombstones.contains_key(*pubkey)
            })
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            continue;
        }
        add_shared_folder_member_devices(folder, keys, &member, &missing, now)?;
        admitted.push(folder.share_id);
    }
    Ok(admitted)
}
