use super::{
    FfiApp, NATIVE_RUNTIME_CONFIG_CACHE, SentAppKeyLinkRequest, app_key_link_request_send_due,
    load_native_runtime_config_cached, native_calendar_export_json, normalize_pubkey,
};
use crate::NativeAppAction;
use crate::state::{UiShare, UiShareMember};
use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, AppKeyRootRef, Drive};
use std::path::Path;

fn share_recipient_evidence_json(config_dir: &Path, display_name: &str) -> String {
    let config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    let state = config.profile.unwrap();
    let account = iris_drive_core::Profile::load(state, config_dir).unwrap();
    let acceptance_event = iris_drive_core::build_nostr_identity_facet_acceptance_event(
        account.app_key.keys(),
        account.state.profile_id,
        [iris_drive_core::NostrIdentityKeyPurpose::AppKey],
        account
            .state
            .profile_roster_ops
            .first()
            .map(|op| op.op_id.clone()),
        20,
    )
    .unwrap();
    let evidence = iris_drive_core::ShareRecipientProfileEvidence {
        profile_id: account.state.profile_id,
        representative_pubkey: Some(account.state.app_key_pubkey.clone()),
        representative_npub: None,
        display_name: Some(display_name.to_owned()),
        roster_ops: account.state.profile_roster_ops,
        acceptances: vec![
            iris_drive_core::parse_nostr_identity_facet_acceptance_event(&acceptance_event)
                .unwrap(),
        ],
    };
    serde_json::to_string(&evidence).unwrap()
}

#[test]
fn native_runtime_config_cache_reuses_unchanged_config_and_invalidates_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = config_path_in(dir.path());
    let first_config = AppConfig {
        relays: vec!["wss://first.example".to_owned()],
        ..AppConfig::default()
    };
    first_config.save(&config_path).unwrap();
    NATIVE_RUNTIME_CONFIG_CACHE.lock().unwrap().clear();

    let first = load_native_runtime_config_cached(&config_path).unwrap();
    assert_eq!(first.relays, vec!["wss://first.example"]);

    NATIVE_RUNTIME_CONFIG_CACHE
        .lock()
        .unwrap()
        .get_mut(&config_path)
        .unwrap()
        .config
        .relays = vec!["cached".to_owned()];
    let cached = load_native_runtime_config_cached(&config_path).unwrap();
    assert_eq!(cached.relays, vec!["cached"]);

    let changed_config = AppConfig {
        relays: vec!["wss://changed.example".to_owned()],
        ..AppConfig::default()
    };
    changed_config.save(&config_path).unwrap();

    let refreshed = load_native_runtime_config_cached(&config_path).unwrap();
    assert_eq!(refreshed.relays, vec!["wss://changed.example"]);
}

#[test]
fn native_calendar_export_returns_default_calendar_json() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let state = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Android calendar test".to_owned(),
    });
    assert!(state.error.is_empty(), "{}", state.error);

    let export = native_calendar_export_json(&dir.path().display().to_string());

    assert_eq!(export["error"], "");
    assert_eq!(export["calendar"]["title"], "Calendar");
    assert_eq!(
        export["calendar"]["events"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(
        export["calendar"]["ownerNpub"]
            .as_str()
            .is_some_and(|owner| owner.starts_with("npub1")),
        "{export}",
    );
}

fn ui_share_member<'a>(share: &'a UiShare, profile_id: &str) -> &'a UiShareMember {
    share
        .members
        .iter()
        .find(|member| member.profile_id == profile_id)
        .unwrap()
}

fn remove_share_wrap_for_epoch(
    folder: &mut iris_drive_core::SharedFolder,
    epoch: u64,
    removed_pubkey: &str,
) {
    folder
        .access
        .key_epochs
        .get_mut(&epoch)
        .unwrap_or_else(|| panic!("share key epoch {epoch} not found"))
        .wrapped_secrets
        .remove(removed_pubkey);
}

#[test]
fn app_runtime_installs_rustls_crypto_provider() {
    let dir = tempfile::tempdir().unwrap();
    let _app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn dispatch_adds_updates_and_removes_roots() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: "My Drive".to_owned(),
        local_path: "/virtual/iris".to_owned(),
    });
    assert_eq!(state.ui.roots.len(), 1);
    assert_eq!(state.ui.roots[0].name, "My Drive");
    assert!(state.error.is_empty());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: "My Drive".to_owned(),
        local_path: "/virtual/iris-renamed".to_owned(),
    });
    assert_eq!(state.ui.roots.len(), 1);
    assert_eq!(state.ui.roots[0].local_path, "/virtual/iris-renamed");

    let state = app.dispatch(NativeAppAction::RemoveRoot {
        name: "My Drive".to_owned(),
    });
    assert!(state.ui.roots.is_empty());
    assert!(state.error.is_empty());
}

#[test]
fn dispatch_rejects_empty_roots() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: String::new(),
        local_path: "/virtual/iris".to_owned(),
    });

    assert!(state.ui.roots.is_empty());
    assert_eq!(state.error, "root name is required");
}

#[test]
fn profile_actions_populate_mobile_parity_state() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });

    let account = state.ui.profile.as_ref().expect("account exists");
    assert_eq!(account.app_key_label, "Pixel");
    assert_eq!(account.authorization_state, "authorized");
    assert!(account.current_app_key_npub.starts_with("npub1"));
    assert_eq!(account.current_app_key_label, "Pixel");
    assert!(account.can_admin_profile);
    assert!(account.can_write_roots);
    assert!(!account.can_export_recovery_phrase);
    assert_eq!(account.active_app_key_count, 1);
    assert_eq!(account.profile_roster_op_count, 2);
    assert_eq!(account.current_key_epoch, Some(1));
    assert_eq!(account.recovery_phrase_facet_count, 0);
    assert_eq!(account.nip46_facet_count, 0);
    assert_eq!(account.social_profile_facet_count, 0);
    assert!(account.missing_key_wraps.is_empty());
    assert!(account.app_key_link_request.is_empty());
    assert!(
        account
            .app_key_link_invite
            .starts_with("https://drive.iris.to/invite/")
    );
    assert!(!account.app_key_link_invite.contains("local-owner"));
    assert!(!account.app_key_link_invite.contains("device-"));
    assert_eq!(state.ui.app_actors.len(), 1);
    assert_eq!(state.ui.app_actors[0].actor_kind, "device");
    assert_eq!(state.ui.app_actors[0].label, "Pixel");
    assert_eq!(state.ui.app_actors[0].display_label, "Pixel");
    assert_eq!(state.ui.app_actors[0].role, "admin");
    assert_eq!(state.ui.app_actors[0].role_label, "Admin");
    assert_eq!(state.ui.app_actors[0].state_label, "Linked");
    assert_eq!(state.ui.app_actors[0].connection_state, "local");
    assert_eq!(state.ui.app_actors[0].connection_label, "This Device");
    assert!(state.ui.snapshot_link.is_empty());
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "running");
    assert_eq!(state.ui.sync.status_label, "Sync on");
    assert!(!state.ui.relays.is_empty());
    assert_eq!(state.ui.relay_statuses.len(), state.ui.relays.len());
    assert_eq!(state.ui.relay_statuses[0].status_label, "saved");
    assert_eq!(state.ui.relay_statuses[0].health, "configured");
    assert!(!state.ui.backups.is_empty());
    assert_eq!(state.ui.backups[0].label, "upload.iris.to");
    assert_eq!(state.ui.paths.data_dir, dir.path().display().to_string());
    assert_eq!(state.ui.setup_state, "authorized");
    assert!(state.ui.setup_complete);
    assert!(!state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.setup_label, "Linked");
    assert_eq!(state.ui.primary_status, "ready");
    assert_eq!(state.ui.primary_status_label, "Ready");
    assert_eq!(state.ui.authorized_app_key_count, 1);
    assert_eq!(state.ui.online_app_key_count, 0);
    assert!(state.ui.shares.is_empty());
    let export = super::export_recovery_secret(dir.path().display().to_string());
    assert!(!export.can_export);
    assert!(export.words.is_empty());
    assert!(export.recovery_phrase.is_empty());
    assert!(
        export.error.contains("loading recovery phrase"),
        "{}",
        export.error
    );

    let state = app.dispatch(NativeAppAction::StartSync);
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "up to date");
    assert_eq!(state.ui.sync.status_label, "Up to date");

    let state = app.dispatch(NativeAppAction::StopSync);
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "paused");
    assert_eq!(state.ui.sync.status_label, "Sync paused");
}

#[test]
fn add_recovery_key_saves_only_generated_public_key_to_devices_view() {
    let dir = tempfile::tempdir().unwrap();
    let generated = super::generate_recovery_key();
    assert!(generated.error.is_empty(), "{}", generated.error);
    assert_eq!(generated.words.len(), 12);
    assert!(generated.recovery_pubkey.starts_with("npub1"));
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });
    assert!(created.error.is_empty(), "{}", created.error);
    let created_account = created.ui.profile.as_ref().expect("profile exists");
    assert_eq!(created_account.recovery_phrase_facet_count, 0);
    assert!(!created_account.can_export_recovery_phrase);
    assert_eq!(created.ui.app_actors.len(), 1);

    let updated = app.dispatch(NativeAppAction::AddRecoveryDevice {
        recovery_pubkey: generated.recovery_pubkey.clone(),
    });
    assert!(updated.error.is_empty(), "{}", updated.error);
    let account = updated.ui.profile.as_ref().expect("profile exists");
    assert_eq!(account.active_app_key_count, 1);
    assert_eq!(account.recovery_phrase_facet_count, 1);
    assert!(!account.can_export_recovery_phrase);
    assert_eq!(account.profile_roster_op_count, 4);
    assert_eq!(updated.ui.authorized_app_key_count, 1);
    assert_eq!(updated.ui.online_app_key_count, 0);
    assert!(!dir.path().join("recovery_phrase").exists());

    let recovery_devices = updated
        .ui
        .app_actors
        .iter()
        .filter(|device| device.role == "recovery")
        .collect::<Vec<_>>();
    assert_eq!(recovery_devices.len(), 1);
    assert_eq!(recovery_devices[0].actor_kind, "recovery_key");
    assert_eq!(recovery_devices[0].display_label, "Recovery key");
    assert_eq!(recovery_devices[0].connection_label, "Recovery key");
    assert!(!recovery_devices[0].is_online);
    assert!(!recovery_devices[0].can_revoke);

    let export = super::export_recovery_secret(dir.path().display().to_string());
    assert!(!export.can_export);
    assert!(export.recovery_phrase.is_empty());
    assert!(
        export.error.contains("loading recovery phrase"),
        "{}",
        export.error
    );

    let repeated = app.dispatch(NativeAppAction::AddRecoveryDevice {
        recovery_pubkey: generated.recovery_pubkey,
    });
    assert!(repeated.error.is_empty(), "{}", repeated.error);
    let repeated_account = repeated.ui.profile.as_ref().expect("profile exists");
    assert_eq!(repeated_account.recovery_phrase_facet_count, 1);
    assert_eq!(repeated_account.profile_roster_op_count, 4);
}

#[test]
fn import_recovery_key_derives_public_key_without_returning_words() {
    let phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let imported = super::recovery_pubkey_for_phrase(phrase);

    assert!(imported.error.is_empty(), "{}", imported.error);
    assert!(imported.recovery_pubkey.starts_with("npub1"));
    assert!(imported.words.is_empty());

    let invalid = super::recovery_pubkey_for_phrase("not enough words".to_owned());
    assert!(!invalid.error.is_empty());
    assert!(invalid.recovery_pubkey.is_empty());
    assert!(invalid.words.is_empty());
}

#[test]
fn app_state_surfaces_shared_with_me_rows_and_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    assert!(created.error.is_empty(), "{}", created.error);

    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let account = iris_drive_core::Profile::load(config.profile.clone().unwrap(), dir.path())
        .expect("account loads");
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".to_owned()),
        Vec::new(),
        10,
    )
    .unwrap();
    let shortcut =
        iris_drive_core::ShareShortcut::new(folder.share_id, "Projects/Alpha shared", "").unwrap();
    config.upsert_shared_folder(folder.clone());
    config.upsert_share_shortcut(shortcut);
    config.save(config_path_in(dir.path())).unwrap();

    let refreshed = app.refresh();

    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    assert_eq!(refreshed.ui.shares.len(), 1);
    let share = &refreshed.ui.shares[0];
    assert_eq!(share.share_id, folder.share_id.to_string());
    assert_eq!(share.display_name, "Alpha");
    assert_eq!(share.source_path, "Projects/Alpha");
    assert_eq!(share.shared_with_me_path, "Shared with me/Alpha");
    assert_eq!(share.role, "admin");
    assert_eq!(share.role_label, "Admin");
    assert_eq!(share.key_status, "available");
    assert_eq!(share.key_status_label, "Available");
    assert_eq!(share.write_authorization, "authorized");
    assert_eq!(share.write_authorization_label, "Authorized");
    assert!(share.can_write);
    assert!(share.can_admin);
    assert_eq!(share.current_key_epoch, Some(1));
    assert!(share.has_current_key_wrap);
    assert!(!share.key_unavailable);
    assert!(!share.repair_needed);
    assert_eq!(share.missing_key_wrap_count, 0);
    assert!(share.missing_key_wraps.is_empty());
    assert_eq!(share.participant_count, 1);
    assert_eq!(share.app_key_count, 1);
    assert_eq!(share.members.len(), 1);
    assert_eq!(
        share.members[0].profile_id,
        created.ui.profile.unwrap().profile_id
    );
    assert_eq!(share.members[0].display_name, "Mac");
    assert_eq!(share.members[0].role, "admin");
    assert_eq!(share.members[0].role_label, "Admin");
    assert_eq!(share.members[0].status, "active");
    assert_eq!(share.members[0].status_label, "Active");
    assert_eq!(share.members[0].app_key_count, 1);
    assert_eq!(share.shortcut_paths, vec!["Projects/Alpha shared"]);
}

#[test]
fn app_state_surfaces_share_missing_wrap_detail() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    assert!(created.error.is_empty(), "{}", created.error);

    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let account = iris_drive_core::Profile::load(config.profile.clone().unwrap(), dir.path())
        .expect("account loads");
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_pubkey = recipient_keys.public_key().to_hex();
    let mut folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".to_owned()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: iris_drive_core::NostrIdentityId::new_v4(),
            app_pubkey: recipient_pubkey.clone(),
            role: iris_drive_core::ShareRole::Reader,
            label: Some("Phone".to_owned()),
            representative_npub_hint: None,
            display_name: Some("Phone".to_owned()),
        }],
        10,
    )
    .unwrap();
    let current_epoch = folder
        .projection()
        .secret_epochs
        .keys()
        .next_back()
        .copied()
        .unwrap();
    remove_share_wrap_for_epoch(&mut folder, current_epoch, &recipient_pubkey);
    config.upsert_shared_folder(folder);
    config.save(config_path_in(dir.path())).unwrap();

    let refreshed = app.refresh();

    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    let share = refreshed.ui.shares.first().unwrap();
    assert_eq!(share.key_status, "repair_needed");
    assert!(share.repair_needed);
    assert_eq!(share.missing_key_wrap_count, 1);
    assert_eq!(
        share.missing_key_wraps,
        vec![iris_drive_core::app_key_summary::pubkey_npub(
            &recipient_pubkey
        )]
    );
}

#[test]
fn app_actions_manage_share_invite_accept_shortcut_and_revoke() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_created = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    assert!(owner_created.error.is_empty(), "{}", owner_created.error);
    let owner_profile = owner_created.ui.profile.clone().unwrap();
    let created_share = owner_app.dispatch(NativeAppAction::CreateShare {
        source_path: "Projects/Alpha".to_owned(),
        display_name: "Alpha".to_owned(),
    });
    assert!(created_share.error.is_empty(), "{}", created_share.error);
    assert_eq!(created_share.ui.shares.len(), 1);
    assert_eq!(created_share.ui.shares[0].shortcut_paths, vec!["Alpha"]);
    let share_id = created_share.ui.shares[0].share_id.clone();

    let recipient_dir = tempfile::tempdir().unwrap();
    let recipient_app = FfiApp::new(
        recipient_dir.path().display().to_string(),
        "test".to_owned(),
    );
    let recipient_created = recipient_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Recipient".to_owned(),
    });
    assert!(
        recipient_created.error.is_empty(),
        "{}",
        recipient_created.error
    );
    let recipient_profile = recipient_created.ui.profile.clone().unwrap();

    let invited = owner_app.dispatch(NativeAppAction::InviteShareMember {
        share_id: share_id.clone(),
        profile_id: recipient_profile.profile_id.clone(),
        app_key: recipient_profile.current_app_key_pubkey.clone(),
        role: "reader".to_owned(),
        representative_npub_hint: "npub1alice".to_owned(),
        display_name: "Alice".to_owned(),
        label: "Recipient".to_owned(),
    });
    assert!(invited.error.is_empty(), "{}", invited.error);
    let invited_owner_share = invited.ui.shares.first().unwrap();
    let recipient_member = ui_share_member(invited_owner_share, &recipient_profile.profile_id);
    assert!(recipient_member.can_revoke);
    assert!(recipient_member.can_change_role);
    let owner_member = ui_share_member(invited_owner_share, &owner_profile.profile_id);
    assert!(!owner_member.can_revoke);
    assert!(!owner_member.can_change_role);
    assert!(
        invited
            .ui
            .last_share_invite
            .starts_with(iris_drive_core::SHARE_INVITE_PREFIX)
    );

    let accepted = recipient_app.dispatch(NativeAppAction::AcceptShareInvite {
        invite: invited.ui.last_share_invite.clone(),
    });
    assert!(accepted.error.is_empty(), "{}", accepted.error);
    assert_eq!(accepted.ui.shares.len(), 1);
    assert_eq!(accepted.ui.shares[0].share_id, share_id);
    assert_eq!(accepted.ui.shares[0].role, "reader");
    assert_eq!(accepted.ui.shares[0].members.len(), 2);
    assert_eq!(accepted.ui.shares[0].shortcut_paths, vec!["Alpha"]);

    let promoted = owner_app.dispatch(NativeAppAction::SetShareMemberRole {
        share_id: share_id.clone(),
        profile_id: recipient_profile.profile_id.clone(),
        role: "editor".to_owned(),
    });
    assert!(promoted.error.is_empty(), "{}", promoted.error);
    assert_eq!(
        ui_share_member(
            promoted.ui.shares.first().unwrap(),
            &recipient_profile.profile_id
        )
        .role,
        "editor"
    );

    let shortcut = recipient_app.dispatch(NativeAppAction::AddShareShortcut {
        share_id: share_id.clone(),
        path: String::new(),
        parent: "Projects".to_owned(),
        target_path: String::new(),
    });
    assert!(shortcut.error.is_empty(), "{}", shortcut.error);
    assert_eq!(
        shortcut.ui.shares[0].shortcut_paths,
        vec!["Alpha", "Projects/Alpha"]
    );

    let revoked = owner_app.dispatch(NativeAppAction::RevokeShareMember {
        share_id,
        profile_id: recipient_profile.profile_id.clone(),
        reason: "removed".to_owned(),
    });
    assert!(revoked.error.is_empty(), "{}", revoked.error);
    let owner_share = revoked.ui.shares.first().unwrap();
    assert_eq!(
        ui_share_member(owner_share, &recipient_profile.profile_id).status,
        "revoked"
    );
}

#[test]
fn app_action_delete_share_removes_share_and_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    assert!(created.error.is_empty(), "{}", created.error);

    let shared = app.dispatch(NativeAppAction::CreateShare {
        source_path: "Projects/Alpha".to_owned(),
        display_name: "Alpha".to_owned(),
    });
    assert!(shared.error.is_empty(), "{}", shared.error);
    let share_id = shared.ui.shares[0].share_id.clone();

    let shortcut = app.dispatch(NativeAppAction::AddShareShortcut {
        share_id: share_id.clone(),
        path: "Projects/Alpha shared".to_owned(),
        parent: String::new(),
        target_path: String::new(),
    });
    assert!(shortcut.error.is_empty(), "{}", shortcut.error);
    assert_eq!(
        shortcut.ui.shares[0].shortcut_paths,
        vec!["Alpha", "Projects/Alpha shared"]
    );

    let deleted = app.dispatch(NativeAppAction::DeleteShare { share_id });
    assert!(deleted.error.is_empty(), "{}", deleted.error);
    assert!(deleted.ui.shares.is_empty());

    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(saved.shared_folders.is_empty());
    assert!(saved.share_shortcuts.is_empty());
}

#[test]
fn app_action_records_pending_share_invite_hint() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_created = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    assert!(owner_created.error.is_empty(), "{}", owner_created.error);
    let created_share = owner_app.dispatch(NativeAppAction::CreateShare {
        source_path: "Projects/Alpha".to_owned(),
        display_name: "Alpha".to_owned(),
    });
    assert!(created_share.error.is_empty(), "{}", created_share.error);
    let share_id = created_share.ui.shares[0].share_id.clone();
    let representative_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let representative_npub = iris_drive_core::app_key_summary::pubkey_npub(&representative_pubkey);

    let pending = owner_app.dispatch(NativeAppAction::RecordPendingShareInvite {
        share_id,
        representative_npub_hint: representative_npub.clone(),
        role: "reader".to_owned(),
        display_name: "Alice".to_owned(),
    });

    assert!(pending.error.is_empty(), "{}", pending.error);
    let share = pending.ui.shares.first().unwrap();
    assert_eq!(share.members.len(), 1);
    assert_eq!(share.pending_invites.len(), 1);
    let invite = &share.pending_invites[0];
    assert_eq!(invite.representative_npub_hint, representative_npub);
    assert_eq!(invite.display_name, "Alice");
    assert_eq!(invite.role, "reader");
    assert_eq!(invite.status, "pending");
}

#[test]
fn app_action_invites_share_member_from_recipient_evidence() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_created = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    assert!(owner_created.error.is_empty(), "{}", owner_created.error);
    let created_share = owner_app.dispatch(NativeAppAction::CreateShare {
        source_path: "Projects/Alpha".to_owned(),
        display_name: "Alpha".to_owned(),
    });
    assert!(created_share.error.is_empty(), "{}", created_share.error);
    let share_id = created_share.ui.shares[0].share_id.clone();

    let recipient_dir = tempfile::tempdir().unwrap();
    let recipient_app = FfiApp::new(
        recipient_dir.path().display().to_string(),
        "test".to_owned(),
    );
    let recipient_created = recipient_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Recipient".to_owned(),
    });
    assert!(
        recipient_created.error.is_empty(),
        "{}",
        recipient_created.error
    );
    let recipient_profile = recipient_created.ui.profile.clone().unwrap();
    let invited = owner_app.dispatch(NativeAppAction::InviteShareMemberFromEvidence {
        share_id,
        evidence_json: share_recipient_evidence_json(recipient_dir.path(), "Alice"),
        role: "editor".to_owned(),
        display_name: String::new(),
    });

    assert!(invited.error.is_empty(), "{}", invited.error);
    assert!(
        invited
            .ui
            .last_share_invite
            .starts_with(iris_drive_core::SHARE_INVITE_PREFIX)
    );
    let share = invited.ui.shares.first().unwrap();
    let alice = share
        .members
        .iter()
        .find(|member| member.profile_id == recipient_profile.profile_id)
        .unwrap();
    assert_eq!(alice.display_name, "Alice");
    assert_eq!(alice.role, "editor");
    assert_eq!(alice.app_key_count, 1);
}

#[test]
fn app_action_exports_share_recipient_evidence_for_core_invites() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_created = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    assert!(owner_created.error.is_empty(), "{}", owner_created.error);
    let created_share = owner_app.dispatch(NativeAppAction::CreateShare {
        source_path: "Projects/Alpha".to_owned(),
        display_name: "Alpha".to_owned(),
    });
    assert!(created_share.error.is_empty(), "{}", created_share.error);
    let share_id = created_share.ui.shares[0].share_id.clone();

    let recipient_dir = tempfile::tempdir().unwrap();
    let recipient_app = FfiApp::new(
        recipient_dir.path().display().to_string(),
        "test".to_owned(),
    );
    let recipient_created = recipient_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Recipient phone".to_owned(),
    });
    assert!(
        recipient_created.error.is_empty(),
        "{}",
        recipient_created.error
    );
    let recipient_profile = recipient_created.ui.profile.clone().unwrap();

    let exported = recipient_app.dispatch(NativeAppAction::ExportShareRecipientEvidence {
        display_name: "Alice".to_owned(),
    });
    assert!(exported.error.is_empty(), "{}", exported.error);
    assert!(!exported.ui.last_share_recipient_evidence.is_empty());
    let evidence: iris_drive_core::ShareRecipientProfileEvidence =
        serde_json::from_str(&exported.ui.last_share_recipient_evidence).unwrap();
    let resolved = iris_drive_core::resolve_share_recipient_from_evidence(&evidence, None).unwrap();
    assert_eq!(
        resolved.profile_id.to_string(),
        recipient_profile.profile_id
    );
    assert_eq!(resolved.display_name.as_deref(), Some("Alice"));
    assert_eq!(resolved.app_pubkeys.len(), 1);

    let invited = owner_app.dispatch(NativeAppAction::InviteShareMemberFromEvidence {
        share_id,
        evidence_json: exported.ui.last_share_recipient_evidence,
        role: "reader".to_owned(),
        display_name: String::new(),
    });
    assert!(invited.error.is_empty(), "{}", invited.error);
    let share = invited.ui.shares.first().unwrap();
    let alice = ui_share_member(share, &recipient_profile.profile_id);
    assert_eq!(alice.display_name, "Alice");
    assert_eq!(alice.role, "reader");
}

#[test]
fn recovery_phrase_export_restores_fresh_profile_without_roster_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let recovery_phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: recovery_phrase.clone(),
        app_key_label: "Pixel".to_owned(),
    });
    let created_account = created.ui.profile.as_ref().expect("account exists");
    assert!(created_account.can_export_recovery_phrase);

    let export = super::export_recovery_secret(dir.path().display().to_string());
    assert!(export.error.is_empty(), "{}", export.error);
    assert!(export.can_export);
    assert_eq!(export.words.len(), 12);
    assert_eq!(export.recovery_phrase.split_whitespace().count(), 12);
    assert_eq!(export.recovery_phrase, recovery_phrase);
    assert!(export.secret_key.starts_with("nsec1"));
    let secret_key = export.secret_key.clone();

    let restored_dir = tempfile::tempdir().unwrap();
    let restored_app = FfiApp::new(restored_dir.path().display().to_string(), "test".to_owned());
    let restored = restored_app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: recovery_phrase.clone(),
        app_key_label: "Restored".to_owned(),
    });

    assert!(restored.error.is_empty(), "{}", restored.error);
    let restored_account = restored.ui.profile.expect("restored account exists");
    assert_ne!(restored_account.profile_id, created_account.profile_id);
    assert_ne!(
        restored_account.current_app_key_npub,
        created_account.current_app_key_npub
    );
    assert!(restored_account.can_export_recovery_phrase);

    let restored_export = super::export_recovery_secret(restored_dir.path().display().to_string());
    assert!(
        restored_export.error.is_empty(),
        "{}",
        restored_export.error
    );
    assert_eq!(restored_export.words.len(), 12);
    assert_eq!(restored_export.recovery_phrase, recovery_phrase);
    assert_eq!(restored_export.secret_key, secret_key);
}

#[test]
fn saved_recovery_phrase_admits_restored_app_key_after_profile_log_sync() {
    let owner_dir = tempfile::tempdir().unwrap();
    let recovery_phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let created = owner_app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: recovery_phrase.clone(),
        app_key_label: "Native".to_owned(),
    });
    let created_account = created.ui.profile.expect("created account exists");
    let owner_config =
        AppConfig::load_or_default(config_path_in(owner_dir.path())).expect("owner config loads");
    let owner_state = owner_config.profile.expect("owner state exists");
    let export = super::export_recovery_secret(owner_dir.path().display().to_string());
    assert!(export.error.is_empty(), "{}", export.error);

    let restored_dir = tempfile::tempdir().unwrap();
    let restored_app = FfiApp::new(restored_dir.path().display().to_string(), "test".to_owned());
    let restored = restored_app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: export.recovery_phrase,
        app_key_label: "Browser".to_owned(),
    });
    assert!(restored.error.is_empty(), "{}", restored.error);
    let restored_account = restored.ui.profile.expect("restored account exists");

    let mut restored_config =
        AppConfig::load_or_default(config_path_in(restored_dir.path())).expect("config loads");
    let mut awaiting_state = restored_config.profile.take().expect("state exists");
    awaiting_state.profile_id = owner_state.profile_id;
    awaiting_state.profile_roster_ops = owner_state.profile_roster_ops.clone();
    awaiting_state.app_keys = None;
    awaiting_state.authorization_state =
        iris_drive_core::AppKeyAuthorizationState::AwaitingApproval;
    restored_config.profile = Some(awaiting_state);
    restored_config
        .save(config_path_in(restored_dir.path()))
        .expect("save restored config");

    let recovered = restored_app.dispatch(NativeAppAction::AdmitAppKeyWithRecoveryPhrase {
        recovery_phrase: String::new(),
        label: "Recovered browser".to_owned(),
    });

    assert!(recovered.error.is_empty(), "{}", recovered.error);
    let recovered_account = recovered.ui.profile.expect("recovered account exists");
    assert_eq!(recovered_account.profile_id, created_account.profile_id);
    assert_eq!(
        recovered_account.current_app_key_pubkey,
        restored_account.current_app_key_pubkey
    );
    assert_eq!(recovered_account.current_app_key_label, "Recovered browser");
    assert!(recovered_account.can_write_roots);
    assert!(recovered_account.can_admin_profile);
    assert_eq!(recovered_account.current_key_epoch, Some(2));
    assert_eq!(
        recovered_account.profile_roster_op_count,
        created_account.profile_roster_op_count + 2
    );
}

#[test]
fn raw_secret_key_restore_uses_fresh_app_key_without_phrase_export() {
    let dir = tempfile::tempdir().unwrap();
    let recovery_phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let created = app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: recovery_phrase.clone(),
        app_key_label: "Pixel".to_owned(),
    });
    assert!(created.error.is_empty(), "{}", created.error);
    let created_account = created.ui.profile.as_ref().expect("profile exists");
    let secret_key = iris_drive_core::recovery_phrase::recovery_phrase_to_nsec(&recovery_phrase)
        .expect("recovery phrase derives secret");

    let restored_dir = tempfile::tempdir().unwrap();
    let restored_app = FfiApp::new(restored_dir.path().display().to_string(), "test".to_owned());
    let restored = restored_app.dispatch(NativeAppAction::RestoreProfile {
        recovery_secret: secret_key,
        app_key_label: "Restored".to_owned(),
    });

    assert!(restored.error.is_empty(), "{}", restored.error);
    let restored_account = restored.ui.profile.expect("restored profile exists");
    assert_ne!(restored_account.profile_id, created_account.profile_id);
    assert_ne!(
        restored_account.current_app_key_npub,
        created_account.current_app_key_npub
    );
    assert!(restored_account.can_admin_profile);
    assert!(!restored_account.can_export_recovery_phrase);

    let raw_export = super::export_recovery_secret(restored_dir.path().display().to_string());
    assert!(!raw_export.can_export);
    assert!(raw_export.words.is_empty());
    assert!(raw_export.recovery_phrase.is_empty());
    assert!(raw_export.secret_key.is_empty());
    assert!(
        raw_export.error.contains("loading recovery phrase"),
        "{}",
        raw_export.error
    );
}

#[test]
fn relay_actions_normalize_and_dedupe_urls() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRelay {
        url: " relay.example/ ".to_owned(),
    });

    assert!(state.error.is_empty());
    assert!(state.ui.relays.contains(&"wss://relay.example".to_owned()));
    assert!(!state.ui.relays.contains(&"relay.example/".to_owned()));
    assert_eq!(
        state
            .ui
            .relays
            .iter()
            .filter(|relay| relay.as_str() == "wss://relay.example")
            .count(),
        1
    );

    let state = app.dispatch(NativeAppAction::AddRelay {
        url: "wss://relay.example".to_owned(),
    });
    assert_eq!(
        state
            .ui
            .relays
            .iter()
            .filter(|relay| relay.as_str() == "wss://relay.example")
            .count(),
        1
    );

    let relay_status = state
        .ui
        .relay_statuses
        .iter()
        .find(|relay| relay.url == "wss://relay.example")
        .expect("normalized relay status is emitted");
    assert_eq!(relay_status.status_label, "saved");
    assert_eq!(relay_status.health, "configured");

    let state = app.dispatch(NativeAppAction::RemoveRelay {
        url: "relay.example/".to_owned(),
    });
    assert!(state.error.is_empty());
    assert!(!state.ui.relays.contains(&"wss://relay.example".to_owned()));
}

#[test]
fn uninitialized_state_exposes_summary_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.state();

    assert_eq!(state.ui.setup_state, "not_configured");
    assert!(!state.ui.setup_complete);
    assert!(!state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.setup_label, "Not linked");
    assert_eq!(state.ui.primary_status, "not_setup");
    assert_eq!(state.ui.primary_status_label, "Ready");
    assert_eq!(state.ui.authorized_app_key_count, 0);
    assert_eq!(state.ui.online_app_key_count, 0);
    assert_eq!(state.ui.file_count, 0);
    assert_eq!(state.ui.visible_file_bytes, 0);
    assert!(state.ui.sites_portal_url.is_empty());
}

#[test]
fn classify_link_input_uses_core_invite_and_key_parsing() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();

    let invite = super::classify_link_input(owner_account.app_key_link_invite.clone());
    assert_eq!(invite.kind, "invite");
    assert!(invite.is_complete);
    assert!(invite.is_valid);
    let admin_app_key_npub = invite.admin_app_key_pubkey.clone();
    assert!(!admin_app_key_npub.is_empty());
    assert!(invite.has_invite_pubkey);

    let npub = super::classify_link_input(admin_app_key_npub.clone());
    assert_eq!(npub.kind, "app_key_pubkey");
    assert!(npub.is_complete);
    assert!(npub.is_valid);
    assert_eq!(npub.admin_app_key_pubkey, admin_app_key_npub);

    let short_invite = super::classify_link_input("https://drive.iris.to/invite/abc".to_owned());
    assert_eq!(short_invite.kind, "invite");
    assert!(!short_invite.is_complete);
    assert!(!short_invite.is_valid);

    let custom_scheme_invite = super::classify_link_input("iris-drive://invite/abc".to_owned());
    assert_eq!(custom_scheme_invite.kind, "unknown");

    let short_npub = super::classify_link_input(admin_app_key_npub[..20].to_owned());
    assert_eq!(short_npub.kind, "app_key_pubkey");
    assert!(!short_npub.is_complete);
    assert!(!short_npub.is_valid);

    let profile_id = owner_account.profile_id.clone();
    let request_device = nostr_sdk::Keys::generate();
    let request_device_npub =
        iris_drive_core::app_key_summary::pubkey_npub(&request_device.public_key().to_hex());
    let approval = super::classify_link_input(
        iris_drive_core::app_key_link_transport::encode_app_key_approval_request(
            &request_device,
            Some(profile_id.parse().unwrap()),
            Some(&iris_drive_core::normalize_app_key_pubkey(&admin_app_key_npub).unwrap()),
            None,
            123,
        )
        .unwrap(),
    );
    assert_eq!(approval.kind, "app_key_approval");
    assert!(approval.is_complete);
    assert!(approval.is_valid);
    assert_eq!(approval.app_key_pubkey, request_device_npub);

    let web_invite_route =
        super::classify_link_input("https://drive.iris.to/invite/demo".to_owned());
    assert_eq!(web_invite_route.kind, "invite");
    assert!(!web_invite_route.is_complete);

    let unrelated = super::classify_link_input(
        "https://drive.iris.to/app-key-linker?owner=npub1example".to_owned(),
    );
    assert_eq!(unrelated.kind, "iris_web");
    assert!(unrelated.is_valid);

    let share_dialog = super::classify_link_input(
        "https://drive.iris.to/share?path=My%20Drive%2FPhotos&name=Photos&recipient_npub=npub1alice&recipient_name=Alice&recipient_profile=123e4567-e89b-42d3-a456-426614174000".to_owned(),
    );
    assert_eq!(share_dialog.kind, "share_dialog");
    assert!(share_dialog.is_complete);
    assert!(share_dialog.is_valid);
    assert_eq!(share_dialog.share_source_path, "My Drive/Photos");
    assert_eq!(share_dialog.share_display_name, "Photos");
    assert_eq!(share_dialog.share_recipient_npub_hint, "npub1alice");
    assert_eq!(share_dialog.share_recipient_display_name, "Alice");
    assert_eq!(
        share_dialog.share_recipient_profile_id,
        "123e4567-e89b-42d3-a456-426614174000"
    );
}

const _: () = {
    assert!(super::NATIVE_DIRECT_ROOT_EXCHANGE_MILLIS >= 5_000);
    assert!(super::NATIVE_DIRECT_ROOT_EXCHANGE_MILLIS >= super::APP_KEY_LINK_EXCHANGE_TICK_MILLIS);
};

#[test]
fn link_device_rejects_bare_app_key_without_profile_target() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let state = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_account.current_app_key_npub,
        app_key_label: "Phone".to_owned(),
    });

    assert!(state.ui.profile.is_none());
    assert!(
        state
            .error
            .contains("paste an NostrIdentity invite URL to link this device")
    );
}

#[test]
fn snapshot_link_uses_drive_iris_nhash_route() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });
    let account = created.ui.profile.as_ref().expect("account exists");
    let root_cid = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
                    :1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100"
        .replace(char::is_whitespace, "");

    let config_path = config_path_in(dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let mut drive = Drive::primary(account.profile_id.clone());
    drive.last_root_cid = Some(root_cid.clone());
    drive.app_key_roots.insert(
        account.current_app_key_pubkey.clone(),
        AppKeyRootRef::legacy(root_cid, 1, 0),
    );
    config.upsert_drive(drive);
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();

    assert!(
        refreshed
            .ui
            .snapshot_link
            .starts_with("https://drive.iris.to/#/nhash1")
    );
    assert!(!refreshed.ui.snapshot_link.contains("/snapshot/"));
}

#[test]
fn app_state_surfaces_local_resolver_and_portal_settings() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });

    assert!(created.ui.local_nhash_resolver_enabled);
    assert!(created.ui.launch_on_startup);
    assert_eq!(
        created.ui.sites_portal_url,
        iris_drive_core::gateway::local_portal_url(iris_drive_core::gateway::DEFAULT_GATEWAY_PORT)
    );
    assert_eq!(
        created.ui.caldav_url,
        iris_drive_core::gateway::local_caldav_url_for_identity(
            iris_drive_core::gateway::DEFAULT_GATEWAY_PORT,
            &created.ui.profile.as_ref().unwrap().current_app_key_npub
        )
    );

    let config_path = config_path_in(dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config.local_nhash_resolver_enabled = false;
    config.launch_on_startup = false;
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();

    assert!(!refreshed.ui.local_nhash_resolver_enabled);
    assert!(!refreshed.ui.launch_on_startup);
    assert!(refreshed.ui.sites_portal_url.is_empty());
    assert!(refreshed.ui.caldav_url.is_empty());
}

#[test]
fn app_state_dispatch_toggles_launch_on_startup() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let disabled = app.dispatch(NativeAppAction::SetLaunchOnStartup { enabled: false });

    assert!(!disabled.ui.launch_on_startup);
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(!config.launch_on_startup);

    let enabled = app.dispatch(NativeAppAction::SetLaunchOnStartup { enabled: true });

    assert!(enabled.ui.launch_on_startup);
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(config.launch_on_startup);
}

#[test]
fn drive_link_for_cid_uses_drive_iris_nhash_route() {
    let root_cid = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
                    :1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100"
        .replace(char::is_whitespace, "");

    let link = super::drive_link_for_cid_value(&root_cid);

    assert!(link.error.is_empty());
    assert!(link.url.starts_with("https://drive.iris.to/#/nhash1"));
}

#[test]
fn logout_clears_local_profile_state_and_key_material() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    assert!(created.ui.profile.is_some());
    assert!(dir.path().join("key").exists());

    let state = app.dispatch(NativeAppAction::Logout);

    assert!(state.error.is_empty());
    assert!(state.ui.profile.is_none());
    assert!(state.ui.app_actors.is_empty());
    assert!(state.ui.roots.is_empty());
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "ready");
    assert_eq!(state.ui.sync.status_label, "Ready");
    assert!(!dir.path().join("key").exists());
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(config.profile.is_none());
    assert!(config.drives.is_empty());
}

#[test]
fn link_action_tracks_pending_approval() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_profile_id = owner_account.profile_id.clone();
    let invite = owner_account.app_key_link_invite.clone();

    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "iPhone".to_owned(),
    });

    let account = state.ui.profile.expect("account exists");
    assert_eq!(account.profile_id, owner_profile_id);
    assert_eq!(account.app_key_label, "iPhone");
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert!(!account.can_admin_profile);
    assert!(
        account
            .app_key_link_request
            .starts_with("https://drive.iris.to/approve-device/")
    );
    let request = iris_drive_core::app_key_link_transport::parse_app_key_approval_request(
        &account.app_key_link_request,
    )
    .unwrap()
    .unwrap();
    assert_eq!(request.profile_id, Some(owner_profile_id.parse().unwrap()));
    assert_eq!(
        iris_drive_core::app_key_summary::pubkey_npub(&request.app_key_hex),
        account.current_app_key_npub
    );
    assert!(!account.app_key_link_request.contains("local-owner"));
    assert!(!account.app_key_link_request.contains("app_key=device-"));
    assert!(
        state.ui.app_actors.is_empty(),
        "pending devices should not appear in the authorized device roster"
    );
    assert_eq!(state.ui.setup_state, "awaiting_approval");
    assert!(!state.ui.setup_complete);
    assert!(state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.primary_status, "awaiting_approval");
    assert_eq!(state.ui.authorized_app_key_count, 0);
    assert!(state.ui.sites_portal_url.is_empty());
}

#[test]
fn app_key_link_request_url_is_stable_across_profile_refreshes() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "iPhone".to_owned(),
    });
    let first = linked
        .ui
        .profile
        .as_ref()
        .expect("linked profile")
        .app_key_link_request
        .clone();
    assert!(first.starts_with("https://drive.iris.to/approve-device/"));

    let second = linked_app
        .dispatch(NativeAppAction::RefreshProfile)
        .ui
        .profile
        .as_ref()
        .expect("refreshed profile")
        .app_key_link_request
        .clone();
    let third = linked_app
        .refresh()
        .ui
        .profile
        .as_ref()
        .expect("refreshed profile")
        .app_key_link_request
        .clone();

    assert_eq!(first, second);
    assert_eq!(first, third);
    let config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let pending = config
        .profile
        .as_ref()
        .and_then(|profile| profile.outbound_app_key_link_request.as_ref())
        .expect("pending request");
    assert_eq!(pending.request_url, first);
}

#[test]
fn owner_can_approve_and_revoke_linked_app_keys() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_invite = owner.ui.profile.unwrap().app_key_link_invite;
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite.clone(),
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let request = linked_account.app_key_link_request;
    let linked_device = linked_account.current_app_key_npub;
    let state = app.dispatch(NativeAppAction::ApproveDevice {
        request,
        label: "Phone".to_owned(),
    });

    assert!(state.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device
            && device.label.is_empty()
            && device.role == "member"
            && device.can_revoke
            && device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::AppointAdmin {
        app_key_pubkey: linked_device.clone(),
    });
    assert!(state.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device
            && device.role == "admin"
            && device.can_demote_admin
            && !device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::DemoteAdmin {
        app_key_pubkey: linked_device.clone(),
    });
    assert!(state.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device
            && device.role == "member"
            && !device.can_demote_admin
            && device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::RevokeDevice {
        app_key_pubkey: linked_device.clone(),
    });

    assert!(
        !state
            .ui
            .app_actors
            .iter()
            .any(|device| device.pubkey == linked_device)
    );
    assert!(state.error.is_empty());
}

#[test]
fn approving_tombstoned_inbound_request_readds_device() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_invite = owner.ui.profile.unwrap().app_key_link_invite;
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub.clone();
    let linked_app_key_hex = normalize_pubkey(&linked_device).unwrap();
    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);
    let revoked = app.dispatch(NativeAppAction::RevokeDevice {
        app_key_pubkey: linked_device.clone(),
    });
    assert!(revoked.error.is_empty(), "{}", revoked.error);

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret).unwrap();
    let removed_at = state
        .profile_projection()
        .tombstones
        .get(&linked_app_key_hex)
        .unwrap()
        .removed_at;
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &linked_app_key_hex,
            Some("Phone".to_owned()),
            &invite_pubkey,
            None,
            u64::try_from(removed_at).unwrap() + 1,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();
    let request = refreshed.ui.profile.unwrap().inbound_app_key_link_requests[0]
        .request_link
        .clone();
    let op_count_before = AppConfig::load_or_default(&config_path)
        .unwrap()
        .profile
        .unwrap()
        .profile_roster_ops
        .len();

    let readded = app.dispatch(NativeAppAction::ApproveDevice {
        request,
        label: "Phone again".to_owned(),
    });

    assert!(readded.error.is_empty(), "{}", readded.error);
    let account = readded.ui.profile.as_ref().unwrap();
    assert!(account.inbound_app_key_link_requests.is_empty());
    assert!(readded.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device && device.label.is_empty() && device.role == "member"
    }));
    assert_eq!(
        AppConfig::load_or_default(&config_path)
            .unwrap()
            .profile
            .unwrap()
            .profile_roster_ops
            .len(),
        op_count_before + 2
    );
}

#[test]
fn delete_device_json_action_revokes_linked_device() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_invite = owner.ui.profile.unwrap().app_key_link_invite;
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite.clone(),
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub.clone();
    let state = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(
        state
            .ui
            .app_actors
            .iter()
            .any(|device| device.pubkey == linked_device)
    );

    let action: NativeAppAction = serde_json::from_value(serde_json::json!({
        "type": "delete_device",
        "app_key_pubkey": linked_device,
    }))
    .unwrap();
    let state = app.dispatch(action);

    assert!(
        state
            .ui
            .app_actors
            .iter()
            .all(|device| device.label != "Phone")
    );
    assert!(state.error.is_empty());
}

#[test]
fn revoked_current_device_refresh_logs_out_and_allows_fresh_relink() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_invite = owner_account.app_key_link_invite.clone();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite.clone(),
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub.clone();
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);
    apply_latest_profile_roster_frame(owner_dir.path(), linked_dir.path());

    let authorized = linked_app.refresh();
    let account = authorized.ui.profile.as_ref().expect("account exists");
    assert_eq!(account.authorization_state, "authorized");
    assert!(authorized.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device && device.label == "Phone" && device.is_current_app_key
    }));

    let running = linked_app.dispatch(NativeAppAction::StartSync);
    assert!(running.ui.sync.running);

    let revoked = owner_app.dispatch(NativeAppAction::RevokeDevice {
        app_key_pubkey: linked_device.clone(),
    });
    assert!(revoked.error.is_empty(), "{}", revoked.error);
    apply_latest_profile_roster_frame(owner_dir.path(), linked_dir.path());

    let refreshed = linked_app.refresh();
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    assert!(refreshed.ui.profile.is_none());
    assert!(refreshed.ui.app_actors.is_empty());
    assert!(refreshed.ui.roots.is_empty());
    assert!(refreshed.ui.snapshot_link.is_empty());
    assert!(!refreshed.ui.sync.running);
    assert_eq!(refreshed.ui.sync.status, "ready");
    assert_eq!(refreshed.ui.sync.status_label, "Ready");
    assert!(!linked_dir.path().join("key").exists());
    let linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    assert!(linked_config.profile.is_none());

    let relinked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let account = relinked.ui.profile.as_ref().expect("account exists");
    assert!(relinked.error.is_empty(), "{}", relinked.error);
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert_ne!(account.current_app_key_npub, linked_device);
    assert_eq!(account.app_key_label, "Phone");
    assert!(
        account
            .app_key_link_request
            .starts_with("https://drive.iris.to/approve-device/")
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn daemon_fips_status_drives_device_online_presence() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_invite = owner_account.app_key_link_invite;
    let current_device = owner_account.current_app_key_npub;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub;

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty());
    assert!(
        approved
            .ui
            .app_actors
            .iter()
            .all(|device| !device.is_online)
    );

    write_daemon_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[linked_device.as_str()],
        &[],
        super::unix_now_seconds(),
    );
    let refreshed = app.refresh();
    let current = refreshed
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == current_device)
        .expect("current device in roster");
    assert!(current.is_current_app_key);
    assert!(current.is_online);
    assert_eq!(current.state, "Linked");
    assert_eq!(current.connection_state, "local");
    assert_eq!(current.connection_label, "This Device");
    assert_eq!(refreshed.ui.fips.state, "running");
    assert_eq!(refreshed.ui.fips.state_label, "Running");
    assert_eq!(refreshed.ui.fips.roster_label, "1/1 online");
    assert_eq!(refreshed.ui.fips.roster_peer_count, 1);
    assert_eq!(refreshed.ui.fips.roster_online_device_count, 1);
    assert_eq!(refreshed.ui.fips.roster_direct_device_count, 1);
    assert_eq!(refreshed.ui.fips.other_peer_count, 0);
    assert_eq!(refreshed.ui.fips.peer_statuses.len(), 1);
    assert_eq!(refreshed.ui.fips.peer_statuses[0].npub, linked_device);
    assert_eq!(
        refreshed.ui.fips.peer_statuses[0].connection_label,
        "UDP, 34 ms"
    );
    let linked = refreshed
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(!linked.is_current_app_key);
    assert!(linked.is_online);
    assert_eq!(linked.state, "Linked");
    assert_eq!(linked.connection_state, "direct");
    assert_eq!(linked.connection_label, "Online (UDP, 34 ms)");

    write_daemon_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[],
        &[linked_device.as_str()],
        super::unix_now_seconds(),
    );
    let mesh_only = app.refresh();
    let linked = mesh_only
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(linked.is_online);
    assert_eq!(linked.connection_state, "mesh");
    assert_eq!(linked.connection_label, "Online (Mesh)");

    write_daemon_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[],
        &[linked_device.as_str()],
        super::unix_now_seconds().saturating_sub(120),
    );
    let stale = app.refresh();
    assert!(stale.ui.app_actors.iter().all(|device| !device.is_online));
    assert_eq!(stale.ui.fips.state, "stale");
    assert_eq!(stale.ui.fips.state_label, "Stale");
    assert_eq!(stale.ui.fips.roster_label, "0/1 online");
    assert_eq!(stale.ui.fips.roster_online_device_count, 0);
    let linked = stale
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in stale roster");
    assert_eq!(linked.connection_state, "offline");
    assert_eq!(linked.connection_label, "Offline");
    assert_eq!(stale.ui.fips.online_device_count, 0);
    assert!(!stale.ui.fips.fresh);
}

#[test]
fn native_fips_status_parser_drives_mobile_presence_source() {
    let owner_dir = tempfile::tempdir().unwrap();
    let current_device = "npub1current";
    let linked_device = "npub1linked";

    write_native_fips_status_fixture(
        owner_dir.path(),
        current_device,
        &[linked_device],
        &[],
        super::unix_now_seconds(),
    );

    let status = super::ui_fips_status_for_native_config_dir(owner_dir.path());

    assert_eq!(status.state, "running");
    assert_eq!(status.state_label, "Running");
    assert_eq!(status.roster_label, "1/1 online");
    assert_eq!(status.roster_online_device_count, 1);
    assert_eq!(status.peer_statuses.len(), 1);
    assert_eq!(status.peer_statuses[0].npub, linked_device);
    assert_eq!(status.peer_statuses[0].connection_label, "TCP, 12 ms");
}

#[test]
fn daemon_fips_status_overrides_stale_native_fips_status_for_desktop_refresh() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_invite = owner_account.app_key_link_invite;
    let current_device = owner_account.current_app_key_npub;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub;

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty());

    let now = super::unix_now_seconds();
    write_native_fips_status_fixture(owner_dir.path(), &current_device, &[], &[], now);
    write_daemon_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[linked_device.as_str()],
        &[],
        now,
    );

    let refreshed = app.refresh();
    let linked = refreshed
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(linked.is_online);
    assert_eq!(linked.connection_state, "direct");
    assert_eq!(linked.connection_label, "Online (UDP, 34 ms)");
    assert_eq!(refreshed.ui.fips.roster_label, "1/1 online");
    assert_eq!(refreshed.ui.fips.roster_online_device_count, 1);
}

#[test]
fn desktop_refresh_ignores_native_fips_status_without_daemon_status() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_invite = owner_account.app_key_link_invite;
    let current_device = owner_account.current_app_key_npub;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub;

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty());

    write_native_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[linked_device.as_str()],
        &[],
        super::unix_now_seconds(),
    );

    let refreshed = app.refresh();

    assert_eq!(refreshed.ui.fips.state, "paused");
    assert_eq!(refreshed.ui.fips.roster_label, "0/0 online");
    assert!(!owner_dir.path().join("native-fips-status.json").exists());
    let linked = refreshed
        .ui
        .app_actors
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(!linked.is_online);
    assert_eq!(linked.connection_state, "offline");
}

#[test]
fn owner_state_surfaces_inbound_requests_for_accept_flow() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_device = linked.ui.profile.unwrap().current_app_key_npub;
    let linked_app_key_hex = normalize_pubkey(&linked_device).unwrap();

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret).unwrap();
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &linked_app_key_hex,
            Some("Phone".to_owned()),
            &invite_pubkey,
            None,
            42,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();
    let account = refreshed.ui.profile.unwrap();
    assert_eq!(account.inbound_app_key_link_requests.len(), 1);
    let request = &account.inbound_app_key_link_requests[0];
    assert_eq!(request.app_key_pubkey, linked_device);
    assert_eq!(request.label, "Phone");
    assert_eq!(request.requested_at, 42);
    assert_eq!(request.request_link, linked_device);

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: request.request_link.clone(),
        label: String::new(),
    });
    assert!(approved.error.is_empty());
    assert!(approved.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device && device.label.is_empty() && device.role == "member"
    }));
}

#[test]
fn owner_can_reject_inbound_app_key_link_request() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_device = linked.ui.profile.unwrap().current_app_key_npub;
    let linked_app_key_hex = normalize_pubkey(&linked_device).unwrap();

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret).unwrap();
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &linked_app_key_hex,
            Some("Phone".to_owned()),
            &invite_pubkey,
            None,
            42,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();
    let request = refreshed.ui.profile.unwrap().inbound_app_key_link_requests[0]
        .request_link
        .clone();
    let rejected = app.dispatch(NativeAppAction::RejectDevice { request });

    assert!(rejected.error.is_empty(), "{}", rejected.error);
    assert!(
        rejected
            .ui
            .profile
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
    assert!(
        rejected
            .ui
            .app_actors
            .iter()
            .all(|device| device.pubkey != linked_device)
    );
    let saved = AppConfig::load_or_default(&config_path).unwrap();
    assert!(
        saved
            .profile
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
}

#[test]
fn app_key_link_request_retry_uses_startup_burst_before_steady_interval() {
    let now = std::time::Instant::now();
    assert!(app_key_link_request_send_due(None, now));

    let first = SentAppKeyLinkRequest {
        last_sent: now,
        attempts: 1,
    };
    assert!(!app_key_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_millis(
            super::APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS - 1
        )
    ));
    assert!(app_key_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_millis(super::APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS)
    ));

    let steady = SentAppKeyLinkRequest {
        last_sent: now,
        attempts: super::APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS,
    };
    assert!(!app_key_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(super::APP_KEY_LINK_REQUEST_RETRY_SECS - 1)
    ));
    assert!(app_key_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(super::APP_KEY_LINK_REQUEST_RETRY_SECS)
    ));
}

fn write_native_fips_status_fixture(
    dir: &Path,
    endpoint_npub: &str,
    connected_peers: &[&str],
    mesh_peers: &[&str],
    updated_at: u64,
) {
    let path = dir.join(super::NATIVE_FIPS_STATUS_FILE_NAME);
    let mut peer_statuses = connected_peers
        .iter()
        .map(|peer| {
            serde_json::json!({
                "npub": peer,
                "transport_type": "tcp",
                "srtt_ms": 12
            })
        })
        .collect::<Vec<_>>();
    peer_statuses.extend(mesh_peers.iter().map(|peer| {
        serde_json::json!({
            "npub": peer,
            "bytes_recv": 1
        })
    }));
    let value = serde_json::json!({
        "running": true,
        "updated_at": updated_at,
        "endpoint_npub": endpoint_npub,
        "authorized_peers": connected_peers.iter().chain(mesh_peers.iter()).copied().collect::<Vec<_>>(),
        "connected_peers": connected_peers,
        "mesh_peers": mesh_peers,
        "peer_statuses": peer_statuses,
        "error": null,
    });
    std::fs::write(path, value.to_string()).unwrap();
}

fn write_daemon_fips_status_fixture(
    dir: &Path,
    endpoint_npub: &str,
    connected_peers: &[&str],
    mesh_peers: &[&str],
    updated_at: u64,
) {
    let path = dir.join(super::DAEMON_STATUS_FILE_NAME);
    let online_peers = connected_peers
        .iter()
        .chain(mesh_peers.iter())
        .copied()
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "running": true,
        "fresh": true,
        "updated_at": updated_at,
        "fips_block_sync": {
            "endpoint_npub": endpoint_npub,
            "discovery_scope": "fips-overlay-v1",
            "authorized_peers": online_peers,
            "online_devices": online_peers,
            "online_peers": online_peers,
            "connected_peers": connected_peers,
            "direct_devices": connected_peers,
            "direct_peers": connected_peers,
            "mesh_devices": mesh_peers,
            "mesh_peers": mesh_peers,
            "peer_statuses": connected_peers.iter().map(|peer| serde_json::json!({
                "npub": peer,
                "transport_type": "udp",
                "srtt_ms": 34
            })).chain(mesh_peers.iter().map(|peer| serde_json::json!({
                "npub": peer,
                "bytes_recv": 1
            }))).collect::<Vec<_>>(),
        },
        "fips_block_sync_error": null,
    });
    std::fs::write(path, value.to_string()).unwrap();
}

#[test]
fn reset_invite_action_rotates_invite_and_clears_requests() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let old_invite = owner.ui.profile.unwrap().app_key_link_invite;

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret).unwrap();
    let linked_device =
        iris_drive_core::AppKey::generate(owner_dir.path().join("tmp-key")).pubkey_hex();
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &linked_device,
            Some("Phone".to_owned()),
            &invite_pubkey,
            None,
            42,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let reset = app.dispatch(NativeAppAction::ResetInvite);
    assert!(reset.error.is_empty());
    let account = reset.ui.profile.unwrap();
    assert_ne!(account.app_key_link_invite, old_invite);
    assert!(account.inbound_app_key_link_requests.is_empty());
}

#[test]
fn native_profile_roster_ops_refresh_authorized_member_roster() {
    let owner_dir = tempfile::tempdir().unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let mut owner = iris_drive_core::Profile::create(owner_dir.path(), Some("Mac".into())).unwrap();
    let mut linked = iris_drive_core::Profile::link_to_profile(
        linked_dir.path(),
        owner.state.profile_id,
        owner.state.app_key_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    linked
        .state
        .queue_outbound_app_key_link_request(
            owner.state.app_key_pubkey.clone(),
            &iris_drive_core::app_key_link_invite_pubkey(&owner.state.app_key_link_secret).unwrap(),
            123,
        )
        .unwrap();
    owner
        .approve_app_key(&linked_pubkey, Some("Phone".into()))
        .unwrap();

    let mut linked_config = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    linked_config.upsert_drive(iris_drive_core::Drive::primary(owner.state.root_scope_id()));
    let first_frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner.state.profile_id,
        admin_app_key_pubkey: owner.state.app_key_pubkey.clone(),
        profile_roster_ops: owner.state.profile_roster_ops.clone(),
        sent_at: 456,
    };
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &first_frame,
        &owner.state.app_key_pubkey,
    )
    .unwrap();
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let third_device = nostr_sdk::Keys::generate().public_key().to_hex();
    owner
        .approve_app_key(&third_device, Some("Pixel".into()))
        .unwrap();
    let updated_frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner.state.profile_id,
        admin_app_key_pubkey: owner.state.app_key_pubkey.clone(),
        profile_roster_ops: owner.state.profile_roster_ops.clone(),
        sent_at: 789,
    };
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &updated_frame,
        &owner.state.app_key_pubkey,
    )
    .unwrap();
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let linked_roster = linked_config
        .profile
        .as_ref()
        .unwrap()
        .app_keys
        .as_ref()
        .unwrap();
    assert!(linked_roster.contains(&third_device));
}

fn apply_latest_profile_roster_frame(from: &Path, to: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(from)).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner_state.profile_id,
        admin_app_key_pubkey: owner_state.app_key_pubkey.clone(),
        profile_roster_ops: owner_state.profile_roster_ops.clone(),
        sent_at: 123,
    };
    let mut linked_config = AppConfig::load_or_default(config_path_in(to)).unwrap();
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();
    linked_config.save(config_path_in(to)).unwrap();
}
