pub mod app_key_link_invite;
pub mod app_key_link_transport;
pub mod app_key_summary;
pub mod app_keys;
mod atomic_file;
pub mod backup_ops;
pub mod backup_summary;
pub mod block_sync;
pub mod blossom_sync;
pub mod calendar;
pub mod config;
pub mod config_lock;
pub mod daemon_liveness;
pub mod device_labels;
pub mod direct_root_transport;
mod fips_bootstrap;
pub mod fips_status;
pub mod fips_sync;
pub mod profile;

/// Convenience constructor: a `BlossomClient` wired with the given
/// signing keys and the given server URLs. Used as both the write and
/// read pool — single profile installs typically use the same servers
/// for both.
#[must_use]
pub fn blossom_sync_client(
    keys: nostr_sdk::Keys,
    servers: &[String],
) -> hashtree_blossom::BlossomClient {
    let client = hashtree_blossom::BlossomClient::new_empty(keys);
    let mut read_servers = client.read_servers().to_vec();
    for server in servers {
        if !read_servers.iter().any(|candidate| candidate == server) {
            read_servers.push(server.clone());
        }
    }

    client
        .with_read_servers(read_servers)
        .with_write_servers(servers.to_vec())
}
pub mod conflict;
pub mod daemon;
pub mod gateway;
pub mod history;
pub mod identity;
pub mod indexer;
pub mod link_input;
pub mod merge;
pub mod network_sync;
pub mod nostr_events;
pub mod nostr_identity;
pub mod paths;
pub mod projection;
pub mod provider;
pub mod recovery_phrase;
pub mod relay_config;
mod relay_filters;
pub mod relay_status;
pub mod relay_sync;
pub mod root_meta;
pub mod share_actions;
pub mod sharing;
pub mod sync;
pub mod sync_cache;
pub mod update_announcement;
pub mod updater;

pub use app_keys::{AppActorEntry, AppActorRole, AppKeysProjection, ApplyDecision};
pub use config::{
    AppConfig, AppKeyRootRef, BackupTarget, BackupTargetCheck, BackupTargetKind, BackupTargetSync,
    ConfigError, Drive, DriveRole, UserProfile,
};
pub use conflict::{
    ConflictDeletedSide, ConflictRecord, ConflictSide, ConflictState, FileSnapshot, SyncAction,
    conflict_filename, conflict_records_from_merge, resolve as resolve_conflict,
};
pub use daemon::{Daemon, DaemonError, ImportReport, PRIMARY_DRIVE_ID};
pub use direct_root_transport::{
    DIRECT_ROOT_APP_TOPIC, DirectRootEvent, DirectRootExchange, DirectRootFrame,
    DirectRootHintApply, DirectRootHintApplyReport, DirectRootHintFrame, DirectRootHintScope,
    DirectRootKeyHint, DirectRootStateRequestFrame, DirectRootWireFrame, apply_direct_root_event,
    apply_direct_root_key_hint_to_config, build_current_direct_root_events,
    coalesce_direct_root_app_messages, decode_direct_root_wire_frame,
    encode_direct_root_hint_frame, encode_direct_root_state_request_frame,
    parse_direct_root_key_hint,
};
pub use fips_sync::{
    FipsAppMessage, FipsBlockSync, FipsNostrPubsubEvent, FipsPeerStatus, FipsRelayStatus,
    FipsSyncError, FsFipsBlockSync,
};
pub use gateway::{GatewayBind, GatewayError, GatewayProxyServer, GatewayServer};
pub use identity::{AppKey, Identity, IdentityError, RecoveryKey};
pub use indexer::{
    IndexError, filter_ignored_entries_from_root, index_dir, layer_conflict_records,
    path_has_ignored_component, read_conflict_records, should_ignore_name,
};
pub use link_input::{
    AppKeyLinkTarget, LinkInputClassification, classify_link_input, normalize_app_key_pubkey,
    resolve_app_key_link_target,
};
pub use merge::{
    AppKeyFileEntry, AppKeySnapshot, AppKeyTombstone, CONFLICTS_PREFIX, META_DIR, MergedConflict,
    MergedConflictFile, MergedConflictKind, MergedConflictTombstone, MergedEntry, MergedView,
    ROOT_META_PATH, TOMBSTONE_PREFIX, WHOLE_FILE_HASH_META_KEY, merge_drives,
    original_path_from_tombstone, tombstone_path,
};
pub use network_sync::{
    DriveRootEventApplyReport, NetworkSyncOptions, NetworkSyncReport, apply_drive_root_events,
    authorized_app_key_pubkeys, drive_root_app_key_can_write_roots,
    drive_root_recipient_app_key_pubkeys, drive_root_writer_app_key_pubkeys,
    sync_once as network_sync_once, sync_once_with_fips, sync_once_with_options,
};
pub use nostr_identity::{
    KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE, KIND_NOSTR_IDENTITY_ROSTER_OP,
    NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA, NOSTR_IDENTITY_ROSTER_SCHEMA,
    NostrIdentityCapabilities, NostrIdentityError, NostrIdentityFacet,
    NostrIdentityFacetAcceptanceContent, NostrIdentityId, NostrIdentityKeyPurpose,
    NostrIdentityRosterLog, NostrIdentityRosterOp, NostrIdentityRosterOpContent,
    NostrIdentityRosterProjection, NostrIdentitySecretEpoch, NostrIdentityTombstone,
    SecretWrapStatus, SignedNostrIdentityFacetAcceptance, SignedNostrIdentityRosterOp,
    build_nostr_identity_facet_acceptance_event, build_nostr_identity_roster_op_event,
    is_nostr_identity_facet_acceptance_event_coordinate,
    is_nostr_identity_roster_op_event_coordinate,
    nostr_identity_candidate_ids_for_pubkey_from_events, nostr_identity_facet_acceptance_d_tag,
    nostr_identity_ids_from_facet_acceptances, nostr_identity_roster_op_d_tag,
    nostr_identity_roster_parent_ids, nostr_identity_tag_kind,
    parse_nostr_identity_facet_acceptance_event, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster,
};
pub use profile::{
    AppKeyAuthorizationState, Profile, ProfileError, ProfileLogoutReport, ProfileState,
    SecretWrapRepairOutcome, app_key_link_invite_keys, app_key_link_invite_pubkey,
    logout_local_profile,
};
pub use projection::{
    PrimaryMergedRoot, PrimaryMergedView, ProjectionError, primary_merged_root,
    primary_merged_root_from_view, primary_merged_view,
};
pub use root_meta::{DriveRootMeta, RootObservation, RootParent};
pub use share_actions::{
    ShareAction, ShareActionResult, dispatch_share_action, repair_missing_share_shortcuts,
    share_action_state,
};
pub use sharing::{
    KIND_SHARE_ACCESS_SNAPSHOT, PendingShareInvite, PendingShareInviteView, ResolvedShareRecipient,
    SHARE_ACCESS_LABEL, SHARE_ACCESS_SNAPSHOT_SCHEMA, SHARE_INVITE_PREFIX, SHARE_INVITE_SCHEMA,
    SHARED_WITH_ME_DIR, ShareAccessDevice, ShareAccessGrant, ShareAccessProjection,
    ShareAccessSnapshot, ShareAccessTarget, ShareInviteBundle, ShareInviteOutcome,
    ShareKeyRepairOutcome, ShareMember, ShareMemberRevokeOutcome, ShareMemberRoleOutcome,
    ShareMemberStatus, ShareRecipient, ShareRecipientProfileEvidence, ShareRole,
    ShareRootWriteAuthorization, ShareShortcut, SharedFolder, SharedFolderKeyStatus,
    SharedFolderMemberView, SharedFolderView, SharingError, SignedShareAccessSnapshot,
    create_shared_folder, current_shared_folder_key, default_share_shortcut_path,
    encode_share_invite, invite_shared_folder_member, invite_shared_folder_resolved_recipient,
    is_share_access_snapshot_event_coordinate, parse_share_access_snapshot_event,
    parse_share_invite, project_share_access, record_pending_share_invite,
    refresh_shared_folder_member_statuses_from_access, repair_shared_folder_key_epoch_wraps,
    resolve_share_recipient_from_evidence, resolve_share_recipient_from_profile_evidence,
    revoke_shared_folder_member, set_shared_folder_member_role, share_access_snapshot_d_tag,
    share_recipient_profile_evidence_for_app_key, shared_folder_app_key_can_admin,
    shared_folder_app_key_can_write_roots, shared_folder_app_key_write_authorization,
    shared_folder_authorized_writer_pubkeys, shared_folder_from_invite_for_profile,
    shared_folder_key_recipient_pubkeys, shared_folder_missing_key_wrap_pubkeys,
    shared_folder_view, shared_folder_views, shared_with_me_path, sign_share_access_snapshot,
    validate_signed_share_access_snapshot,
};
pub use sync::{
    ConflictResolution, SyncBaseState, SyncError, SyncReport, sync as run_sync, sync_with_base,
    sync_with_base_anchor, sync_with_cache,
};
pub use sync_cache::{
    CachedBaseAnchor, CachedBaseState, CachedPathState, CachedRoot, ContentNeed, RetrievalOutcome,
    SOURCE_STATE_AVAILABLE, SOURCE_STATE_MISSING, SOURCE_STATE_POISONED, SOURCE_STATE_UNKNOWN,
    SourceAvailability, SyncCache, SyncCacheError,
};
pub use update_announcement::UpdateAnnouncementExchange;

/// Schema version for the iris-drive config file. Bump when fields are
/// removed or repurposed so older installs fail closed instead of carrying
/// stale state forward.
///
/// v2: added optional `ProfileState` for the recovery/app-key split + `AppKeys`.
/// v3: removed plain working-directory mode; configs are strict.
/// v4: renamed the persisted local identity field from `account` to `profile`.
/// v5: replaced shared-folder roster/member op logs with canonical access snapshots.
pub const CONFIG_SCHEMA_VERSION: u32 = 5;

#[cfg(test)]
mod tests {
    use nostr_identity::IdentityRosterOp;
    use nostr_sdk::Keys;

    #[test]
    fn blossom_sync_client_preserves_configured_writes_as_reads_too() {
        let write_server = "https://write.example".to_string();
        let client =
            super::blossom_sync_client(Keys::generate(), std::slice::from_ref(&write_server));

        assert_eq!(client.write_servers(), std::slice::from_ref(&write_server));
        assert_eq!(client.read_servers(), std::slice::from_ref(&write_server));
    }

    #[test]
    fn nostr_identity_app_key_labels_are_not_written_to_identity_roster_events() {
        let keys = Keys::generate();
        let profile_id = super::NostrIdentityId::new_v4();
        let event = super::build_nostr_identity_roster_op_event(
            &keys,
            profile_id,
            Vec::new(),
            None,
            super::NostrIdentityRosterOp::AddFacet {
                facet: super::NostrIdentityFacet::app_key(
                    keys.public_key().to_hex(),
                    1,
                    Some("Pixel".to_owned()),
                    super::NostrIdentityCapabilities::app_admin(),
                ),
            },
            1,
        )
        .unwrap();

        let signed = nostr_identity::parse_identity_roster_op_event(&event).unwrap();
        let IdentityRosterOp::AddKey { key } = signed.content.op else {
            panic!("expected add-key profile op");
        };

        assert_eq!(key.label, None);
    }
}
