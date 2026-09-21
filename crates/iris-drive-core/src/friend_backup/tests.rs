use super::*;
use crate::AppKey;
use nostr_sdk::Keys;
use nostr_sdk::nips::nip19::ToBech32;
use std::collections::BTreeMap;

fn npub() -> String {
    Keys::generate().public_key().to_bech32().unwrap()
}

#[test]
fn absent_config_and_old_install_start_without_offering_disk() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "schema_version = 4\n").unwrap();
    assert_eq!(
        FriendBackupConfig::load(dir.path()).unwrap(),
        FriendBackupConfig::default()
    );
    assert!(!dir.path().join("friends.toml").exists());
}

#[test]
fn concurrent_adds_preserve_both_private_contacts() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = dir.path().to_path_buf();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                upsert_friend(&path, &npub(), None, 0, true).unwrap();
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        FriendBackupConfig::load(dir.path()).unwrap().friends.len(),
        2
    );
}

#[test]
fn private_config_round_trips_without_changing_app_config() {
    let dir = tempfile::tempdir().unwrap();
    let app_config = "schema_version = 4\n";
    std::fs::write(dir.path().join("config.toml"), app_config).unwrap();
    let friend = npub();
    set_capacity(dir.path(), 500).unwrap();
    let saved = upsert_friend(dir.path(), &friend, Some(" Pat ".into()), 400, true).unwrap();
    assert_eq!(saved.friends[0].label.as_deref(), Some("Pat"));
    assert_eq!(FriendBackupConfig::load(dir.path()).unwrap(), saved);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).unwrap(),
        app_config
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(dir.path().join("friends.toml"))
            .unwrap()
            .permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);
    }
}

#[test]
fn plain_npub_and_invite_address_the_same_contact_without_offering_space() {
    let dir = tempfile::tempdir().unwrap();
    let friend = npub();
    let invite = encode_backup_invite(&friend).unwrap();
    assert_eq!(parse_backup_contact(&invite).unwrap(), friend);
    assert_eq!(
        parse_backup_contact(&format!(" {} ", friend.to_uppercase())).unwrap(),
        friend
    );
    upsert_friend(dir.path(), &friend, None, 0, true).unwrap();
    let config = upsert_friend(dir.path(), &invite, Some("Pat".into()), 0, true).unwrap();
    assert_eq!(config.capacity_bytes, 0);
    assert_eq!(config.friends.len(), 1);
    assert_eq!(config.friends[0].npub, friend);
    assert_eq!(config.friends[0].quota_bytes, 0);
}

#[test]
fn malformed_and_tampered_contacts_fail_closed() {
    let friend = npub();
    let invite = encode_backup_invite(&friend).unwrap();
    let mut bad_checksum = friend.clone();
    bad_checksum.pop();
    bad_checksum.push(if friend.ends_with('q') { 'p' } else { 'q' });
    for invalid in [
        bad_checksum,
        Keys::generate().secret_key().to_bech32().unwrap(),
        Keys::generate().public_key().to_hex(),
        format!("{invite}&quota=500"),
        format!("{invite}&npub={friend}"),
        format!("{invite}#fragment"),
        format!("iris-drive://backup?npub={friend}%20"),
        format!("https://example.com/backup?npub={friend}"),
    ] {
        assert!(
            parse_backup_contact(&invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn quotas_are_bounded_by_capacity_and_checked_for_overflow() {
    let first = FriendBackupPeer {
        npub: npub(),
        label: None,
        quota_bytes: u64::MAX,
        enabled: true,
    };
    let second = FriendBackupPeer {
        npub: npub(),
        label: None,
        quota_bytes: 1,
        enabled: true,
    };
    assert!(
        FriendBackupConfig {
            capacity_bytes: u64::MAX,
            friends: vec![first, second]
        }
        .validate()
        .is_err()
    );
    let dir = tempfile::tempdir().unwrap();
    set_capacity(dir.path(), 100).unwrap();
    let first = npub();
    upsert_friend(dir.path(), &first, None, 80, true).unwrap();
    assert!(upsert_friend(dir.path(), &npub(), None, 21, true).is_err());
    assert!(set_capacity(dir.path(), 79).is_err());
    assert_eq!(
        FriendBackupConfig::load(dir.path()).unwrap().capacity_bytes,
        100
    );
}

#[test]
fn held_data_prevents_quota_reduction_disable_or_removal() {
    let friend = npub();
    let mut config = FriendBackupConfig {
        capacity_bytes: 100,
        friends: vec![FriendBackupPeer {
            npub: friend.clone(),
            label: None,
            quota_bytes: 80,
            enabled: true,
        }],
    };
    let retained = BTreeMap::from([(friend.clone(), 70)]);
    config.validate_retained(&retained).unwrap();
    config.friends[0].quota_bytes = 69;
    assert!(config.validate_retained(&retained).is_err());
    config.friends[0].quota_bytes = 70;
    config.validate_retained(&retained).unwrap();
    config.friends[0].enabled = false;
    assert!(config.validate_retained(&retained).is_err());
    config.friends.clear();
    assert!(config.validate_retained(&retained).is_err());
    config.validate_retained(&BTreeMap::new()).unwrap();
}

#[test]
fn mutations_account_for_interrupted_writes_and_orphan_contacts() {
    let dir = tempfile::tempdir().unwrap();
    let friend = npub();
    set_capacity(dir.path(), 100).unwrap();
    let config = upsert_friend(dir.path(), &friend, None, 80, true).unwrap();
    let peer_dir = dir
        .path()
        .join("friend-backups")
        .join(config.friends[0].pubkey_hex().unwrap());
    std::fs::create_dir_all(&peer_dir).unwrap();
    std::fs::write(peer_dir.join("interrupted-write"), [0; 70]).unwrap();
    assert!(upsert_friend(dir.path(), &friend, None, 69, true).is_err());
    assert!(upsert_friend(dir.path(), &friend, None, 80, false).is_err());
    assert!(remove_friend(dir.path(), &friend).is_err());
    assert!(FriendBackupConfig::default().save(dir.path()).is_err());
    assert_eq!(FriendBackupConfig::load(dir.path()).unwrap(), config);
    // Existing bytes still count if settings were lost independently.
    std::fs::remove_file(dir.path().join("friends.toml")).unwrap();
    assert!(set_capacity(dir.path(), 100).is_err());
}

#[test]
fn config_rejects_duplicates_unknown_fields_and_oversized_labels() {
    let friend = npub();
    let peer = FriendBackupPeer {
        npub: friend.clone(),
        label: None,
        quota_bytes: 0,
        enabled: true,
    };
    assert!(
        FriendBackupConfig {
            capacity_bytes: 0,
            friends: vec![peer.clone(), peer]
        }
        .validate()
        .is_err()
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("friends.toml"),
        "capacity_bytes = 0\npublic_follow = true\n",
    )
    .unwrap();
    assert!(FriendBackupConfig::load(dir.path()).is_err());
    std::fs::remove_file(dir.path().join("friends.toml")).unwrap();
    assert!(upsert_friend(dir.path(), &friend, Some("x".repeat(129)), 0, true).is_err());
    assert!(upsert_friend(dir.path(), &friend, Some("Pat\nadmin".into()), 0, true).is_err());
    let config = FriendBackupConfig {
        capacity_bytes: 0,
        friends: (0..65)
            .map(|_| FriendBackupPeer {
                npub: npub(),
                label: None,
                quota_bytes: 0,
                enabled: true,
            })
            .collect(),
    };
    assert!(config.validate().is_err());
}

#[test]
fn backup_identity_is_stable_and_separate_from_app_identity() {
    let dir = tempfile::tempdir().unwrap();
    let device = AppKey::load_or_generate(dir.path().join("key")).unwrap();
    let keys = backup_keys(&device).unwrap();
    assert_ne!(keys.public_key(), device.keys().public_key());
    assert_eq!(
        keys.public_key(),
        backup_keys(&device).unwrap().public_key()
    );
    assert_eq!(
        backup_npub(dir.path()).unwrap(),
        keys.public_key().to_bech32().unwrap()
    );
    let another = AppKey::generate(dir.path().join("another"));
    assert_ne!(
        keys.public_key(),
        backup_keys(&another).unwrap().public_key()
    );
}

#[test]
fn reading_an_address_does_not_create_an_app_identity() {
    let dir = tempfile::tempdir().unwrap();
    assert!(backup_npub(dir.path()).is_err());
    assert!(!dir.path().join("key").exists());
}

#[test]
fn own_backup_address_cannot_be_accepted_as_a_friend() {
    let dir = tempfile::tempdir().unwrap();
    AppKey::load_or_generate(dir.path().join("key")).unwrap();
    let own = backup_npub(dir.path()).unwrap();
    assert!(upsert_friend(dir.path(), &own, None, 0, true).is_err());
    assert!(
        FriendBackupConfig::load(dir.path())
            .unwrap()
            .friends
            .is_empty()
    );
}
