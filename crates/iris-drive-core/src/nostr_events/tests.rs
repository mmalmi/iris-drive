use super::*;
use crate::nostr_identity::{
    NostrIdentityCapabilities, NostrIdentityFacet, NostrIdentityId, NostrIdentityRosterOp,
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
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let authorized_app_keys = vec![device.public_key().to_hex()];
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
        // Set an explicit published_at so roundtrip is stable.
        1_700_000_000,
        7,
    );
    let event =
        build_drive_root_event(&device, &root_scope_id, "main", &root, &authorized_app_keys)
            .unwrap();
    assert_eq!(event.kind.as_u16(), KIND_DRIVE_ROOT);
    assert_eq!(tag_value(&event, "ms").as_deref(), Some("1700000000000"));
    let (device_pk, parsed_scope, drive_id, parsed_root) =
        parse_drive_root_event_for_device(&event, &device).unwrap();
    let preview = parse_drive_root_event_preview(&event).unwrap();
    assert_eq!(device_pk, device.public_key().to_hex());
    assert_eq!(parsed_scope, root_scope_id);
    assert_eq!(drive_id, "main");
    assert_eq!(parsed_root.root_cid, root.root_cid);
    assert_eq!(parsed_root.dck_generation, root.dck_generation);
    assert_eq!(parsed_root.published_at, root.published_at);
    assert_eq!(preview.published_at_ms, Some(1_700_000_000_000));
}

#[test]
fn drive_root_event_roundtrip_preserves_causal_fields() {
    let device = Keys::generate();
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let parent = RootParent {
        app_key_pubkey: device.public_key().to_hex(),
        app_key_seq: 2,
        root_cid: Cid::encrypted([0x20; 32], [0x21; 32]).to_string(),
    };
    let observed_device = Keys::generate().public_key().to_hex();
    let observed = BTreeMap::from([(
        observed_device.clone(),
        RootObservation {
            app_key_seq: 9,
            root_cid: Cid::encrypted([0x30; 32], [0x31; 32]).to_string(),
        },
    )]);
    let root = AppKeyRootRef {
        root_cid: Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
        published_at: 1_700_000_000,
        dck_generation: 7,
        app_key_seq: 3,
        parents: vec![parent.clone()],
        observed: observed.clone(),
        local_only: false,
    };

    let event = build_drive_root_event(
        &device,
        &root_scope_id,
        "main",
        &root,
        &[device.public_key().to_hex(), observed_device],
    )
    .unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(parsed_root.app_key_seq, 3);
    assert_eq!(parsed_root.parents, vec![parent]);
    assert_eq!(parsed_root.observed, observed);
}

#[test]
fn drive_root_event_does_not_publish_root_key_in_cleartext() {
    let device = Keys::generate();
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root_key = [0x44; 32];
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x33; 32], root_key).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_event(
        &device,
        &root_scope_id,
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
    let event = EventBuilder::new(Kind::from(30079u16), "{}".to_string())
        .tag(Tag::identifier(drive_root_d_tag(
            &NostrIdentityId::new_v4().to_string(),
            "main",
        )))
        .custom_created_at(nostr_sdk::Timestamp::from(1_700_000_000))
        .sign_with_keys(&device)
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
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x33; 32], [0x44; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let drive_event = build_drive_root_event(
        &device,
        &root_scope_id,
        "main",
        &root,
        &[device.public_key().to_hex()],
    )
    .unwrap();
    assert!(is_drive_root_event_coordinate(&drive_event));

    let files_event = build_private_hashtree_root_event(&owner, "main", &root).unwrap();
    assert!(!is_drive_root_event_coordinate(&files_event));

    let profile_id = NostrIdentityId::new_v4();
    let profile_event = crate::build_nostr_identity_roster_op_event(
        &owner,
        profile_id,
        Vec::new(),
        Some(1),
        NostrIdentityRosterOp::AddFacet {
            facet: NostrIdentityFacet::app_key(
                owner.public_key().to_hex(),
                1_700_000_000,
                Some("owner app".to_string()),
                NostrIdentityCapabilities::app_admin(),
            ),
        },
        1_700_000_000,
    )
    .unwrap();
    assert!(!is_drive_root_event_coordinate(&profile_event));
}

#[test]
fn private_hashtree_root_event_is_files_app_compatible() {
    let owner = Keys::generate();
    let root_key = [0x44; 32];
    let root_hash = [0x33; 32];
    let root = AppKeyRootRef::legacy(
        Cid::encrypted(root_hash, root_key).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_private_hashtree_root_event(&owner, "main", &root).unwrap();
    assert_eq!(event.kind.as_u16(), KIND_HASHTREE_ROOT);
    assert_eq!(event.pubkey, owner.public_key());
    assert_eq!(event.tags.identifier(), Some("main"));
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
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(Cid::public([0x11; 32]).to_string(), 1_700_000_000, 1);

    assert!(
        build_drive_root_event(
            &device,
            &root_scope_id,
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
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x22; 32], [0x33; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_event(&device, &root_scope_id, "main", &root, &[]).unwrap();
    let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
    assert_eq!(parsed_root.root_cid, root.root_cid);
}

#[test]
fn drive_root_event_with_zero_published_at_uses_wall_clock() {
    let device = Keys::generate();
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x56; 32], [0x78; 32]).to_string(),
        0, // caller has not stamped; use wall-clock time
        1,
    );
    let event = build_drive_root_event(
        &device,
        &root_scope_id,
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
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(
        Cid::encrypted([0x56; 32], [0x78; 32]).to_string(),
        1_700_000_000,
        1,
    );

    let event = build_drive_root_publish_event(
        &device,
        &root_scope_id,
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
    let scope = NostrIdentityId::new_v4().to_string();
    let tag = drive_root_d_tag(&scope, "main");
    assert_eq!(tag, format!("iris-drive/{scope}/main/root"));
}

#[test]
fn drive_root_d_tag_parse_round_trip() {
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let drive_id = "shared-photos";
    let tag = drive_root_d_tag(&root_scope_id, drive_id);
    let (parsed_scope, parsed_drive) = parse_drive_root_d_tag(&tag).unwrap();
    assert_eq!(parsed_scope, root_scope_id);
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
    let other = EventBuilder::new(Kind::from(1u16), "{}".to_string())
        .tag(Tag::identifier(drive_root_d_tag(
            &NostrIdentityId::new_v4().to_string(),
            "main",
        )))
        .sign_with_keys(&device)
        .unwrap();
    assert!(matches!(
        parse_drive_root_event(&other),
        Err(WireError::WrongKind { .. })
    ));
}

#[test]
fn drive_root_event_attributes_to_device_signer() {
    // Important property: even if two AppKeys publish for the same
    // root scope + drive, the event's author is the AppKey pubkey, so the
    // merge engine can attribute each root to the right app actor.
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let root_scope_id = NostrIdentityId::new_v4().to_string();
    let root = AppKeyRootRef::legacy(Cid::encrypted([0x88; 32], [0x99; 32]).to_string(), 0, 1);
    let ev_a = build_drive_root_event(
        &device_a,
        &root_scope_id,
        "main",
        &root,
        &[device_a.public_key().to_hex()],
    )
    .unwrap();
    let ev_b = build_drive_root_event(
        &device_b,
        &root_scope_id,
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
