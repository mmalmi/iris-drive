use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use thiserror::Error;

use crate::nostr_identity::{
    NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA, NostrIdentityEncryptedDeviceLabelsPayload,
    NostrIdentityId,
};

const DRIVE_DEVICE_LABEL_CIPHERTEXT_VERSION: &str = "v1";
const DRIVE_DEVICE_LABEL_NONCE_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum DeviceLabelCipherError {
    #[error("device-label payload: {0}")]
    Payload(String),
    #[error("device-label ciphertext is malformed")]
    MalformedCiphertext,
    #[error("device-label base64: {0}")]
    Base64(String),
    #[error("device-label encryption failed")]
    Encrypt,
    #[error("device-label decryption failed")]
    Decrypt,
}

pub type DriveDeviceLabelPayload = NostrIdentityEncryptedDeviceLabelsPayload;

#[must_use]
pub fn drive_device_label_payload(
    profile_id: NostrIdentityId,
    secret_epoch: u64,
    labels: BTreeMap<String, String>,
    updated_at: i64,
) -> DriveDeviceLabelPayload {
    DriveDeviceLabelPayload {
        schema: NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA,
        profile_id,
        secret_epoch,
        labels: normalize_drive_device_labels(labels),
        updated_at,
    }
}

pub fn encrypt_drive_device_labels_with_dck(
    payload: &DriveDeviceLabelPayload,
    dck: &[u8; 32],
) -> Result<String, DeviceLabelCipherError> {
    let payload = drive_device_label_payload(
        payload.profile_id,
        payload.secret_epoch,
        payload.labels.clone(),
        payload.updated_at,
    );
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|error| DeviceLabelCipherError::Payload(error.to_string()))?;
    let cipher = Aes256Gcm::new_from_slice(dck).map_err(|_| DeviceLabelCipherError::Encrypt)?;
    let mut nonce = [0_u8; DRIVE_DEVICE_LABEL_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| DeviceLabelCipherError::Encrypt)?;
    Ok(format!(
        "{DRIVE_DEVICE_LABEL_CIPHERTEXT_VERSION}.{}.{}",
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(encrypted)
    ))
}

pub fn decrypt_drive_device_labels_with_dck(
    ciphertext: &str,
    dck: &[u8; 32],
) -> Result<DriveDeviceLabelPayload, DeviceLabelCipherError> {
    let mut parts = ciphertext.split('.');
    let Some(version) = parts.next() else {
        return Err(DeviceLabelCipherError::MalformedCiphertext);
    };
    let Some(nonce) = parts.next() else {
        return Err(DeviceLabelCipherError::MalformedCiphertext);
    };
    let Some(encrypted) = parts.next() else {
        return Err(DeviceLabelCipherError::MalformedCiphertext);
    };
    if parts.next().is_some()
        || version != DRIVE_DEVICE_LABEL_CIPHERTEXT_VERSION
        || nonce.is_empty()
        || encrypted.is_empty()
    {
        return Err(DeviceLabelCipherError::MalformedCiphertext);
    }
    let nonce = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|error| DeviceLabelCipherError::Base64(error.to_string()))?;
    if nonce.len() != DRIVE_DEVICE_LABEL_NONCE_LEN {
        return Err(DeviceLabelCipherError::MalformedCiphertext);
    }
    let encrypted = URL_SAFE_NO_PAD
        .decode(encrypted)
        .map_err(|error| DeviceLabelCipherError::Base64(error.to_string()))?;
    let cipher = Aes256Gcm::new_from_slice(dck).map_err(|_| DeviceLabelCipherError::Decrypt)?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), encrypted.as_slice())
        .map_err(|_| DeviceLabelCipherError::Decrypt)?;
    let payload: DriveDeviceLabelPayload = serde_json::from_slice(&plaintext)
        .map_err(|error| DeviceLabelCipherError::Payload(error.to_string()))?;
    if payload.schema != NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA {
        return Err(DeviceLabelCipherError::Payload(
            "unsupported schema".to_string(),
        ));
    }
    Ok(drive_device_label_payload(
        payload.profile_id,
        payload.secret_epoch,
        payload.labels,
        payload.updated_at,
    ))
}

#[must_use]
pub fn normalize_drive_device_labels(labels: BTreeMap<String, String>) -> BTreeMap<String, String> {
    labels
        .into_iter()
        .filter_map(|(pubkey, label)| {
            let pubkey = pubkey.trim().to_ascii_lowercase();
            let label = label.trim().to_owned();
            (!pubkey.is_empty() && !label.is_empty()).then_some((pubkey, label))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_device_labels_roundtrip_with_dck() {
        let profile_id = NostrIdentityId::new_v4();
        let dck = [7_u8; 32];
        let payload = drive_device_label_payload(
            profile_id,
            3,
            BTreeMap::from([
                ("aa".to_string(), "  Mac  ".to_string()),
                ("bb".to_string(), "Phone".to_string()),
                ("cc".to_string(), " ".to_string()),
            ]),
            42,
        );

        let encrypted = encrypt_drive_device_labels_with_dck(&payload, &dck).unwrap();
        assert!(encrypted.starts_with("v1."));
        assert!(!encrypted.contains("Mac"));
        assert!(!encrypted.contains("Phone"));

        let decrypted = decrypt_drive_device_labels_with_dck(&encrypted, &dck).unwrap();
        assert_eq!(
            decrypted.schema,
            NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA
        );
        assert_eq!(decrypted.profile_id, profile_id);
        assert_eq!(decrypted.secret_epoch, 3);
        assert_eq!(decrypted.labels.get("aa").map(String::as_str), Some("Mac"));
        assert_eq!(
            decrypted.labels.get("bb").map(String::as_str),
            Some("Phone")
        );
        assert!(!decrypted.labels.contains_key("cc"));
    }

    #[test]
    fn drive_device_label_decrypt_rejects_wrong_dck() {
        let payload = drive_device_label_payload(NostrIdentityId::new_v4(), 1, BTreeMap::new(), 1);
        let encrypted = encrypt_drive_device_labels_with_dck(&payload, &[1_u8; 32]).unwrap();

        assert!(decrypt_drive_device_labels_with_dck(&encrypted, &[2_u8; 32]).is_err());
    }
}
