use std::fs;
use std::io::Write;
use std::path::Path;

use bip39::{Language, Mnemonic};
use nostr_sdk::nips::nip06::FromMnemonic;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use nostr_sdk::{Keys, SecretKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::IrisProfileId;

pub const RECOVERY_PHRASE_WORD_COUNT: usize = 12;

#[derive(Debug, Error)]
pub enum RecoveryPhraseError {
    #[error("expected a 12-word recovery phrase")]
    WrongWordCount,
    #[error("invalid recovery phrase: {0}")]
    InvalidPhrase(String),
    #[error("invalid secret key: {0}")]
    InvalidSecret(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub fn generate_recovery_phrase() -> Result<String, RecoveryPhraseError> {
    let mnemonic = Mnemonic::generate_in(Language::English, RECOVERY_PHRASE_WORD_COUNT)
        .map_err(|error| RecoveryPhraseError::InvalidPhrase(error.to_string()))?;
    Ok(mnemonic.to_string())
}

pub fn validate_recovery_phrase(phrase: &str) -> Result<String, RecoveryPhraseError> {
    let normalized = normalize_recovery_phrase(phrase);
    let word_count = normalized.split_whitespace().count();
    if word_count != RECOVERY_PHRASE_WORD_COUNT {
        return Err(RecoveryPhraseError::WrongWordCount);
    }
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &normalized)
        .map_err(|error| RecoveryPhraseError::InvalidPhrase(error.to_string()))?;
    if mnemonic.word_count() != RECOVERY_PHRASE_WORD_COUNT {
        return Err(RecoveryPhraseError::WrongWordCount);
    }
    Ok(normalized)
}

pub fn recovery_phrase_to_keys(phrase: &str) -> Result<Keys, RecoveryPhraseError> {
    let normalized = validate_recovery_phrase(phrase)?;
    Keys::from_mnemonic(normalized, None)
        .map_err(|error| RecoveryPhraseError::InvalidPhrase(error.to_string()))
}

pub fn recovery_phrase_to_nsec(phrase: &str) -> Result<String, RecoveryPhraseError> {
    let keys = recovery_phrase_to_keys(phrase)?;
    keys.secret_key()
        .to_bech32()
        .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))
}

pub fn recovery_phrase_to_profile_id(phrase: &str) -> Result<IrisProfileId, RecoveryPhraseError> {
    let normalized = validate_recovery_phrase(phrase)?;
    let digest = Sha256::digest([b"iris-profile-id-v1".as_slice(), normalized.as_bytes()].concat());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(IrisProfileId::from_uuid(uuid::Uuid::from_bytes(bytes)))
}

pub fn normalize_recovery_phrase(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn secret_input_to_nsec(input: &str) -> Result<String, RecoveryPhraseError> {
    let trimmed = input.trim();
    let secret = if trimmed.starts_with("nsec1") {
        SecretKey::from_bech32(trimmed)
            .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))?
    } else if trimmed.len() == 64 && trimmed.chars().all(|char| char.is_ascii_hexdigit()) {
        SecretKey::from_hex(trimmed)
            .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))?
    } else {
        return recovery_phrase_to_nsec(trimmed);
    };
    secret
        .to_bech32()
        .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))
}

pub fn save_recovery_phrase(
    path: impl AsRef<Path>,
    phrase: &str,
) -> Result<(), RecoveryPhraseError> {
    let normalized = validate_recovery_phrase(phrase)?;
    write_secret_file(path.as_ref(), normalized.as_bytes())?;
    Ok(())
}

pub fn load_recovery_phrase(path: impl AsRef<Path>) -> Result<String, RecoveryPhraseError> {
    let raw = fs::read_to_string(path)?;
    validate_recovery_phrase(&raw)
}

#[cfg(unix)]
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generated_recovery_phrase_has_12_words_and_derives_key() {
        let phrase = generate_recovery_phrase().unwrap();

        assert_eq!(
            phrase.split_whitespace().count(),
            RECOVERY_PHRASE_WORD_COUNT
        );

        let nsec = recovery_phrase_to_nsec(&phrase).unwrap();
        assert!(nsec.starts_with("nsec1"));
        assert_eq!(secret_input_to_nsec(&phrase).unwrap(), nsec);
    }

    #[test]
    fn recovery_phrase_normalizes_case_and_whitespace() {
        let phrase = generate_recovery_phrase().unwrap();
        let shouty = phrase
            .split_whitespace()
            .map(str::to_uppercase)
            .collect::<Vec<_>>()
            .join(" \n ");

        assert_eq!(
            recovery_phrase_to_nsec(&shouty).unwrap(),
            recovery_phrase_to_nsec(&phrase).unwrap()
        );
    }

    #[test]
    fn recovery_phrase_requires_12_words() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

        let error = recovery_phrase_to_nsec(phrase).unwrap_err();

        assert!(matches!(error, RecoveryPhraseError::WrongWordCount));
    }

    #[test]
    fn recovery_phrase_uses_nip06_derivation() {
        let phrase =
            "leader monkey parrot ring guide accident before fence cannon height naive bean";

        let secret = SecretKey::from_bech32(recovery_phrase_to_nsec(phrase).unwrap()).unwrap();

        assert_eq!(
            secret.to_secret_hex(),
            "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a"
        );
    }

    #[test]
    fn recovery_phrase_derives_stable_uuid_profile_id() {
        let phrase =
            "leader monkey parrot ring guide accident before fence cannon height naive bean";

        let profile_id = recovery_phrase_to_profile_id(phrase).unwrap();
        let same_profile_id = recovery_phrase_to_profile_id(&phrase.to_uppercase()).unwrap();

        assert_eq!(profile_id, same_profile_id);
        assert_eq!(profile_id.as_uuid().get_version_num(), 4);
    }

    #[test]
    fn secret_input_accepts_nsec_and_hex() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let hex = keys.secret_key().to_secret_hex();

        assert_eq!(secret_input_to_nsec(&nsec).unwrap(), nsec);
        assert_eq!(secret_input_to_nsec(&hex).unwrap(), nsec);
    }

    #[test]
    fn recovery_phrase_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recovery_phrase");
        let phrase = generate_recovery_phrase().unwrap();

        save_recovery_phrase(&path, &phrase).unwrap();

        assert_eq!(load_recovery_phrase(&path).unwrap(), phrase);
    }
}
