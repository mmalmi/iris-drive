//! Generate disposable native data for checking that other clients open a
//! native My Drive root with only the recovery phrase. The phrase is the
//! public NIP-06 test vector; every key here is throwaway.
//!
//! `cargo run -p iris-drive-core --example recovery_root_fixture`
use anyhow::{Context, Result};
use hashtree_core::{Cid, EncryptionMode, encrypt_object, sha256};
use iris_drive_core::nostr_events::{
    KIND_DRIVE_ROOT, build_drive_root_event, parse_drive_root_event_for_device,
};
use iris_drive_core::recovery_acceptance::{
    recovery_phrase_add_op, sign_recovery_phrase_acceptance,
};
use iris_drive_core::recovery_phrase::recovery_phrase_to_keys;
use iris_drive_core::{AppKeyRootRef, Drive, Profile, drive_root_key_recipient_pubkeys};
use nostr_sdk::JsonUtil;
use serde_json::{Value, json};

const PHRASE: &str =
    "leader monkey parrot ring guide accident before fence cannon height naive bean";

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut device = Profile::create(dir.path(), Some("Laptop".into()))?;
    let recovery_keys = recovery_phrase_to_keys(PHRASE)?;
    let recovery_pubkey = device.add_recovery_phrase(PHRASE)?;
    let add_op = recovery_phrase_add_op(&device.state, &recovery_pubkey).context("add op")?;
    let acceptance =
        sign_recovery_phrase_acceptance(&recovery_keys, device.state.profile_id, add_op)?;

    let plaintext = b"Files a recovery phrase brings back.";
    let (bytes, key) = encrypt_object(plaintext, EncryptionMode::ChkV1)?;
    let cid = Cid::encrypted(sha256(&bytes), key);
    let root = AppKeyRootRef::legacy(cid.to_string(), 1_794_682_800, 2);
    let drive = Drive::primary(device.state.root_scope_id());
    let recipients = drive_root_key_recipient_pubkeys(&device.state, &drive);
    let root_event = build_drive_root_event(
        device.app_key.keys(),
        &device.state.root_scope_id(),
        "main",
        &root,
        &recipients,
    )?;
    let (_, _, _, opened) = parse_drive_root_event_for_device(&root_event, &recovery_keys)?;
    anyhow::ensure!(
        opened.root_cid == root.root_cid,
        "phrase did not open the root"
    );

    let roster_events = device
        .state
        .profile_roster_ops
        .iter()
        .map(|op| serde_json::from_str(&op.event_json))
        .collect::<serde_json::Result<Vec<Value>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "source": "Emitted by `cargo run -p iris-drive-core --example recovery_root_fixture`. A native device adds the recovery phrase, publishes a My Drive root wrapped for itself and the phrase, and the phrase signs its facet acceptance. Disposable keys; the phrase is the public NIP-06 test vector.",
            "phrase": PHRASE,
            "recoveryPubkey": recovery_pubkey,
            "deviceAppKeyPubkey": device.state.app_key_pubkey,
            "profileId": device.state.profile_id,
            "rosterEvents": roster_events,
            "acceptanceEvent": serde_json::from_str::<Value>(&acceptance.as_json())?,
            "rootEvent": serde_json::from_str::<Value>(&root_event.as_json())?,
            "rootEventKind": KIND_DRIVE_ROOT,
            "rootDTag": format!("iris-drive/{}/main/root", device.state.root_scope_id()),
            "rootHashHex": hex::encode(cid.hash),
            "rootKeyHex": hex::encode(key),
            "rootCid": cid.to_string(),
            "rootCiphertextHex": hex::encode(bytes),
            "rootPlaintextHex": hex::encode(plaintext),
        }))?
    );
    Ok(())
}
