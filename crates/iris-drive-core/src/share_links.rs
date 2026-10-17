//! Compact private invite references use the same encrypted nhash leaf format
//! as Iris Chat. Drive's signed recipient and permission payload stays distinct.
use crate::{
    AppConfig, AppKey, Daemon, SHARE_INVITE_PREFIX, ShareInviteBundle,
    paths::{config_path_in, key_path_in},
};
use anyhow::{Context, Result, bail};
use hashtree_core::{
    NHashData, Store, content_hash, decrypt_chk, encrypt_chk, nhash_decode, nhash_encode_full,
};
use std::{path::Path, time::Duration};

pub const SHARE_INVITE_WEB_PREFIX: &str = "https://drive.iris.to/#/share-invite/";

pub const MAX_INVITE_BYTES: usize = 128 * 1024;

pub fn encode_compact_invite(bundle: &ShareInviteBundle) -> Result<(String, Vec<u8>)> {
    let mut bundle = bundle.clone();
    bundle.shared_folder.app_key_roots.clear();
    let bytes = serde_json::to_vec(&bundle)?;
    if bytes.len() + 16 > MAX_INVITE_BYTES {
        bail!("Invite is too large");
    }
    let (ciphertext, key) = encrypt_chk(&bytes)?;
    let reference = nhash_encode_full(&NHashData {
        hash: content_hash(&ciphertext),
        decrypt_key: Some(key),
    })?;
    Ok((format!("{SHARE_INVITE_WEB_PREFIX}{reference}"), ciphertext))
}

pub fn compact_invite_reference(input: &str) -> Option<&str> {
    let input = input.trim();
    let reference = input
        .strip_prefix(SHARE_INVITE_PREFIX)
        .or_else(|| {
            input
                .split_once("#/share-invite/")
                .map(|(_, reference)| reference)
        })
        .unwrap_or(input);
    reference.starts_with("nhash1").then_some(reference)
}

fn decode_reference(reference: &str) -> Result<NHashData> {
    if reference.len() > 160 {
        bail!("Invalid invite reference");
    }
    let cid = nhash_decode(reference)?;
    if cid.decrypt_key.is_none() || nhash_encode_full(&cid)? != reference {
        bail!("Invalid private invite reference");
    }
    Ok(cid)
}

pub fn decode_compact_invite(reference: &str, ciphertext: &[u8]) -> Result<String> {
    let cid = decode_reference(reference)?;
    if ciphertext.len() > MAX_INVITE_BYTES || content_hash(ciphertext) != cid.hash {
        bail!("Invalid invite content");
    }
    let bytes = decrypt_chk(ciphertext, &cid.decrypt_key.context("Invite key missing")?)?;
    let bundle: ShareInviteBundle = serde_json::from_slice(&bytes)?;
    let invite = crate::encode_share_invite(&bundle)?;
    crate::parse_share_invite(&invite)?;
    Ok(invite)
}

pub async fn publish_compact_invite(config_dir: &Path, input: &str) -> Result<String> {
    let bundle = crate::parse_share_invite(input)?;
    let (invite, ciphertext) = encode_compact_invite(&bundle)?;
    let daemon = Daemon::open(config_dir)?;
    daemon
        .tree()
        .get_store()
        .put(content_hash(&ciphertext), ciphertext.clone())
        .await?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let key = AppKey::load(key_path_in(config_dir))?;
    let client = crate::blossom_sync_client(key.keys().clone(), &config.blossom_servers);
    tokio::time::timeout(
        Duration::from_secs(20),
        client.upload_if_missing(&ciphertext),
    )
    .await
    .context("Uploading invite timed out")?
    .context("Could not upload invite")?;
    Ok(invite)
}

pub async fn resolve_share_invite(config_dir: &Path, input: &str) -> Result<String> {
    let Some(reference) = compact_invite_reference(input) else {
        return Ok(input.to_owned());
    };
    let cid = decode_reference(reference)?;
    let daemon = Daemon::open(config_dir)?;
    if let Some(bytes) = daemon.tree().get_store().get(&cid.hash).await? {
        return decode_compact_invite(reference, &bytes);
    }
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    for server in &config.blossom_servers {
        let result = async {
            let mut response = client
                .get(format!(
                    "{}/{}.bin",
                    server.trim_end_matches('/'),
                    hashtree_core::to_hex(&cid.hash)
                ))
                .send()
                .await?
                .error_for_status()?;
            if response
                .content_length()
                .is_some_and(|size| size > MAX_INVITE_BYTES as u64)
            {
                bail!("Invite is too large");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > MAX_INVITE_BYTES {
                    bail!("Invite is too large");
                }
                bytes.extend_from_slice(&chunk);
            }
            let invite = decode_compact_invite(reference, &bytes)?;
            daemon.tree().get_store().put(cid.hash, bytes).await?;
            Ok::<_, anyhow::Error>(invite)
        }
        .await;
        if let Ok(invite) = result {
            return Ok(invite);
        }
    }
    bail!("Could not load invite. Check your connection and try again.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_invite_round_trip_is_short_and_rejects_tampering() {
        let owner = nostr_sdk::Keys::generate();
        let recipient = nostr_sdk::Keys::generate();
        let mut folder = crate::create_shared_folder(
            &owner,
            crate::NostrIdentityId::new_v4(),
            "Photos",
            "Photos",
            None,
            vec![],
            1,
        )
        .unwrap();
        let outcome = crate::invite_shared_folder_member(
            &mut folder,
            &owner,
            crate::ShareRecipient {
                profile_id: crate::NostrIdentityId::new_v4(),
                app_pubkey: recipient.public_key().to_hex(),
                role: crate::ShareRole::Editor,
                label: None,
                representative_npub_hint: None,
                display_name: None,
            },
            2,
        )
        .unwrap();
        let bundle = crate::parse_share_invite(&outcome.invite_url).unwrap();
        let (link, mut ciphertext) = encode_compact_invite(&bundle).unwrap();
        assert!(link.len() < 180);
        let reference = compact_invite_reference(&link).unwrap();
        let decoded = decode_compact_invite(reference, &ciphertext).unwrap();
        assert_eq!(crate::parse_share_invite(&decoded).unwrap(), bundle);
        ciphertext[0] ^= 1;
        assert!(decode_compact_invite(reference, &ciphertext).is_err());
    }

    #[test]
    fn private_reference_rejects_public_and_oversized_identifiers() {
        assert!(decode_reference(&hashtree_core::nhash_encode(&[1; 32]).unwrap()).is_err());
        assert!(decode_reference(&"x".repeat(161)).is_err());
    }
}
