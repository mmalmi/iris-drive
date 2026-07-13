use nostr_sdk::{Filter, PublicKey, SingleLetterTag, Timestamp};

use crate::calendar::CALENDAR_TREE_NAME;
use crate::nostr_events::{KIND_DRIVE_ROOT, KIND_HASHTREE_ROOT, drive_root_d_tag};
use crate::sharing::share_access_snapshot_d_tag;
use crate::{
    KIND_NOSTR_IDENTITY_ROSTER_OP, KIND_SHARE_ACCESS_SNAPSHOT, NostrIdentityId, SHARE_ACCESS_LABEL,
};

/// Build the relay filter set covering profile roster ops and drive-root
/// events for a single profile's primary drive. Full `AppKeys` roster snapshots
/// are intentionally excluded from relays; `NostrIdentity` roster ops are the
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
    share_ids: &[NostrIdentityId],
) -> Vec<Filter> {
    let mut filters = Vec::new();
    if let Ok(profile_id) = root_scope_id.parse::<NostrIdentityId>() {
        filters.push(nostr_identity_roster_op_filter(profile_id));
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
        filters.push(device_approval_applied_ack_filter(current_app_key));
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

pub(crate) fn device_approval_applied_ack_filter(admin_app_key: PublicKey) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::P),
            admin_app_key.to_hex(),
        )
        .limit(64)
}

#[must_use]
pub fn device_approval_receipt_filter(state: &crate::ProfileState) -> Option<Filter> {
    device_approval_receipt_subscription(state).map(|(_, filter)| filter)
}

pub(crate) fn device_approval_receipt_subscription(
    state: &crate::ProfileState,
) -> Option<(String, Filter)> {
    let pending = state.outbound_app_key_link_request.as_ref()?;
    let (bootstrap, _) =
        crate::app_key_link_transport::parse_pending_app_key_approval_bootstrap(pending).ok()?;
    let request_pubkey = PublicKey::parse(&bootstrap.request_npub).ok()?;
    let request_pubkey_hex = request_pubkey.to_hex();
    Some((
        request_pubkey_hex.clone(),
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .custom_tag(
                SingleLetterTag::lowercase(nostr_sdk::Alphabet::P),
                request_pubkey_hex,
            )
            .since(Timestamp::from(pending.requested_at))
            .limit(8),
    ))
}

fn push_drive_root_filters(filters: &mut Vec<Filter>, root_scope_id: &str, drive_id: &str) {
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
            .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), d_tag),
    );
}

pub(crate) fn nostr_identity_roster_op_filter(profile_id: NostrIdentityId) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::I),
            profile_id.to_string(),
        )
}

pub(crate) fn share_access_snapshot_filter(share_id: NostrIdentityId) -> Filter {
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
