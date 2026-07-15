use std::collections::{BTreeMap, BTreeSet};

use crate::app_keys::{AppActorEntry, AppActorRole};
use crate::nostr_identity::{NostrIdentityKeyPurpose, SecretWrapStatus};
use crate::profile::{AppKeyAuthorizationState, ProfileState};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;

#[must_use]
pub fn authorization_state_key(state: AppKeyAuthorizationState) -> &'static str {
    match state {
        AppKeyAuthorizationState::Authorized => "authorized",
        AppKeyAuthorizationState::AwaitingApproval => "awaiting_approval",
        AppKeyAuthorizationState::Revoked => "revoked",
    }
}

#[must_use]
pub fn primary_status_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "ready",
        "awaiting_approval" => "awaiting_approval",
        "revoked" => "revoked",
        _ => "not_setup",
    }
}

#[must_use]
pub fn setup_label_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "Linked",
        "awaiting_approval" => "Awaiting approval",
        "revoked" => "Revoked",
        _ => "Not linked",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupStateFlags {
    pub setup_complete: bool,
    pub awaiting_approval: bool,
    pub revoked: bool,
}

#[must_use]
pub fn setup_state_flags(setup_state: &str) -> SetupStateFlags {
    SetupStateFlags {
        setup_complete: setup_state == "authorized",
        awaiting_approval: setup_state == "awaiting_approval",
        revoked: setup_state == "revoked",
    }
}

#[must_use]
pub fn primary_status_label(primary_status: &str) -> &'static str {
    match primary_status {
        "revoked" => "Device removed",
        "awaiting_approval" => "Waiting for approval",
        _ => "Ready",
    }
}

#[must_use]
pub fn sync_status_label(sync_status: &str) -> String {
    match sync_status {
        "running" => "Sync on".to_owned(),
        "syncing" => "Syncing".to_owned(),
        "synced" => "Synced".to_owned(),
        "root synced" => "Root synced".to_owned(),
        "profile synced" => "Profile synced".to_owned(),
        "up to date" => "Up to date".to_owned(),
        "ready" => "Ready".to_owned(),
        "sync error" => "Sync failed".to_owned(),
        "paused" => "Sync paused".to_owned(),
        value if value.trim().is_empty() => "Ready".to_owned(),
        value => value.to_owned(),
    }
}

#[must_use]
pub fn app_actor_role_key(role: AppActorRole) -> &'static str {
    match role {
        AppActorRole::Admin => "admin",
        AppActorRole::Member => "member",
    }
}

#[must_use]
pub fn app_actor_role_label(role: AppActorRole) -> &'static str {
    match role {
        AppActorRole::Admin => "Admin",
        AppActorRole::Member => "Member",
    }
}

#[must_use]
pub fn app_key_display_label(
    is_current_app_key: bool,
    label: Option<&str>,
    fallback: &str,
) -> String {
    if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
        return label.to_owned();
    }
    let fallback = fallback.trim();
    if !fallback.is_empty() && !looks_like_public_key_label(fallback) {
        return fallback.to_owned();
    }
    if is_current_app_key {
        "This device".to_owned()
    } else {
        "Linked device".to_owned()
    }
}

fn looks_like_public_key_label(value: &str) -> bool {
    value.starts_with("npub1")
        || (value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()))
}

#[allow(clippy::fn_params_excessive_bools)]
#[must_use]
pub fn app_key_connection_state(
    is_current_app_key: bool,
    is_online: bool,
    is_direct: bool,
    is_mesh: bool,
) -> &'static str {
    if is_current_app_key {
        "local"
    } else if is_direct {
        "direct"
    } else if is_mesh {
        "mesh"
    } else if is_online {
        "online"
    } else {
        "offline"
    }
}

#[must_use]
pub fn app_key_connection_label(
    connection_state: &str,
    transport_type: Option<&str>,
    srtt_ms: Option<u64>,
) -> String {
    if connection_state == "local" {
        return "This Device".to_owned();
    }
    if connection_state == "offline" {
        return "Offline".to_owned();
    }
    let transport = transport_type.map(str::to_uppercase);
    match (transport, srtt_ms, connection_state) {
        (Some(transport), Some(srtt_ms), _) => format!("Online ({transport}, {srtt_ms} ms)"),
        (Some(transport), None, _) => format!("Online ({transport})"),
        (None, Some(srtt_ms), _) => format!("Online ({srtt_ms} ms)"),
        (None, None, "mesh") => "Online (Mesh)".to_owned(),
        _ => "Online".to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppKeyManagementActions {
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
}

#[must_use]
pub fn app_key_management_actions(
    can_admin_profile: bool,
    is_current_app_key: bool,
    is_admin: bool,
    admin_count: usize,
) -> AppKeyManagementActions {
    AppKeyManagementActions {
        can_revoke: can_admin_profile && !is_current_app_key,
        can_appoint_admin: can_admin_profile && !is_current_app_key && !is_admin,
        can_demote_admin: can_admin_profile && !is_current_app_key && is_admin && admin_count > 1,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppKeyConnectionDetails {
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppKeyConnectivity {
    pub online_app_keys: BTreeSet<String>,
    pub direct_app_keys: BTreeSet<String>,
    pub mesh_app_keys: BTreeSet<String>,
    pub peer_statuses: BTreeMap<String, AppKeyConnectionDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct AppKeyRosterRow {
    pub pubkey_hex: String,
    pub npub: String,
    pub label: Option<String>,
    pub display_label: String,
    pub role: String,
    pub role_label: String,
    pub state: String,
    pub state_label: String,
    pub is_current_app_key: bool,
    pub is_online: bool,
    pub is_direct: bool,
    pub is_mesh: bool,
    pub online_via: Option<String>,
    pub connection_state: String,
    pub connection_label: String,
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
    pub added_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentitySummary {
    pub profile_id: String,
    pub current_app_key_pubkey_hex: String,
    pub current_app_key_npub: String,
    pub current_app_key_label: Option<String>,
    pub authorization_state: String,
    pub can_write_roots: bool,
    pub can_admin_profile: bool,
    pub active_app_key_count: usize,
    pub profile_roster_op_count: usize,
    pub current_key_epoch: Option<u64>,
    pub recovery_phrase_facet_count: usize,
    pub nip46_facet_count: usize,
    pub social_profile_facet_count: usize,
    pub missing_key_wrap_npubs: Vec<String>,
}

#[must_use]
pub fn pubkey_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| hex.to_owned())
}

#[must_use]
pub fn nostr_identity_summary(state: &ProfileState) -> NostrIdentitySummary {
    let projection = state.profile_projection();
    let has_profile_roster = state.has_profile_roster_evidence();
    let current_facet = projection.active_facets.get(&state.app_key_pubkey);
    let current_key_epoch = projection
        .secret_epochs
        .keys()
        .next_back()
        .copied()
        .or_else(|| {
            state
                .app_keys
                .as_ref()
                .map(|snapshot| snapshot.dck_generation)
        });
    let missing_key_wrap_npubs = current_key_epoch.map_or_else(Vec::new, |epoch| {
        projection
            .active_facets
            .values()
            .filter(|facet| {
                matches!(
                    projection.secret_wrap_status(&facet.pubkey, epoch),
                    SecretWrapStatus::RepairNeeded
                )
            })
            .map(|facet| pubkey_npub(&facet.pubkey))
            .collect()
    });

    NostrIdentitySummary {
        profile_id: state.profile_id.to_string(),
        current_app_key_pubkey_hex: state.app_key_pubkey.clone(),
        current_app_key_npub: pubkey_npub(&state.app_key_pubkey),
        current_app_key_label: current_facet
            .and_then(|facet| facet.label.clone())
            .or_else(|| state.app_key_label.clone()),
        authorization_state: authorization_state_key(state.authorization_state).to_owned(),
        can_write_roots: if has_profile_roster {
            projection.can_write_roots(&state.app_key_pubkey)
        } else {
            state.can_write_roots()
        },
        can_admin_profile: if has_profile_roster {
            projection.can_admin_profile(&state.app_key_pubkey)
        } else {
            state.can_admin_profile()
        },
        active_app_key_count: if has_profile_roster {
            projection.active_app_key_pubkeys().len()
        } else {
            state
                .app_keys
                .as_ref()
                .map_or(0, |snapshot| snapshot.app_actors.len())
        },
        profile_roster_op_count: state.profile_roster_ops.len(),
        current_key_epoch,
        recovery_phrase_facet_count: facet_count_for_purpose(
            &projection,
            NostrIdentityKeyPurpose::RecoveryPhrase,
        ),
        nip46_facet_count: facet_count_for_purpose(
            &projection,
            NostrIdentityKeyPurpose::Nip46Signer,
        ),
        social_profile_facet_count: facet_count_for_purpose(
            &projection,
            NostrIdentityKeyPurpose::SocialProfile,
        ),
        missing_key_wrap_npubs,
    }
}

fn facet_count_for_purpose(
    projection: &crate::NostrIdentityRosterProjection,
    purpose: NostrIdentityKeyPurpose,
) -> usize {
    projection
        .active_facets
        .values()
        .filter(|facet| facet.has_purpose(purpose))
        .count()
}

#[must_use]
pub fn app_key_roster_rows(
    app_actors: &[AppActorEntry],
    current_app_key_pubkey: &str,
    can_admin_profile: bool,
    current_app_key_online: bool,
    connectivity: &AppKeyConnectivity,
) -> Vec<AppKeyRosterRow> {
    let admin_count = app_actors
        .iter()
        .filter(|actor| actor.role == AppActorRole::Admin)
        .count();

    app_actors
        .iter()
        .map(|actor| {
            let npub = pubkey_npub(&actor.pubkey);
            let is_current_app_key = actor.pubkey == current_app_key_pubkey;
            let is_direct = !is_current_app_key && connectivity.direct_app_keys.contains(&npub);
            let is_mesh = !is_current_app_key && connectivity.mesh_app_keys.contains(&npub);
            let is_online = if is_current_app_key {
                current_app_key_online
            } else {
                connectivity.online_app_keys.contains(&npub) || is_direct || is_mesh
            };
            let online_via = app_key_online_via(is_current_app_key, is_online, is_direct, is_mesh);
            let connection_state =
                app_key_connection_state(is_current_app_key, is_online, is_direct, is_mesh)
                    .to_owned();
            let connection = connectivity.peer_statuses.get(&npub);
            let transport_type = connection.and_then(|status| status.transport_type.clone());
            let srtt_ms = connection.and_then(|status| status.srtt_ms);
            let actions = app_key_management_actions(
                can_admin_profile,
                is_current_app_key,
                actor.role == AppActorRole::Admin,
                admin_count,
            );
            AppKeyRosterRow {
                pubkey_hex: actor.pubkey.clone(),
                npub: npub.clone(),
                label: actor.label.clone(),
                display_label: app_key_display_label(
                    is_current_app_key,
                    actor.label.as_deref(),
                    &npub,
                ),
                role: app_actor_role_key(actor.role).to_owned(),
                role_label: app_actor_role_label(actor.role).to_owned(),
                state: "Linked".to_owned(),
                state_label: "Linked".to_owned(),
                is_current_app_key,
                is_online,
                is_direct,
                is_mesh,
                online_via,
                connection_label: app_key_connection_label(
                    &connection_state,
                    transport_type.as_deref(),
                    srtt_ms,
                ),
                connection_state,
                transport_type,
                srtt_ms,
                can_revoke: actions.can_revoke,
                can_appoint_admin: actions.can_appoint_admin,
                can_demote_admin: actions.can_demote_admin,
                added_at: actor.added_at,
            }
        })
        .collect()
}

#[allow(clippy::fn_params_excessive_bools)]
fn app_key_online_via(
    is_current_app_key: bool,
    is_online: bool,
    is_direct: bool,
    is_mesh: bool,
) -> Option<String> {
    if is_current_app_key && is_online {
        Some("local".to_owned())
    } else if is_direct {
        Some("direct".to_owned())
    } else if is_mesh {
        Some("mesh".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppActorRole, AppKeyAuthorizationState, Profile};
    use tempfile::tempdir;

    #[test]
    fn shared_app_key_summary_labels_match_native_clients() {
        assert_eq!(
            authorization_state_key(AppKeyAuthorizationState::Authorized),
            "authorized"
        );
        assert_eq!(
            authorization_state_key(AppKeyAuthorizationState::AwaitingApproval),
            "awaiting_approval"
        );
        assert_eq!(
            authorization_state_key(AppKeyAuthorizationState::Revoked),
            "revoked"
        );

        assert_eq!(primary_status_for_setup_state("authorized"), "ready");
        assert_eq!(setup_label_for_setup_state("authorized"), "Linked");
        assert_eq!(
            setup_state_flags("authorized"),
            SetupStateFlags {
                setup_complete: true,
                awaiting_approval: false,
                revoked: false,
            }
        );
        assert_eq!(
            setup_state_flags("awaiting_approval"),
            SetupStateFlags {
                setup_complete: false,
                awaiting_approval: true,
                revoked: false,
            }
        );
        assert_eq!(
            setup_state_flags("revoked"),
            SetupStateFlags {
                setup_complete: false,
                awaiting_approval: false,
                revoked: true,
            }
        );
        assert_eq!(
            primary_status_label("awaiting_approval"),
            "Waiting for approval"
        );
        assert_eq!(primary_status_label("revoked"), "Device removed");
        assert_eq!(sync_status_label("running"), "Sync on");
        assert_eq!(sync_status_label("profile synced"), "Profile synced");
        assert_eq!(sync_status_label("up to date"), "Up to date");
        assert_eq!(sync_status_label("ready"), "Ready");
        assert_eq!(sync_status_label(""), "Ready");
        assert_eq!(sync_status_label("paused"), "Sync paused");

        assert_eq!(app_actor_role_key(AppActorRole::Admin), "admin");
        assert_eq!(app_actor_role_label(AppActorRole::Member), "Member");
        assert_eq!(app_key_display_label(true, Some("Mac"), "npub1x"), "Mac");
        assert_eq!(
            app_key_display_label(false, Some("  Phone  "), "npub1x"),
            "Phone"
        );
        assert_eq!(
            app_key_display_label(false, Some("  "), "npub1x"),
            "Linked device"
        );
        assert_eq!(
            app_key_display_label(
                true,
                None,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            "This device"
        );

        let direct = app_key_connection_state(false, true, true, false);
        assert_eq!(direct, "direct");
        assert_eq!(
            app_key_connection_label(direct, Some("tcp"), Some(17)),
            "Online (TCP, 17 ms)"
        );
        assert_eq!(
            app_key_connection_label("mesh", None, None),
            "Online (Mesh)"
        );
        assert_eq!(app_key_connection_label("offline", None, None), "Offline");
        assert_eq!(app_key_connection_label("local", None, None), "This Device");

        let member = app_key_management_actions(true, false, false, 2);
        assert!(member.can_revoke);
        assert!(member.can_appoint_admin);
        assert!(!member.can_demote_admin);

        let peer_admin = app_key_management_actions(true, false, true, 2);
        assert!(peer_admin.can_revoke);
        assert!(!peer_admin.can_appoint_admin);
        assert!(peer_admin.can_demote_admin);

        let sole_admin = app_key_management_actions(true, false, true, 1);
        assert!(!sole_admin.can_demote_admin);

        let current = app_key_management_actions(true, true, true, 2);
        assert!(!current.can_revoke);
        assert!(!current.can_appoint_admin);
        assert!(!current.can_demote_admin);
    }

    #[test]
    fn shared_app_key_rows_include_presence_roles_and_actions() {
        let current = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let remote = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let current_npub = pubkey_npub(current);
        let remote_npub = pubkey_npub(remote);
        let app_actors = vec![
            crate::app_keys::AppActorEntry::admin(current.to_owned(), 10, Some("Mac".to_owned())),
            crate::app_keys::AppActorEntry::member(remote.to_owned(), 11, Some("Phone".to_owned())),
        ];
        let connectivity = AppKeyConnectivity {
            online_app_keys: [remote_npub.clone()].into_iter().collect(),
            direct_app_keys: [remote_npub.clone()].into_iter().collect(),
            peer_statuses: [(
                remote_npub.clone(),
                AppKeyConnectionDetails {
                    transport_type: Some("tcp".to_owned()),
                    srtt_ms: Some(12),
                },
            )]
            .into_iter()
            .collect(),
            ..AppKeyConnectivity::default()
        };

        let rows = app_key_roster_rows(&app_actors, current, true, true, &connectivity);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].npub, current_npub);
        assert_eq!(rows[0].display_label, "Mac");
        assert_eq!(rows[0].role, "admin");
        assert_eq!(rows[0].role_label, "Admin");
        assert_eq!(rows[0].connection_state, "local");
        assert_eq!(rows[0].connection_label, "This Device");
        assert!(!rows[0].can_revoke);

        assert_eq!(rows[1].npub, remote_npub);
        assert_eq!(rows[1].display_label, "Phone");
        assert_eq!(rows[1].role, "member");
        assert_eq!(rows[1].role_label, "Member");
        assert!(rows[1].is_online);
        assert!(rows[1].is_direct);
        assert_eq!(rows[1].online_via.as_deref(), Some("direct"));
        assert_eq!(rows[1].connection_state, "direct");
        assert_eq!(rows[1].connection_label, "Online (TCP, 12 ms)");
        assert!(rows[1].can_revoke);
        assert!(rows[1].can_appoint_admin);
        assert!(!rows[1].can_demote_admin);
    }

    #[test]
    fn shared_app_key_rows_do_not_promote_pubkeys_to_display_labels() {
        let current = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let remote = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let app_actors = vec![
            crate::app_keys::AppActorEntry::admin(current.to_owned(), 10, None),
            crate::app_keys::AppActorEntry::member(remote.to_owned(), 11, None),
        ];

        let rows = app_key_roster_rows(
            &app_actors,
            current,
            true,
            true,
            &AppKeyConnectivity::default(),
        );

        assert_eq!(rows[0].npub, pubkey_npub(current));
        assert_eq!(rows[0].display_label, "This device");
        assert_eq!(rows[1].npub, pubkey_npub(remote));
        assert_eq!(rows[1].display_label, "Linked device");
    }

    #[test]
    fn nostr_identity_summary_uses_profile_roster_projection() {
        let dir = tempdir().unwrap();
        let mut account = Profile::create(dir.path(), Some("Native".to_owned())).unwrap();
        let profile_id = account.state.profile_id.to_string();
        let current_app_key = account.state.app_key_pubkey.clone();
        let remote = nostr_sdk::Keys::generate().public_key().to_hex();
        account
            .approve_app_key(&remote, Some("Web".to_owned()))
            .expect("approve app key");
        let latest_created_at = account
            .state
            .profile_roster_ops
            .iter()
            .map(|op| op.content.created_at)
            .max()
            .unwrap_or(0);
        let incomplete_epoch_event = crate::build_nostr_identity_roster_op_event(
            account.app_key.keys(),
            account.state.profile_id,
            crate::nostr_identity_roster_parent_ids(&account.state.profile_roster_ops),
            None,
            crate::NostrIdentityRosterOp::RotateSecretEpoch {
                epoch: 3,
                wrapped_secrets: [(current_app_key.clone(), "wrap-current".to_owned())]
                    .into_iter()
                    .collect(),
            },
            latest_created_at + 1,
        )
        .unwrap();
        account
            .state
            .profile_roster_ops
            .push(crate::parse_nostr_identity_roster_op_event(&incomplete_epoch_event).unwrap());
        account.state.sync_app_keys_from_profile();
        account.state.recompute_authorization();

        let summary = nostr_identity_summary(&account.state);

        assert_eq!(summary.profile_id, profile_id);
        assert_eq!(summary.current_app_key_pubkey_hex, current_app_key);
        assert_eq!(summary.current_app_key_label.as_deref(), Some("Native"));
        assert_eq!(summary.authorization_state, "authorized");
        assert!(summary.can_write_roots);
        assert!(summary.can_admin_profile);
        assert_eq!(summary.active_app_key_count, 2);
        assert_eq!(summary.current_key_epoch, Some(3));
        assert_eq!(summary.recovery_phrase_facet_count, 0);
        assert_eq!(summary.nip46_facet_count, 0);
        assert_eq!(summary.social_profile_facet_count, 0);
        assert_eq!(summary.missing_key_wrap_npubs.len(), 1);
        assert!(
            summary
                .missing_key_wrap_npubs
                .contains(&pubkey_npub(&remote))
        );
    }
}
