use std::collections::BTreeSet;

use crate::config::AppConfig;

pub(super) fn full_profile_covers_known_app_key_roots(
    profile: &crate::profile::ProfileState,
    config: &AppConfig,
) -> bool {
    if !profile.has_profile_roster_evidence() {
        return true;
    }
    let active_app_keys = profile
        .active_root_writer_app_key_pubkeys()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let projection = profile.profile_projection();
    config.drives.iter().all(|drive| {
        drive.app_key_roots.keys().all(|app_key_pubkey| {
            active_app_keys.contains(app_key_pubkey)
                || projection.tombstones.contains_key(app_key_pubkey)
        })
    })
}
