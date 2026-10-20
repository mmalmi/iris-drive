use super::*;

fn assert_drive_root_replay_download_behavior(
    current_config: &AppConfig,
    drive_root_event: &Event,
    linked_keys: &nostr_sdk::Keys,
    publisher: &str,
    current_root_cid: &str,
) {
    let mut replay_config = current_config.clone();
    let replay_outcome = apply_native_drive_root_relay_event_to_config(
        &mut replay_config,
        drive_root_event,
        linked_keys,
    )
    .unwrap();
    assert_eq!(replay_outcome, NativeAppKeyLinkRelayEventApply::Current);
    assert_eq!(
        drive_root_event_download_target(
            &replay_config,
            drive_root_event,
            linked_keys,
            replay_outcome,
        )
        .unwrap(),
        Some(current_root_cid.to_owned()),
        "replayed current metadata must retry a transiently failed block download"
    );

    replay_config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get_mut(publisher)
        .unwrap()
        .root_cid = hashtree_core::Cid::encrypted([0x55; 32], [0x66; 32]).to_string();
    assert_eq!(
        drive_root_event_download_target(
            &replay_config,
            drive_root_event,
            linked_keys,
            replay_outcome,
        )
        .unwrap(),
        None,
        "a stale replay must not download a root superseded in config"
    );
}

struct AuthorizedDriveRootFixture {
    linked_dir: tempfile::TempDir,
    event: Event,
    publisher: String,
    root_cid: String,
}

struct ApprovalReceiptFixture {
    linked_dir: tempfile::TempDir,
    event: Event,
}

#[test]
fn ack_ready_receipt_is_sent_before_slow_roster_backfill() {
    assert_eq!(
        approval_receipt_relay_steps(true, NativeAppKeyLinkRelayEventApply::AppliedRoster, true,),
        [
            ApprovalReceiptRelayStep::SendAck,
            ApprovalReceiptRelayStep::BackfillRoster,
        ],
        "an already durable approval must not wait behind relay backfill before ACK"
    );
    assert_eq!(
        approval_receipt_relay_steps(true, NativeAppKeyLinkRelayEventApply::AppliedRoster, false,),
        [
            ApprovalReceiptRelayStep::BackfillRoster,
            ApprovalReceiptRelayStep::SendAck,
        ],
        "an incomplete approval needs roster history before its ACK can be built"
    );
}

fn approval_receipt_fixture() -> ApprovalReceiptFixture {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iOS owner".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Web peer".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let approved =
        approve_owner_from_pending_request(&owner_app, owner_dir.path(), linked_dir.path(), None);
    assert!(approved.error.is_empty(), "{}", approved.error);
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let event = Event::from_json(
        &owner_config
            .profile
            .as_ref()
            .unwrap()
            .pending_device_approval_receipts
            .first()
            .expect("owner queued approval receipt")
            .event_json,
    )
    .unwrap();

    ApprovalReceiptFixture { linked_dir, event }
}

fn authorized_drive_root_fixture() -> AuthorizedDriveRootFixture {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iOS owner".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("owner.txt"), b"owner content").unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut daemon = iris_drive_core::Daemon::open(owner_dir.path()).unwrap();
        daemon.import_source_dir(source.path()).await.unwrap();
    });

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Web peer".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);

    let approved =
        approve_owner_from_pending_request(&owner_app, owner_dir.path(), linked_dir.path(), None);
    assert!(approved.error.is_empty(), "{}", approved.error);
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let receipt_event = Event::from_json(
        &owner_state
            .pending_device_approval_receipts
            .first()
            .expect("owner queued approval receipt")
            .event_json,
    )
    .unwrap();
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    apply_native_app_key_link_relay_event_to_config(&mut linked_config, &receipt_event).unwrap();
    for op in &owner_state.profile_roster_ops {
        let event = Event::from_json(&op.event_json).unwrap();
        apply_native_app_key_link_relay_event_to_config(&mut linked_config, &event).unwrap();
    }
    assert_eq!(
        linked_config.profile.as_ref().unwrap().authorization_state,
        AppKeyAuthorizationState::Authorized
    );
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let owner_profile =
        iris_drive_core::Profile::load(owner_state.clone(), owner_dir.path()).unwrap();
    let owner_root = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&owner_state.app_key_pubkey)
        .unwrap()
        .clone();
    let authorized = owner_state.active_root_writer_app_key_pubkeys();
    let drive_root_event = iris_drive_core::nostr_events::build_drive_root_event(
        owner_profile.app_key.keys(),
        &owner_state.root_scope_id(),
        iris_drive_core::PRIMARY_DRIVE_ID,
        &owner_root,
        &authorized,
    )
    .unwrap();

    AuthorizedDriveRootFixture {
        linked_dir,
        event: drive_root_event,
        publisher: owner_state.app_key_pubkey.clone(),
        root_cid: owner_root.root_cid,
    }
}

#[test]
fn mobile_relay_handler_applies_authorized_drive_root_per_app_key() {
    let fixture = authorized_drive_root_fixture();
    let mut linked_config =
        AppConfig::load_or_default(config_path_in(fixture.linked_dir.path())).unwrap();
    let linked_app_key =
        iris_drive_core::AppKey::load(key_path_in(fixture.linked_dir.path())).unwrap();

    let outcome = apply_native_drive_root_relay_event_to_config(
        &mut linked_config,
        &fixture.event,
        linked_app_key.keys(),
    )
    .unwrap();

    assert_eq!(outcome, NativeAppKeyLinkRelayEventApply::AppliedDriveRoot);
    assert_eq!(
        linked_config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .get(&fixture.publisher)
            .unwrap()
            .root_cid,
        fixture.root_cid
    );

    assert_drive_root_replay_download_behavior(
        &linked_config,
        &fixture.event,
        linked_app_key.keys(),
        &fixture.publisher,
        &fixture.root_cid,
    );
}

#[test]
fn mobile_relay_persistence_preserves_approval_during_file_provider_root_update() {
    let fixture = approval_receipt_fixture();
    let config_dir = fixture.linked_dir.path();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            let disk_mutation =
                iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
                    .await
                    .unwrap();
            let mut file_provider_config =
                AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
            let config_mutation = Mutex::new(());
            let mut relay_apply = Box::pin(apply_and_persist_native_relay_event(
                config_dir,
                &config_mutation,
                &fixture.event,
            ));

            // Poll the real mobile relay persistence path while the independent
            // FileProvider process owns its config transaction. A correct path waits;
            // the previous in-process-only mutex completed here and was overwritten by
            // the FileProvider's stale snapshot below.
            let first_poll = std::future::Future::poll(
                relay_apply.as_mut(),
                &mut Context::from_waker(Waker::noop()),
            );
            let profile = file_provider_config.profile.as_ref().unwrap();
            let local_app_key = profile.app_key_pubkey.clone();
            let local_root_cid = hashtree_core::Cid::encrypted([0x77; 32], [0x88; 32]).to_string();
            file_provider_config
                .drives
                .iter_mut()
                .find(|drive| drive.drive_id == iris_drive_core::PRIMARY_DRIVE_ID)
                .unwrap()
                .app_key_roots
                .insert(
                    local_app_key,
                    iris_drive_core::AppKeyRootRef::legacy(local_root_cid, 77, 0),
                );
            file_provider_config
                .save(config_path_in(config_dir))
                .unwrap();
            drop(disk_mutation);
            match first_poll {
                Poll::Ready(result) => result,
                Poll::Pending => relay_apply.await,
            }
        })
        .unwrap();

    assert_eq!(
        result.outcome,
        NativeAppKeyLinkRelayEventApply::AppliedRoster
    );
    assert!(result.profile_id.is_some());
    assert!(!result.approval_was_ready);
    assert!(!result.approval_is_ready);
    assert!(result.drive_root_to_download.is_none());
    let saved = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    assert!(
        saved
            .profile
            .as_ref()
            .unwrap()
            .outbound_app_key_link_request
            .as_ref()
            .unwrap()
            .approval_receipt_event
            .iter()
            .any(|event_json| {
                Event::from_json(event_json).is_ok_and(|event| event.id == fixture.event.id)
            }),
        "FileProvider persistence must not overwrite the applied approval receipt"
    );
    assert!(
        saved
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .contains_key(&saved.profile.as_ref().unwrap().app_key_pubkey),
        "relay persistence must retain the concurrent FileProvider root update"
    );
}

#[test]
fn native_device_approval_preserves_concurrent_file_provider_root_update() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iOS owner".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Web peer".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let request = linked.ui.profile.unwrap().app_key_link_request;

    let file_provider_transaction =
        iris_drive_core::config_lock::ConfigMutationLock::acquire_blocking(owner_dir.path())
            .unwrap();
    let mut file_provider_config =
        AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let local_app_key = file_provider_config
        .profile
        .as_ref()
        .unwrap()
        .app_key_pubkey
        .clone();
    let local_root_cid = hashtree_core::Cid::encrypted([0x99; 32], [0xaa; 32]).to_string();
    file_provider_config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .insert(
            local_app_key.clone(),
            iris_drive_core::AppKeyRootRef::legacy(local_root_cid.clone(), 99, 0),
        );
    let (approved_tx, approved_rx) = std::sync::mpsc::channel();
    let approving_app = owner_app.clone();
    let approval = std::thread::spawn(move || {
        let state = approving_app.dispatch(NativeAppAction::ApproveDevice {
            request,
            label: "Web peer".to_owned(),
        });
        approved_tx.send(state).unwrap();
    });

    let early_approval = approved_rx
        .recv_timeout(std::time::Duration::from_millis(250))
        .ok();
    file_provider_config
        .save(config_path_in(owner_dir.path()))
        .unwrap();
    drop(file_provider_transaction);
    let approved = early_approval.unwrap_or_else(|| {
        approved_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
    });
    approval.join().unwrap();

    assert!(approved.error.is_empty(), "{}", approved.error);
    let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert!(
        !saved
            .profile
            .as_ref()
            .unwrap()
            .pending_device_approval_receipts
            .is_empty(),
        "FileProvider persistence must not discard the approval receipt"
    );
    assert_eq!(
        saved
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .get(&local_app_key)
            .unwrap()
            .root_cid,
        local_root_cid,
        "approval persistence must retain the concurrent FileProvider root"
    );
}
