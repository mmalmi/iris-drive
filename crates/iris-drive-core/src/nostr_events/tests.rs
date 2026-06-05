use super::*;
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileFacet, IrisProfileId, IrisProfileRosterOp,
};
use nostr_sdk::JsonUtil;
use std::collections::BTreeMap;

fn tag_value(event: &Event, tag_name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let fields = tag.as_slice();
        if fields.first().is_some_and(|name| name == tag_name) {
            fields.get(1).cloned()
        } else {
            None
        }
    })
}

#[test]
fn drive_root_event_roundtrip() {
    let device = Keys::generate();
    let root_scope_id = IrisProfileId::new_v4().to_string();
    let authorized_devices = vec![device.public_key().to_hex()];
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
        // Set an explicit published_at so roundtrip is stable.
        1_700_000_000,
        7,
    );
    let event = build_drive_root_event(&device, &root_scope_id, "main", &root, &authorized_devices)
        .unwrap();
    assert_eq!(event.kind.as_u16(), KIND_DRIVE_ROOT);
    let (device_pk, parsed_owner, drive_id, parsed_root) =
        parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(device_pk, device.public_key().to_hex());
    assert_eq!(parsed_owner, root_scope_id);
    assert_eq!(drive_id, "main");
    assert_eq!(parsed_root.root_cid, root.root_cid);
    assert_eq!(parsed_root.dck_generation, root.dck_generation);
    assert_eq!(parsed_root.published_at, root.published_at);
}

#[test]
fn drive_root_event_roundtrip_preserves_causal_fields() {
    let device = Keys::generate();
    let owner = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let parent = RootParent {
        device_id: device.public_key().to_hex(),
        device_seq: 2,
        root_cid: Cid::encrypted([0x20; 32], [0x21; 32]).to_string(),
    };
    let observed_device = Keys::generate().public_key().to_hex();
    let observed = BTreeMap::from([(
        observed_device.clone(),
        RootObservation {
            device_seq: 9,
            root_cid: Cid::encrypted([0x30; 32], [0x31; 32]).to_string(),
        },
    )]);
    let root = DeviceRootRef {
        root_cid: Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
        published_at: 1_700_000_000,
        dck_generation: 7,
        device_seq: 3,
        parents: vec![parent.clone()],
        observed: observed.clone(),
        local_only: false,
    };

    let event = build_drive_root_event(
        &device,
        &owner_hex,
        "main",
        &root,
        &[device.public_key().to_hex(), observed_device],
    )
    .unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(parsed_root.device_seq, 3);
    assert_eq!(parsed_root.parents, vec![parent]);
    assert_eq!(parsed_root.observed, observed);
}

#[test]
fn drive_root_event_does_not_publish_root_key_in_cleartext() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root_key = [0x44; 32];
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x33; 32], root_key).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_event(
        &device,
        &owner,
        "main",
        &root,
        &[device.public_key().to_hex()],
    )
    .unwrap();

    assert!(!event.content.contains(&root.root_cid));
    assert!(!event.content.contains(&hex::encode(root_key)));
    assert!(parse_drive_root_event(&event).is_err());

    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(parsed_root.root_cid, root.root_cid);
}

#[test]
fn retired_drive_root_kind_is_rejected() {
    let device = Keys::generate();
    let event = EventBuilder::new(
        Kind::from(30079u16),
        "{}".to_string(),
        [Tag::identifier(drive_root_d_tag(
            &IrisProfileId::new_v4().to_string(),
            "main",
        ))],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(1_700_000_000))
    .to_event(&device)
    .unwrap();

    assert!(matches!(
        parse_drive_root_event_for_device(&event, &device),
        Err(WireError::WrongKind {
            expected: KIND_DRIVE_ROOT,
            got: 30079
        })
    ));
}

#[test]
fn drive_root_coordinate_does_not_match_other_30078_records() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x33; 32], [0x44; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let drive_event = build_drive_root_event(
        &device,
        &owner_hex,
        "main",
        &root,
        &[device.public_key().to_hex()],
    )
    .unwrap();
    assert!(is_drive_root_event_coordinate(&drive_event));

    let files_event = build_private_hashtree_root_event(&owner, "main", &root).unwrap();
    assert!(!is_drive_root_event_coordinate(&files_event));

    let profile_id = IrisProfileId::new_v4();
    let profile_event = crate::build_iris_profile_roster_op_event(
        &owner,
        profile_id,
        Vec::new(),
        Some(1),
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                owner.public_key().to_hex(),
                1_700_000_000,
                Some("owner app".to_string()),
                IrisProfileCapabilities::app_admin(),
            ),
        },
        1_700_000_000,
    )
    .unwrap();
    assert!(!is_drive_root_event_coordinate(&profile_event));
}

#[test]
fn app_key_link_request_event_round_trips_and_is_its_own_coordinate() {
    let device = Keys::generate();
    let frame = crate::app_key_link_transport::AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: crate::IrisProfileId::new_v4(),
        app_key_pubkey: device.public_key().to_hex(),
        link_secret: "join-secret".to_string(),
        label: Some("phone".to_string()),
        requested_at: 123,
        url: "iris-drive://app-key-link?app_key=example".to_string(),
    };

    let event = build_app_key_link_request_event(&device, &frame).unwrap();

    assert!(is_app_key_link_request_event_coordinate(&event));
    assert!(!is_drive_root_event_coordinate(&event));
    assert_eq!(
        event.identifier(),
        Some(app_key_link_request_d_tag(frame.profile_id).as_str())
    );
    assert_eq!(parse_app_key_link_request_event(&event).unwrap(), frame);
}

#[test]
fn app_key_link_request_event_d_tag_is_profile_scoped() {
    let device = Keys::generate();
    let frame = crate::app_key_link_transport::AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: crate::IrisProfileId::new_v4(),
        app_key_pubkey: device.public_key().to_hex(),
        link_secret: "join-secret".to_string(),
        label: None,
        requested_at: 123,
        url: "iris-drive://app-key-link?app_key=example".to_string(),
    };
    let other_profile = crate::IrisProfileId::new_v4();
    let event = EventBuilder::new(
        Kind::from(KIND_APP_KEY_LINK_REQUEST),
        serde_json::to_string(&frame).unwrap(),
        [Tag::identifier(app_key_link_request_d_tag(other_profile))],
    )
    .to_event(&device)
    .unwrap();

    assert!(matches!(
        parse_app_key_link_request_event(&event),
        Err(WireError::AppKeyLinkProfileMismatch { .. })
    ));
}

#[test]
fn app_key_link_request_event_must_be_signed_by_requesting_device() {
    let device = Keys::generate();
    let attacker = Keys::generate();
    let frame = crate::app_key_link_transport::AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: crate::IrisProfileId::new_v4(),
        app_key_pubkey: device.public_key().to_hex(),
        link_secret: "join-secret".to_string(),
        label: None,
        requested_at: 123,
        url: "iris-drive://app-key-link?app_key=example".to_string(),
    };

    let event = build_app_key_link_request_event(&attacker, &frame).unwrap();

    assert!(matches!(
        parse_app_key_link_request_event(&event),
        Err(WireError::AppKeyLinkSignerMismatch { .. })
    ));
}

#[test]
fn private_hashtree_root_event_is_files_app_compatible() {
    let owner = Keys::generate();
    let root_key = [0x44; 32];
    let root_hash = [0x33; 32];
    let root = DeviceRootRef::legacy(
        Cid::encrypted(root_hash, root_key).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_private_hashtree_root_event(&owner, "main", &root).unwrap();
    assert_eq!(event.kind.as_u16(), 30078);
    assert_eq!(event.pubkey, owner.public_key());
    assert_eq!(event.identifier(), Some("main"));
    assert_eq!(event.content, "");
    assert_eq!(tag_value(&event, "l").as_deref(), Some("hashtree"));
    assert_eq!(tag_value(&event, "hash"), Some(hex::encode(root_hash)));
    assert!(tag_value(&event, "key").is_none());
    assert!(!event.as_json().contains(&hex::encode(root_key)));

    let parsed = hashtree_nostr::parse_verified_hashtree_root_event(&event)
        .unwrap()
        .unwrap();
    let resolved = hashtree_nostr::resolve_self_encrypted_root_cid(&parsed, &owner).unwrap();
    assert_eq!(parsed.event.pubkey, owner.public_key().to_hex());
    assert_eq!(parsed.tree_name, "main");
    assert_eq!(resolved.to_string(), root.root_cid);
}

#[test]
fn drive_root_event_builder_rejects_unencrypted_root() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root = DeviceRootRef::legacy(Cid::public([0x11; 32]).to_string(), 1_700_000_000, 1);

    assert!(
        build_drive_root_event(
            &device,
            &owner,
            "main",
            &root,
            &[device.public_key().to_hex()]
        )
        .is_err()
    );
}

#[test]
fn drive_root_event_builder_always_wraps_for_signing_device() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x22; 32], [0x33; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_event(&device, &owner, "main", &root, &[]).unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(parsed_root.root_cid, root.root_cid);
}

#[test]
fn drive_root_event_with_zero_published_at_uses_wall_clock() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x56; 32], [0x78; 32]).to_string(),
        0, // caller has not stamped; use wall-clock time
        1,
    );
    let event = build_drive_root_event(
        &device,
        &owner,
        "main",
        &root,
        &[device.public_key().to_hex()],
    )
    .unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    // Should be roughly now, not 0.
    assert!(parsed_root.published_at > 1_500_000_000);
}

#[test]
fn drive_root_publish_event_advances_past_stored_root_timestamp() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root = DeviceRootRef::legacy(
        Cid::encrypted([0x56; 32], [0x78; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_publish_event(
        &device,
        &owner,
        "main",
        &root,
        &[device.public_key().to_hex()],
    )
    .unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();

    assert!(parsed_root.published_at > root.published_at);
}

#[test]
fn drive_root_d_tag_format() {
    let scope = IrisProfileId::new_v4().to_string();
    let tag = drive_root_d_tag(&scope, "main");
    assert_eq!(tag, format!("iris-drive/{scope}/main/root"));
}

#[test]
fn drive_root_d_tag_parse_round_trip() {
    let owner = IrisProfileId::new_v4().to_string();
    let drive_id = "shared-photos";
    let tag = drive_root_d_tag(&owner, drive_id);
    let (parsed_owner, parsed_drive) = parse_drive_root_d_tag(&tag).unwrap();
    assert_eq!(parsed_owner, owner);
    assert_eq!(parsed_drive, drive_id);
}

#[test]
fn drive_root_d_tag_malformed_rejected() {
    for bad in &[
        "wrong-prefix/abc/main/root",
        "iris-drive/abc/main",
        "iris-drive//main/root",
        "iris-drive/abc//root",
        "iris-drive/abc",
    ] {
        assert!(parse_drive_root_d_tag(bad).is_err(), "should reject {bad}");
    }
}

#[test]
fn drive_root_event_wrong_kind_rejected() {
    let device = Keys::generate();
    let other = EventBuilder::new(
        Kind::from(1u16),
        "{}".to_string(),
        [Tag::identifier(drive_root_d_tag(
            &device.public_key().to_hex(),
            "main",
        ))],
    )
    .to_event(&device)
    .unwrap();
    assert!(matches!(
        parse_drive_root_event(&other),
        Err(WireError::WrongKind { .. })
    ));
}

#[test]
fn drive_root_event_attributes_to_device_signer() {
    // Important property: even if two devices publish for the same
    // owner+drive, the event's author is the device pubkey, so the
    // merge engine can attribute each root to the right device.
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let root = DeviceRootRef::legacy(Cid::encrypted([0x88; 32], [0x99; 32]).to_string(), 0, 1);
    let ev_a = build_drive_root_event(
        &device_a,
        &owner,
        "main",
        &root,
        &[device_a.public_key().to_hex()],
    )
    .unwrap();
    let ev_b = build_drive_root_event(
        &device_b,
        &owner,
        "main",
        &root,
        &[device_b.public_key().to_hex()],
    )
    .unwrap();
    let (pk_a, _, _, _) = parse_drive_root_event_for_device(&ev_a, &device_a).unwrap();
    let (pk_b, _, _, _) = parse_drive_root_event_for_device(&ev_b, &device_b).unwrap();
    assert_eq!(pk_a, device_a.public_key().to_hex());
    assert_eq!(pk_b, device_b.public_key().to_hex());
    assert_ne!(pk_a, pk_b);
}
