//! Event content only the profile's own devices can read: encrypted with a
//! fresh key, the key wrapped for each device with NIP-44. Iris Drive web
//! builds the same envelope (`src/drive/deviceStorageReport.ts`).
//!
//! `{"v":1,"key_wraps":{<device>:<nip44 of the hex key>},"data":"<nonce>.<ciphertext>"}`,
//! nonce and ciphertext base64url without padding, AES-256-GCM.

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Keys, PublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const NONCE_LEN: usize = 12;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SealedContentError {
    /// Not wrapped for this device: sent before it was linked.
    #[error("not readable by this device yet")]
    KeyUnavailable,
    #[error("{0}")]
    Invalid(String),
}

fn invalid(error: impl std::fmt::Display) -> SealedContentError {
    SealedContentError::Invalid(error.to_string())
}

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u8,
    key_wraps: BTreeMap<String, String>,
    data: String,
}

/// `plaintext` sealed for `recipients` and the sender itself, as event content.
pub fn seal(
    keys: &Keys,
    recipients: &[String],
    plaintext: &[u8],
) -> Result<String, SealedContentError> {
    let mut key = [0_u8; 32];
    OsRng.fill_bytes(&mut key);
    let mut nonce = [0_u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = Aes256Gcm::new_from_slice(&key)
        .map_err(invalid)?
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(invalid)?;
    let mut key_wraps = BTreeMap::new();
    let own = keys.public_key().to_hex();
    for recipient in recipients.iter().chain(std::iter::once(&own)) {
        if key_wraps.contains_key(recipient) {
            continue;
        }
        let pubkey = PublicKey::from_hex(recipient).map_err(invalid)?;
        let wrap = nip44::encrypt(
            keys.secret_key(),
            &pubkey,
            hex::encode(key),
            Nip44Version::V2,
        )
        .map_err(invalid)?;
        key_wraps.insert(recipient.clone(), wrap);
    }
    serde_json::to_string(&Wire {
        v: 1,
        key_wraps,
        data: format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ),
    })
    .map_err(invalid)
}

/// The plaintext of `content` that `author` sealed, opened with this device's key.
pub fn open(content: &str, author: &PublicKey, keys: &Keys) -> Result<Vec<u8>, SealedContentError> {
    let wire: Wire = serde_json::from_str(content).map_err(invalid)?;
    if wire.v != 1 {
        return Err(invalid(format!("unsupported version {}", wire.v)));
    }
    let wrap = wire
        .key_wraps
        .get(&keys.public_key().to_hex())
        .ok_or(SealedContentError::KeyUnavailable)?;
    let key_hex = nip44::decrypt(keys.secret_key(), author, wrap).map_err(invalid)?;
    let key = hex::decode(key_hex).map_err(invalid)?;
    let (nonce, ciphertext) = wire
        .data
        .split_once('.')
        .ok_or_else(|| invalid("malformed"))?;
    let nonce = URL_SAFE_NO_PAD.decode(nonce).map_err(invalid)?;
    if nonce.len() != NONCE_LEN || key.len() != 32 {
        return Err(invalid("malformed"));
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(ciphertext).map_err(invalid)?;
    Aes256Gcm::new_from_slice(&key)
        .map_err(invalid)?
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| invalid("could not be decrypted"))
}
