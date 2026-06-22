use nostr_sdk::{Filter, PublicKey, SingleLetterTag};

use crate::calendar::CALENDAR_TREE_NAME;
use crate::nostr_events::{
    KIND_APP_KEY_LINK_REQUEST, KIND_DRIVE_ROOT, KIND_HASHTREE_ROOT, app_key_link_request_d_tag,
    drive_root_d_tag,
};
use crate::sharing::share_access_snapshot_d_tag;
use crate::{
    IrisProfileId, KIND_IRIS_PROFILE_ROSTER_OP, KIND_SHARE_ACCESS_SNAPSHOT, SHARE_ACCESS_LABEL,
};

/// Build the relay filter set covering profile roster ops and drive-root
/// events for a single profile's primary drive. Full `AppKeys` roster snapshots
/// are intentionally excluded from relays; `IrisProfile` roster ops are the
/// relay roster format.
///
/// The drive-root filter intentionally does **not** narrow by author:
/// the d-tag `iris-drive/<profile_or_share_id>/<drive>/root` already pins the drive,
/// and `apply_remote_drive_root_event` rejects events from unauthorized
/// `AppKeys`. Skipping the author filter means the daemon
/// doesn't need to re-subscribe every time the roster changes; newly
/// approved `AppKeys` events flow in automatically.
#[must_use]
pub fn subscription_filters(
    current_app_key_pubkey_hex: &str,
    root_scope_id: &str,
    drive_id: &str,
) -> Vec<Filter> {
    subscription_filters_for_shared_roots(current_app_key_pubkey_hex, root_scope_id, drive_id, &[])
}

#[must_use]
pub fn subscription_filters_for_shared_roots(
    current_app_key_pubkey_hex: &str,
    root_scope_id: &str,
    drive_id: &str,
    share_ids: &[IrisProfileId],
) -> Vec<Filter> {
    let mut filters = Vec::new();
    if let Ok(profile_id) = root_scope_id.parse::<IrisProfileId>() {
        filters.push(iris_profile_roster_op_filter(profile_id));
        filters.push(
            Filter::new()
                .kind(nostr_sdk::Kind::from(KIND_APP_KEY_LINK_REQUEST))
                .custom_tag(
                    SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
                    app_key_link_request_d_tag(profile_id),
                ),
        );
    }
    push_drive_root_filters(&mut filters, root_scope_id, drive_id);
    for share_id in share_ids {
        let share_scope = share_id.to_string();
        if share_scope == root_scope_id {
            continue;
        }
        filters.push(share_access_snapshot_filter(*share_id));
        push_drive_root_filters(&mut filters, &share_scope, crate::PRIMARY_DRIVE_ID);
    }
    if let Ok(current_app_key) = PublicKey::from_hex(current_app_key_pubkey_hex) {
        let mut tree_names = vec![drive_id, CALENDAR_TREE_NAME];
        tree_names.sort_unstable();
        tree_names.dedup();
        for tree_name in tree_names {
            filters.push(
                Filter::new()
                    .author(current_app_key)
                    .kind(nostr_sdk::Kind::from(KIND_HASHTREE_ROOT))
                    .identifier(tree_name)
                    .custom_tag(
                        SingleLetterTag::lowercase(nostr_sdk::Alphabet::L),
                        hashtree_nostr::HASHTREE_LABEL,
                    ),
            );
        }
    }
    filters
}

fn push_drive_root_filters(filters: &mut Vec<Filter>, root_scope_id: &str, drive_id: &str) {
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
            .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), d_tag),
    );
}

pub(crate) fn iris_profile_roster_op_filter(profile_id: IrisProfileId) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_IRIS_PROFILE_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::I),
            profile_id.to_string(),
        )
}

pub(crate) fn share_access_snapshot_filter(share_id: IrisProfileId) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_SHARE_ACCESS_SNAPSHOT))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::L),
            SHARE_ACCESS_LABEL,
        )
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
            share_access_snapshot_d_tag(share_id),
        )
}
