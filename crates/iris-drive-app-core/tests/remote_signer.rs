#[allow(dead_code)]
#[path = "../../iris-drive-cli/tests/support/mod.rs"]
mod support;

use iris_drive_app_core::{FfiApp, NativeAppAction};
use iris_drive_core::{AppConfig, Profile, paths::config_path_in, relay_sync};
use nostr_connect::prelude::{
    NostrConnectKeys, NostrConnectRemoteSigner, NostrConnectRequest, NostrConnectSignerActions,
};
use nostr_sdk::{Keys, PublicKey};
use std::time::Duration;
use support::{LocalNostrRelay, add_config_relay};
use tempfile::tempdir;

struct ApproveRecovery;
impl NostrConnectSignerActions for ApproveRecovery {
    fn approve(&self, _: &PublicKey, _: &NostrConnectRequest) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signer_restores_existing_profile_through_native_setup() {
    let roster = LocalNostrRelay::spawn().await;
    let transport = LocalNostrRelay::spawn().await;
    let owner_dir = tempdir().unwrap();
    let fresh_dir = tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("Original device".into())).unwrap();
    let client = relay_sync::connect(std::slice::from_ref(&roster.url))
        .await
        .unwrap();
    relay_sync::publish_nostr_identity_roster_ops(&client, &owner.state.profile_roster_ops)
        .await
        .unwrap();
    let remote = NostrConnectRemoteSigner::new(
        NostrConnectKeys {
            signer: Keys::generate(),
            user: owner.app_key.keys().clone(),
        },
        [transport.url.as_str()],
        Some("single-use-test".into()),
        None,
    )
    .unwrap();
    let uri = remote.bunker_uri().to_string();
    let gate = transport.pause_relay_queries();
    let service = tokio::spawn(async move {
        remote.serve(ApproveRecovery).await.unwrap();
    });
    tokio::task::spawn_blocking(move || {
        gate.await_query();
        drop(gate);
    })
    .await
    .unwrap();
    add_config_relay(fresh_dir.path(), &roster.url);
    let path = fresh_dir.path().to_owned();
    let (app, pending_key) = tokio::task::spawn_blocking(move || {
        let app = FfiApp::new(path.display().to_string(), "test".into());
        let pending = app.dispatch(NativeAppAction::StartJoinRequest {
            app_key_label: "New device".into(),
        });
        assert!(pending.ui.awaiting_approval);
        let key = AppConfig::load_or_default(config_path_in(&path))
            .unwrap()
            .profile
            .unwrap()
            .app_key_pubkey;
        (app, key)
    })
    .await
    .unwrap();
    let state = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            app.dispatch(NativeAppAction::RestoreProfile {
                recovery_secret: uri,
                app_key_label: "New device".into(),
            })
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(state.error.is_empty(), "{}", state.error);
    assert!(state.ui.setup_complete);
    let config = AppConfig::load_or_default(config_path_in(fresh_dir.path())).unwrap();
    let restored = Profile::load(config.profile.unwrap(), fresh_dir.path()).unwrap();
    assert_eq!(restored.state.profile_id, owner.state.profile_id);
    assert_eq!(restored.state.app_key_pubkey, pending_key);
    assert_eq!(
        restored.current_dck().unwrap(),
        owner.current_dck().unwrap()
    );
    assert!(
        !std::fs::read_to_string(config_path_in(fresh_dir.path()))
            .unwrap()
            .contains("single-use-test")
    );
    assert_eq!(
        config.drives[0].root_scope_id,
        restored.state.root_scope_id()
    );
    relay_sync::publish_nostr_identity_roster_ops(&client, &restored.state.profile_roster_ops)
        .await
        .unwrap();
    let events = relay_sync::fetch_nostr_identity_roster_ops(
        &client,
        owner.state.profile_id,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    owner.state.profile_roster_ops = events
        .iter()
        .map(|event| iris_drive_core::parse_nostr_identity_roster_op_event(event).unwrap())
        .collect();
    owner.state.profile_roster_projection = None;
    owner.state.sync_app_keys_from_profile();
    assert!(
        owner
            .state
            .profile_projection()
            .can_write_roots(&pending_key)
    );
    assert_eq!(
        owner.current_dck().unwrap(),
        restored.current_dck().unwrap()
    );
    service.abort();
    relay_sync::shutdown_client(&client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_waiting_preserves_pending_setup() {
    let roster = LocalNostrRelay::spawn().await;
    let transport = LocalNostrRelay::spawn().await;
    let fresh_dir = tempdir().unwrap();
    add_config_relay(fresh_dir.path(), &roster.url);
    let path = fresh_dir.path().to_owned();
    let app = tokio::task::spawn_blocking(move || {
        let app = FfiApp::new(path.display().to_string(), "test".into());
        assert!(
            app.dispatch(NativeAppAction::StartJoinRequest {
                app_key_label: "New device".into()
            })
            .ui
            .awaiting_approval
        );
        app
    })
    .await
    .unwrap();
    let profile = AppConfig::load_or_default(config_path_in(fresh_dir.path()))
        .unwrap()
        .profile
        .unwrap();
    let gate = transport.pause_relay_queries();
    let uri = format!(
        "bunker://{}?relay={}&secret=private-test-secret",
        Keys::generate().public_key(),
        transport.url
    );
    let pending_app = app.clone();
    let pending = tokio::task::spawn_blocking(move || {
        pending_app.dispatch(NativeAppAction::RestoreProfile {
            recovery_secret: uri,
            app_key_label: "New device".into(),
        })
    });
    tokio::task::spawn_blocking(move || {
        gate.await_query();
        assert!(app.state().signer_approval_url.is_none());
        assert!(
            app.dispatch(NativeAppAction::SignerApprovalLink)
                .signer_approval_url
                .is_none()
        );
        let _ = app.dispatch(NativeAppAction::CancelSignerRecovery);
        drop(gate);
    })
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.error, "Sign-in cancelled");
    let next = AppConfig::load_or_default(config_path_in(fresh_dir.path()))
        .unwrap()
        .profile
        .unwrap();
    assert_eq!(profile.profile_id, next.profile_id);
    assert_eq!(profile.app_key_pubkey, next.app_key_pubkey);
    assert!(next.profile_roster_ops.is_empty());
}
