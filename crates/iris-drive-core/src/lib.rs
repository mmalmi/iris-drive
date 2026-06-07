pub mod app_key_link_invite;
pub mod app_key_link_transport;
pub mod app_key_summary;
pub mod app_keys;
pub mod backup_ops;
pub mod backup_summary;
pub mod block_sync;
pub mod blossom_sync;
pub mod config;
pub mod direct_root_transport;
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
pub mod iris_profile;
pub mod link_input;
pub mod merge;
pub mod network_sync;
pub mod nostr_events;
pub mod paths;
pub mod projection;
pub mod provider;
pub mod recovery_phrase;
pub mod relay_config;
pub mod relay_status;
pub mod relay_sync;
pub mod root_meta;
pub mod share_actions;
pub mod sharing;
pub mod sync;
pub mod sync_cache;
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
    DIRECT_ROOT_APP_TOPIC, DIRECT_ROOT_MESH_STREAM_PREFIX, DirectRootEvent, DirectRootExchange,
    DirectRootFrame, apply_direct_root_event, build_current_direct_root_events,
    direct_root_mesh_stream,
};
pub use fips_sync::{FipsBlockSync, FipsSyncError, FsFipsBlockSync};
pub use gateway::{GatewayBind, GatewayError, GatewayServer};
pub use hashtree_fips_transport::FipsAppMessage;
pub use identity::{AppKey, Identity, IdentityError, RecoveryKey};
pub use indexer::{
    IndexError, filter_ignored_entries_from_root, index_dir, layer_conflict_records,
    path_has_ignored_component, read_conflict_records, should_ignore_name,
};
pub use iris_profile::{
    IRIS_PROFILE_FACET_ACCEPTANCE_SCHEMA, IRIS_PROFILE_ROSTER_SCHEMA, IrisProfileCapabilities,
    IrisProfileError, IrisProfileFacet, IrisProfileFacetAcceptanceContent, IrisProfileId,
    IrisProfileKeyEpoch, IrisProfileKeyPurpose, IrisProfileRosterLog, IrisProfileRosterOp,
    IrisProfileRosterOpContent, IrisProfileRosterProjection, IrisProfileTombstone,
    KIND_IRIS_PROFILE_FACET_ACCEPTANCE, KIND_IRIS_PROFILE_ROSTER_OP, KeyWrapStatus,
    SignedIrisProfileFacetAcceptance, SignedIrisProfileRosterOp,
    build_iris_profile_facet_acceptance_event, build_iris_profile_roster_op_event,
    iris_profile_candidate_ids_for_pubkey_from_events, iris_profile_facet_acceptance_d_tag,
    iris_profile_ids_from_facet_acceptances, iris_profile_roster_op_d_tag,
    iris_profile_roster_parent_ids, iris_profile_tag_kind,
    is_iris_profile_facet_acceptance_event_coordinate, is_iris_profile_roster_op_event_coordinate,
    parse_iris_profile_facet_acceptance_event, parse_iris_profile_roster_op_event,
    project_iris_profile_roster,
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
    DriveRootEventApplyReport, NetworkSyncReport, apply_drive_root_events,
    authorized_app_key_pubkeys, sync_once as network_sync_once, sync_once_with_fips,
};
pub use profile::{
    AppKeyAuthorizationState, KeyWrapRepairOutcome, Profile, ProfileError, ProfileLogoutReport,
    ProfileState, logout_local_profile,
};
pub use projection::{
    PrimaryMergedRoot, PrimaryMergedView, ProjectionError, primary_merged_root, primary_merged_view,
};
pub use root_meta::{DriveRootMeta, RootObservation, RootParent};
pub use share_actions::{
    ShareAction, ShareActionResult, dispatch_share_action, repair_missing_share_shortcuts,
    share_action_state,
};
pub use sharing::{
    KIND_SHARE_ROSTER_CHECKPOINT, PendingShareInvite, PendingShareInviteView,
    ResolvedShareRecipient, SHARE_INVITE_PREFIX, SHARE_INVITE_SCHEMA,
    SHARE_ROSTER_CHECKPOINT_SCHEMA, SHARED_WITH_ME_DIR, ShareInviteBundle, ShareInviteOutcome,
    ShareKeyRepairOutcome, ShareMember, ShareMemberRevokeOutcome, ShareMemberRoleOutcome,
    ShareMemberStatus, ShareRecipient, ShareRecipientProfileEvidence, ShareRole,
    ShareRootWriteAuthorization, ShareRosterCheckpointContent, ShareShortcut, SharedFolder,
    SharedFolderKeyStatus, SharedFolderMemberView, SharedFolderView, SharingError,
    SignedShareRosterCheckpoint, create_shared_folder, current_shared_folder_key,
    default_share_shortcut_path, encode_share_invite, invite_shared_folder_member,
    invite_shared_folder_resolved_recipient, parse_share_invite,
    parse_share_roster_checkpoint_event, record_pending_share_invite,
    refresh_shared_folder_member_statuses_from_roster, repair_shared_folder_key_epoch_wraps,
    resolve_share_recipient_from_evidence, resolve_share_recipient_from_profile_evidence,
    revoke_shared_folder_member, set_shared_folder_member_role,
    share_recipient_profile_evidence_for_app_key, shared_folder_app_key_can_admin,
    shared_folder_app_key_can_write_roots, shared_folder_app_key_write_authorization,
    shared_folder_authorized_writer_pubkeys, shared_folder_from_invite_for_profile,
    shared_folder_key_recipient_pubkeys, shared_folder_missing_key_wrap_pubkeys,
    shared_folder_view, shared_folder_views, shared_with_me_path, sign_share_roster_checkpoint,
    validate_share_roster_checkpoint,
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

/// Schema version for the iris-drive config file. Bump when fields are
/// removed or repurposed so older installs fail closed instead of carrying
/// stale state forward.
///
/// v2: added optional `ProfileState` for the recovery/app-key split + `AppKeys`.
/// v3: removed plain working-directory mode; configs are strict.
/// v4: renamed the persisted local identity field from `account` to `profile`.
pub const CONFIG_SCHEMA_VERSION: u32 = 4;

#[cfg(test)]
mod tests {
    use nostr_sdk::Keys;

    #[test]
    fn blossom_sync_client_preserves_configured_writes_as_reads_too() {
        let write_server = "https://write.example".to_string();
        let client =
            super::blossom_sync_client(Keys::generate(), std::slice::from_ref(&write_server));

        assert_eq!(client.write_servers(), std::slice::from_ref(&write_server));
        assert_eq!(client.read_servers(), std::slice::from_ref(&write_server));
    }
}
