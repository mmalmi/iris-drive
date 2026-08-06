use super::*;
use crate::{AppConfig, Profile, ProfileState};
use nostr_sdk::ToBech32;
use tempfile::{TempDir, tempdir};

fn applied_approval_fixture() -> (TempDir, ProfileState, Profile, Event, AppConfig) {
    let owner_dir = tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("iPhone".into())).unwrap();
    let linked_dir = tempdir().unwrap();
    let mut linked = Profile::start_join_request(linked_dir.path(), Some("Mac".into())).unwrap();
    let approval = create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    linked.state.queue_unbound_app_key_join_request(
        10,
        approval.url,
        approval.request_keys.secret_key().to_secret_hex(),
    );
    owner
        .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
        .unwrap();
    let receipt =
        Event::from_json(&owner.state.pending_device_approval_receipts[0].event_json).unwrap();
    let mut linked_config = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    assert_eq!(
        crate::relay_sync::apply_remote_device_approval_receipt_event(
            &mut linked_config,
            &receipt,
        )
        .unwrap(),
        crate::relay_sync::NostrIdentityRosterOpApply::Applied,
    );
    (linked_dir, owner.state, linked, receipt, linked_config)
}

#[test]
fn roster_fingerprint_changes_with_profile_roster_ops() {
    let dir = tempdir().unwrap();
    let mut account = Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
    let before = app_key_link_roster_fingerprint(
        &app_actor,
        account.state.profile_id,
        &account.state.profile_roster_ops,
    );

    account
        .approve_app_key(&app_actor, Some("Browser".into()))
        .unwrap();
    let after = app_key_link_roster_fingerprint(
        &app_actor,
        account.state.profile_id,
        &account.state.profile_roster_ops,
    );

    assert_ne!(before, after);
}

#[test]
fn roster_ack_matches_current_profile_roster_fingerprint() {
    let dir = tempdir().unwrap();
    let mut account = Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
    account
        .approve_app_key(&app_actor, Some("Browser".into()))
        .unwrap();

    let recipients = app_key_link_roster_recipients(&account.state);
    let recipient = recipients
        .iter()
        .find(|recipient| recipient.app_key_pubkey == app_actor)
        .expect("approved app actor is a roster recipient");
    let frame = AppKeyLinkRosterAckFrame {
        schema: 1,
        admin_app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_pubkey: app_actor,
        roster_fingerprint: recipient.roster_fingerprint.clone(),
        acknowledged_at: 123,
    };

    assert!(app_key_link_roster_ack_matches_state(
        &account.state,
        &frame
    ));
}

#[test]
fn applied_ack_clears_only_the_exact_durably_applied_approval() {
    let (_, mut owner, linked, receipt, mut linked_config) = applied_approval_fixture();
    assert!(
        device_approval_applied_ack_event(
            linked_config.profile.as_ref().unwrap(),
            linked.app_key.keys(),
            &receipt,
            11,
        )
        .is_err(),
        "the receipt alone must not acknowledge an incomplete key epoch"
    );
    let roster = app_key_link_roster_frame(&owner, 11).unwrap();
    assert!(matches!(
        crate::relay_sync::apply_app_key_link_roster_frame(
            &mut linked_config,
            &roster,
            &owner.app_key_pubkey,
        )
        .unwrap(),
        crate::relay_sync::AppKeyLinkRosterApply::Applied(_)
    ));
    let linked_state = linked_config.profile.as_ref().unwrap();
    let ack = device_approval_applied_ack_event(linked_state, linked.app_key.keys(), &receipt, 11)
        .unwrap();

    assert!(apply_device_approval_applied_ack_event(&mut owner, &ack).unwrap());
    assert!(owner.pending_device_approval_receipts.is_empty());
    assert!(!apply_device_approval_applied_ack_event(&mut owner, &ack).unwrap());
}

#[test]
fn lost_applied_ack_is_rebuilt_after_full_roster_apply() {
    let (linked_dir, mut owner, linked, receipt, mut linked_config) = applied_approval_fixture();
    assert!(
        device_approval_applied_ack_event(
            linked_config.profile.as_ref().unwrap(),
            linked.app_key.keys(),
            &receipt,
            11,
        )
        .is_err()
    );

    let roster = app_key_link_roster_frame(&owner, 12).unwrap();
    assert!(matches!(
        crate::relay_sync::apply_app_key_link_roster_frame(
            &mut linked_config,
            &roster,
            &owner.app_key_pubkey,
        )
        .unwrap(),
        crate::relay_sync::AppKeyLinkRosterApply::Applied(_)
    ));
    let first_ack = device_approval_applied_ack_event(
        linked_config.profile.as_ref().unwrap(),
        linked.app_key.keys(),
        &receipt,
        11,
    )
    .unwrap();
    let linked_config_path = linked_dir.path().join("config.toml");
    linked_config.save(&linked_config_path).unwrap();
    let mut linked_config = AppConfig::load_or_default(&linked_config_path).unwrap();

    // The first ACK is lost. The owner therefore resends the exact receipt
    // after the linked device has restarted with the full roster applied.
    assert_eq!(
        crate::relay_sync::apply_remote_device_approval_receipt_event(
            &mut linked_config,
            &receipt,
        )
        .unwrap(),
        crate::relay_sync::NostrIdentityRosterOpApply::Current,
    );
    let replayed_ack = device_approval_applied_ack_event(
        linked_config.profile.as_ref().unwrap(),
        linked.app_key.keys(),
        &receipt,
        13,
    )
    .unwrap();
    let first = parse_nostr_identity_device_approval_applied_ack_event(&first_ack).unwrap();
    let replayed = parse_nostr_identity_device_approval_applied_ack_event(&replayed_ack).unwrap();
    assert_eq!(replayed.request_pubkey, first.request_pubkey);
    assert_eq!(replayed.device_app_key_pubkey, first.device_app_key_pubkey);
    assert_eq!(replayed.approval_event_id, first.approval_event_id);
    assert_eq!(replayed.approved_by_pubkey, first.approved_by_pubkey);

    assert!(apply_device_approval_applied_ack_event(&mut owner, &replayed_ack).unwrap());
    assert!(owner.pending_device_approval_receipts.is_empty());
}

#[test]
fn concurrent_same_profile_approvals_ack_every_durably_applied_receipt() {
    let first_owner_dir = tempdir().unwrap();
    let mut first_owner =
        Profile::create(first_owner_dir.path(), Some("first owner".into())).unwrap();
    let second_owner_dir = tempdir().unwrap();
    let second_app_key = crate::AppKey::generate(second_owner_dir.path().join("key"));
    second_app_key.save().unwrap();
    let second_pubkey = second_app_key.pubkey_hex();
    first_owner
        .approve_app_key(&second_pubkey, Some("second owner".into()))
        .unwrap();
    first_owner.appoint_admin(&second_pubkey).unwrap();
    let mut second_state = first_owner.state.clone();
    second_state.app_key_pubkey = second_pubkey;
    second_state.app_key_label = Some("second owner".into());
    second_state.outbound_app_key_link_request = None;
    second_state.inbound_app_key_link_requests.clear();
    second_state.handled_app_key_link_requests.clear();
    second_state.pending_device_approval_receipts.clear();
    second_state.sync_app_keys_from_profile();
    let mut second_owner = Profile {
        state: second_state,
        app_key: second_app_key,
    };
    assert!(second_owner.state.can_admin_profile());

    let linked_dir = tempdir().unwrap();
    let mut linked = Profile::start_join_request(linked_dir.path(), Some("Mac".into())).unwrap();
    let approval = create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    linked.state.queue_unbound_app_key_join_request(
        10,
        approval.url,
        approval.request_keys.secret_key().to_secret_hex(),
    );
    first_owner
        .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
        .unwrap();
    second_owner
        .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
        .unwrap();
    let first_receipt =
        Event::from_json(&first_owner.state.pending_device_approval_receipts[0].event_json)
            .unwrap();
    let second_receipt =
        Event::from_json(&second_owner.state.pending_device_approval_receipts[0].event_json)
            .unwrap();
    let mut linked_config = AppConfig {
        profile: Some(linked.state),
        ..AppConfig::default()
    };

    crate::relay_sync::apply_remote_device_approval_receipt_event(
        &mut linked_config,
        &first_receipt,
    )
    .unwrap();
    let mut current_roster_config = linked_config.clone();
    crate::relay_sync::apply_remote_device_approval_receipt_event(
        &mut linked_config,
        &second_receipt,
    )
    .unwrap();
    let second_roster_event = {
        let pending = current_roster_config
            .profile
            .as_ref()
            .unwrap()
            .outbound_app_key_link_request
            .as_ref()
            .unwrap();
        let receipt =
            parse_pending_app_key_approval_receipt_event(pending, &second_receipt).unwrap();
        Event::from_json(receipt.signed_roster_event.as_deref().unwrap()).unwrap()
    };
    assert_eq!(
        crate::relay_sync::apply_remote_nostr_identity_roster_op_event(
            &mut current_roster_config,
            &second_roster_event,
        )
        .unwrap(),
        crate::relay_sync::NostrIdentityRosterOpApply::Applied,
    );
    assert_eq!(
        crate::relay_sync::apply_remote_device_approval_receipt_event(
            &mut current_roster_config,
            &second_receipt,
        )
        .unwrap(),
        crate::relay_sync::NostrIdentityRosterOpApply::Applied,
        "inserting a new receipt must remain persistable when its roster op is current",
    );
    let mut merged_ops = std::collections::BTreeMap::new();
    for op in first_owner
        .state
        .profile_roster_ops
        .iter()
        .chain(&second_owner.state.profile_roster_ops)
    {
        merged_ops.insert(op.op_id.clone(), op.clone());
    }
    let roster = AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: first_owner.state.profile_id,
        admin_app_key_pubkey: first_owner.state.app_key_pubkey.clone(),
        profile_roster_ops: merged_ops.into_values().collect(),
        sent_at: 11,
    };
    assert!(matches!(
        crate::relay_sync::apply_app_key_link_roster_frame(
            &mut linked_config,
            &roster,
            &first_owner.state.app_key_pubkey,
        )
        .unwrap(),
        crate::relay_sync::AppKeyLinkRosterApply::Applied(_)
    ));

    let acknowledgements = device_approval_applied_ack_events(
        linked_config.profile.as_ref().unwrap(),
        linked.app_key.keys(),
        12,
    )
    .unwrap();
    assert_eq!(acknowledgements.len(), 2);
    for acknowledgement in &acknowledgements {
        apply_device_approval_applied_ack_event(&mut first_owner.state, acknowledgement).unwrap();
        apply_device_approval_applied_ack_event(&mut second_owner.state, acknowledgement).unwrap();
    }
    assert!(
        first_owner
            .state
            .pending_device_approval_receipts
            .is_empty()
    );
    assert!(
        second_owner
            .state
            .pending_device_approval_receipts
            .is_empty()
    );
}

#[test]
fn applied_ack_rejects_a_valid_receipt_that_did_not_bind_the_profile() {
    let first_owner_dir = tempdir().unwrap();
    let mut first_owner =
        Profile::create(first_owner_dir.path(), Some("first owner".into())).unwrap();
    let second_owner_dir = tempdir().unwrap();
    let mut second_owner =
        Profile::create(second_owner_dir.path(), Some("second owner".into())).unwrap();
    let linked_dir = tempdir().unwrap();
    let mut linked = Profile::start_join_request(linked_dir.path(), Some("Mac".into())).unwrap();
    let approval = create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    linked.state.queue_unbound_app_key_join_request(
        10,
        approval.url,
        approval.request_keys.secret_key().to_secret_hex(),
    );
    first_owner
        .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
        .unwrap();
    second_owner
        .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
        .unwrap();
    let first_receipt =
        Event::from_json(&first_owner.state.pending_device_approval_receipts[0].event_json)
            .unwrap();
    let second_receipt =
        Event::from_json(&second_owner.state.pending_device_approval_receipts[0].event_json)
            .unwrap();
    let mut linked_config = AppConfig {
        profile: Some(linked.state),
        ..AppConfig::default()
    };
    crate::relay_sync::apply_remote_device_approval_receipt_event(
        &mut linked_config,
        &first_receipt,
    )
    .unwrap();
    let roster = app_key_link_roster_frame(&first_owner.state, 11).unwrap();
    crate::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &roster,
        &first_owner.state.app_key_pubkey,
    )
    .unwrap();
    let state = linked_config.profile.as_ref().unwrap();

    assert!(
        device_approval_applied_ack_event(state, linked.app_key.keys(), &first_receipt, 12,)
            .is_ok()
    );
    assert!(
        device_approval_applied_ack_event(state, linked.app_key.keys(), &second_receipt, 12,)
            .is_err(),
        "a losing profile must never receive an ACK for its unapplied receipt"
    );
    let acknowledgements =
        device_approval_applied_ack_events(state, linked.app_key.keys(), 12).unwrap();
    assert_eq!(acknowledgements.len(), 1);
    assert_eq!(
        parse_nostr_identity_device_approval_applied_ack_event(&acknowledgements[0])
            .unwrap()
            .approved_by_pubkey,
        first_owner.state.app_key_pubkey,
    );
}

#[test]
fn pending_request_frame_carries_only_compact_bootstrap_material() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Mac".into())).unwrap();
    let linked_dir = tempdir().unwrap();
    let mut linked = Profile::link_to_profile(
        linked_dir.path(),
        owner.state.profile_id,
        owner.state.app_key_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    let approval_request = create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    assert_ne!(
        approval_request.bootstrap.request_secret,
        approval_request.request_keys.secret_key().to_secret_hex(),
        "the anti-spam request secret must be independent of the receipt key",
    );
    linked
        .state
        .queue_outbound_app_key_link_request(
            owner.state.app_key_pubkey.clone(),
            &crate::profile::app_key_link_invite_pubkey(&owner.state.app_key_link_secret).unwrap(),
            123,
            approval_request.url.clone(),
            approval_request.request_keys.secret_key().to_secret_hex(),
        )
        .unwrap();

    let frame = pending_app_key_link_request_frame(&linked.state)
        .expect("build pending frame")
        .expect("pending frame");
    let pending = linked
        .state
        .outbound_app_key_link_request
        .as_ref()
        .expect("persisted pending request");
    let (persisted_bootstrap, persisted_request_keys) =
        parse_pending_app_key_approval_bootstrap(pending).expect("persisted bootstrap material");
    let frame_url =
        app_key_link_request_frame_url(&frame, &linked.state.app_key_pubkey).expect("frame URL");
    let bootstrap = parse_app_key_approval_bootstrap(&frame_url)
        .expect("parse bootstrap")
        .expect("bootstrap");
    assert_eq!(bootstrap, approval_request.bootstrap);
    assert_eq!(bootstrap, persisted_bootstrap);
    assert_eq!(
        bootstrap.request_npub,
        persisted_request_keys.public_key().to_bech32().unwrap()
    );
    assert_eq!(
        bootstrap.device_app_key_npub,
        linked.app_key.keys().public_key().to_bech32().unwrap()
    );
    assert_ne!(bootstrap.device_app_key_npub, bootstrap.request_npub);
    assert_eq!(
        serde_json::to_value(&frame)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["i", "l", "r", "s", "v"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    assert!(frame_url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
    assert!(
        frame_url.len() <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
        "bootstrap URL was {}",
        frame_url.len()
    );
}

#[test]
fn pending_request_frame_compacts_long_utf8_device_labels() {
    let device_app_key = Keys::generate();
    let request_keys = Keys::generate();
    let frame = AppKeyLinkRequestFrame {
        schema: 1,
        invite_pubkey: Keys::generate().public_key().to_hex(),
        label: Some("Iris Drive 🚀 Release iPhone".to_owned()),
        request_npub: request_keys.public_key().to_bech32().unwrap(),
        request_secret: URL_SAFE_NO_PAD.encode([7_u8; 32]),
    };

    let url = app_key_link_request_frame_url(&frame, &device_app_key.public_key().to_hex())
        .expect("encode compact frame URL");
    let bootstrap = parse_app_key_approval_bootstrap(&url)
        .unwrap()
        .expect("bootstrap");

    assert_eq!(bootstrap.label.as_deref(), Some("Iris Drive 🚀"));
}

#[test]
fn approval_bootstrap_has_stable_app_npub_distinct_request_npub_and_32_byte_secret() {
    let app_key = nostr_sdk::Keys::generate();
    let local = create_app_key_approval_bootstrap(&app_key, Some("Web + Native"))
        .expect("encode bootstrap");
    let bootstrap = parse_app_key_approval_bootstrap(&local.url)
        .unwrap()
        .expect("bootstrap");
    assert_eq!(bootstrap, local.bootstrap);
    assert_eq!(bootstrap.label.as_deref(), Some("Web + Native"));
    assert_eq!(
        bootstrap.device_app_key_npub,
        app_key.public_key().to_bech32().unwrap()
    );
    assert_eq!(
        bootstrap.request_npub,
        local.request_keys.public_key().to_bech32().unwrap()
    );
    assert_ne!(bootstrap.device_app_key_npub, bootstrap.request_npub);
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(&bootstrap.request_secret)
            .unwrap()
            .len(),
        32
    );
    let payload = &local.url[APP_KEY_APPROVAL_REQUEST_PREFIX.len()..];
    let payload: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(
        payload
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["deviceAppKeyNpub", "label", "requestNpub", "requestSecret"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    assert!(
        local.url.len() <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
        "bootstrap URL was {}",
        local.url.len()
    );
}

#[test]
fn approval_parser_rejects_legacy_full_request_app_key_only_and_suffix_fallbacks() {
    let app_key = nostr_sdk::Keys::generate();
    let app_key_hex = app_key.public_key().to_hex();
    let legacy = format!("iris-drive://app-key-link?app_key={app_key_hex}&ignored=yes");
    assert!(!app_key_approval_input_has_prefix(&legacy));
    assert!(parse_app_key_approval_bootstrap(&legacy).unwrap().is_none());

    let local = create_app_key_approval_bootstrap(&app_key, Some("Phone")).unwrap();
    assert!(
        parse_app_key_approval_bootstrap(&format!("nostr:{}", local.url))
            .unwrap()
            .is_none()
    );
    assert!(
        parse_app_key_approval_bootstrap(&format!("{}?relay=wss://example.test", local.url))
            .is_err()
    );
    assert!(parse_app_key_approval_bootstrap(&format!("{}#scan", local.url)).is_err());

    let mut payload = serde_json::to_value(&local.bootstrap).unwrap();
    payload["requestedAt"] = 123.into();
    let full_request_url = format!(
        "{APP_KEY_APPROVAL_REQUEST_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );
    assert!(parse_app_key_approval_bootstrap(&full_request_url).is_err());

    let mut same_key = serde_json::to_value(&local.bootstrap).unwrap();
    same_key["requestNpub"] = same_key["deviceAppKeyNpub"].clone();
    let same_key_url = format!(
        "{APP_KEY_APPROVAL_REQUEST_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&same_key).unwrap())
    );
    assert!(parse_app_key_approval_bootstrap(&same_key_url).is_err());
    assert!(
        parse_app_key_approval_bootstrap("https://drive.iris.to/app-key-linker?owner=x")
            .unwrap()
            .is_none()
    );
}

#[test]
fn request_frame_rejects_legacy_fields_instead_of_falling_back() {
    let local = create_app_key_approval_bootstrap(&Keys::generate(), Some("Phone")).unwrap();
    let frame = AppKeyLinkRequestFrame {
        schema: 1,
        invite_pubkey: Keys::generate().public_key().to_hex(),
        label: local.bootstrap.label.clone(),
        request_npub: local.bootstrap.request_npub,
        request_secret: local.bootstrap.request_secret,
    };
    let mut value = serde_json::to_value(frame).unwrap();
    value["url"] = local.url.into();
    assert!(serde_json::from_value::<AppKeyLinkRequestFrame>(value).is_err());
}
