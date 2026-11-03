//! Generate disposable native wire data for cross-client tests. Uses only
//! well-known test identities, in-memory bytes, and the production encoders.
use anyhow::Result;
use hashtree_core::{Cid, EncryptionMode, encrypt_object, sha256};
use iris_drive_core::{
    AppKeyRootRef, NostrIdentityId, ShareRecipient, ShareRole, create_shared_folder,
    invite_shared_folder_member, nostr_events::build_drive_root_event, parse_share_invite,
    share_links::encode_compact_invite,
};
use nostr_sdk::Keys;
use serde_json::{Value, json};

fn main() -> Result<()> {
    // Public test scalars, unrelated to any installed account or credentials.
    let owner = Keys::parse(&format!("{:064x}", 1))?;
    let reader = Keys::parse(&format!("{:064x}", 2))?;
    let recipient = NostrIdentityId::new_v4();
    let mut folder = create_shared_folder(
        &owner,
        NostrIdentityId::new_v4(),
        "Picnic",
        "Picnic",
        None,
        vec![],
        1,
    )?;
    let invite = invite_shared_folder_member(
        &mut folder,
        &owner,
        ShareRecipient {
            profile_id: recipient,
            app_pubkey: reader.public_key().to_hex(),
            role: ShareRole::Reader,
            label: None,
            representative_npub_hint: None,
            display_name: None,
        },
        2,
    )?;
    let bundle = parse_share_invite(&invite.invite_url)?;
    let (link, ciphertext) = encode_compact_invite(&bundle)?;
    let plaintext = b"Picnic at noon.";
    let roots = [("chk", EncryptionMode::ChkV1), ("random", EncryptionMode::RndV1)]
        .map(|(name, mode)| -> Result<Value> {
            let (bytes, key) = encrypt_object(plaintext, mode)?;
            let cid = Cid::encrypted(sha256(&bytes), key);
            let root = AppKeyRootRef::legacy(cid.to_string(), 1_700_000_000, 1);
            let event = build_drive_root_event(&owner, &folder.share_id.to_string(), "main", &root, &[reader.public_key().to_hex()])?;
            Ok(json!({"suite": name, "cid": cid.to_string(), "ciphertext_hex": hex::encode(bytes), "event": event}))
        }).into_iter().collect::<Result<Vec<_>>>()?;
    println!(
        "{}",
        json!({
            "schema": 1, "recipient_test_scalar": 2, "recipient_pubkey": reader.public_key().to_hex(),
            "recipient_profile_id": recipient, "share_id": folder.share_id, "plaintext_hex": hex::encode(plaintext),
            "roots": roots, "invite": { "url": link, "ciphertext_hex": hex::encode(ciphertext), "role": "reader" }
        })
    );
    Ok(())
}
