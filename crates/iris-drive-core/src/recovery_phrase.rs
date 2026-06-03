use bip39::{Language, Mnemonic};
use nostr_sdk::SecretKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use thiserror::Error;

pub const RECOVERY_PHRASE_WORD_COUNT: usize = 24;
const SECRET_KEY_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum RecoveryPhraseError {
    #[error("expected a 24-word recovery phrase")]
    WrongWordCount,
    #[error("invalid recovery phrase: {0}")]
    InvalidPhrase(String),
    #[error("invalid secret key: {0}")]
    InvalidSecret(String),
}

pub fn secret_to_recovery_phrase(secret: &SecretKey) -> Result<String, RecoveryPhraseError> {
    let mnemonic = Mnemonic::from_entropy_in(Language::English, secret.as_secret_bytes())
        .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))?;
    Ok(mnemonic.to_string())
}

pub fn recovery_phrase_to_nsec(phrase: &str) -> Result<String, RecoveryPhraseError> {
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
    let entropy = mnemonic.to_entropy();
    if entropy.len() != SECRET_KEY_BYTES {
        return Err(RecoveryPhraseError::InvalidSecret(format!(
            "expected {SECRET_KEY_BYTES} bytes, got {}",
            entropy.len()
        )));
    }
    let secret = SecretKey::from_slice(&entropy)
        .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))?;
    secret
        .to_bech32()
        .map_err(|error| RecoveryPhraseError::InvalidSecret(error.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::Keys;

    #[test]
    fn secret_round_trips_through_24_word_recovery_phrase() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let phrase = secret_to_recovery_phrase(keys.secret_key()).unwrap();

        assert_eq!(
            phrase.split_whitespace().count(),
            RECOVERY_PHRASE_WORD_COUNT
        );
        assert_eq!(recovery_phrase_to_nsec(&phrase).unwrap(), nsec);
        assert_eq!(secret_input_to_nsec(&phrase).unwrap(), nsec);
    }

    #[test]
    fn recovery_phrase_normalizes_case_and_whitespace() {
        let secret = SecretKey::from_hex("01".repeat(32)).expect("fixture secret should be valid");
        let phrase = secret_to_recovery_phrase(&secret).unwrap();
        let shouty = phrase
            .split_whitespace()
            .map(str::to_uppercase)
            .collect::<Vec<_>>()
            .join(" \n ");

        assert_eq!(
            recovery_phrase_to_nsec(&shouty).unwrap(),
            secret.to_bech32().unwrap()
        );
    }

    #[test]
    fn recovery_phrase_requires_24_words() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        let error = recovery_phrase_to_nsec(phrase).unwrap_err();

        assert!(matches!(error, RecoveryPhraseError::WrongWordCount));
    }

    #[test]
    fn secret_input_accepts_nsec_and_hex() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let hex = keys.secret_key().to_secret_hex();

        assert_eq!(secret_input_to_nsec(&nsec).unwrap(), nsec);
        assert_eq!(secret_input_to_nsec(&hex).unwrap(), nsec);
    }
}
