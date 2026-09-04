use super::*;
use nostr_sdk::{Event, JsonUtil};
use tempfile::tempdir;

#[test]
fn approval_wrap_and_labels_are_web_decryptable() {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("Native owner".into())).unwrap();
    let web_device = Keys::generate();
    let web_pubkey = web_device.public_key().to_hex();

    profile
        .approve_app_key(&web_pubkey, Some("Web browser".into()))
        .unwrap();

    let projection = profile.state.profile_projection();
    let latest_epoch = projection.secret_epochs.values().next_back().unwrap();
    let signer = PublicKey::from_hex(&latest_epoch.signed_by_pubkey).unwrap();
    let wrapped = latest_epoch.wrapped_secrets.get(&web_pubkey).unwrap();
    let dck_hex = nip44::decrypt(web_device.secret_key(), &signer, wrapped)
        .expect("Web's string NIP-44 API must decrypt the native DCK wrap");
    assert_eq!(dck_hex.len(), 64);
    assert!(dck_hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let dck: [u8; 32] = hex::decode(&dck_hex).unwrap().try_into().unwrap();

    let payloads = profile
        .state
        .profile_roster_ops
        .iter()
        .flat_map(|op| {
            let event = Event::from_json(&op.event_json).unwrap();
            crate::nostr_identity::encrypted_device_label_payloads_from_nostr_identity_roster_op_event(
                &event,
            )
        })
        .filter_map(|encrypted| decrypt_drive_device_labels_with_dck(&encrypted, &dck).ok())
        .collect::<Vec<_>>();
    let current = payloads
        .iter()
        .max_by_key(|payload| (payload.secret_epoch, payload.updated_at))
        .expect("approval publishes current-DCK device labels");
    assert_eq!(
        current
            .labels
            .get(&profile.state.app_key_pubkey)
            .map(String::as_str),
        Some("Native owner")
    );
    assert_eq!(
        current.labels.get(&web_pubkey).map(String::as_str),
        Some("Web browser")
    );
}

#[test]
fn profile_dck_decoder_accepts_legacy_native_raw_bytes() {
    let legacy_dck = [0xff_u8; 32];
    assert_eq!(decode_dck_plaintext(&legacy_dck).unwrap(), legacy_dck);
}
