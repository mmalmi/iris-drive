//! `AppKey` and recovery-authority Nostr keypairs.
//!
//! Iris Drive has two local key kinds:
//!
//! - [`AppKey`] (`<config>/key`) - generated for every app install
//!   or runtime. A person can have more than one `AppKey` on the same hardware.
//!   Authorized `AppKeys` sign drive roots and `NostrIdentity` roster ops.
//! - [`RecoveryKey`] - a transient recovery-authority key loaded from an
//!   nsec/hex secret or 12-word phrase. It can admit a fresh `AppKey` when that
//!   authority is present in a roster, but it is not the stable `NostrIdentity` id.
//!
//! Create, restore, and link all generate a fresh `AppKey`. Restore from a
//! recovery phrase may keep that user-supplied phrase for later admission of
//! more `AppKeys`; normal create/link flows do not mint recovery material.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use nostr_sdk::{Keys, SecretKey};
use thiserror::Error;

use crate::recovery_phrase::{recovery_phrase_to_keys, secret_input_to_nsec};

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid key file: {0}")]
    InvalidKey(String),
}

macro_rules! keypair_struct {
    ($name:ident) => {
        /// A persisted Nostr keypair. See module docs for which file it lives in.
        #[derive(Debug, Clone)]
        pub struct $name {
            keys: Keys,
            path: PathBuf,
        }

        impl $name {
            pub fn new(keys: Keys, path: impl Into<PathBuf>) -> Self {
                Self {
                    keys,
                    path: path.into(),
                }
            }

            #[must_use]
            pub fn keys(&self) -> &Keys {
                &self.keys
            }

            #[must_use]
            pub fn path(&self) -> &Path {
                &self.path
            }

            #[must_use]
            pub fn pubkey_hex(&self) -> String {
                self.keys.public_key().to_hex()
            }

            #[must_use]
            pub fn pubkey_bech32(&self) -> String {
                self.keys.public_key().to_bech32().unwrap_or_default()
            }

            /// Generate a fresh keypair in memory, without persisting.
            pub fn generate(path: impl Into<PathBuf>) -> Self {
                Self::new(Keys::generate(), path)
            }

            /// Load from disk. Errors if the file is missing or unrecognized.
            pub fn load(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
                let path = path.as_ref();
                let raw = fs::read_to_string(path)?;
                let secret = parse_secret_key(raw.trim())?;
                Ok(Self::new(Keys::new(secret), path.to_path_buf()))
            }

            /// Construct from a bech32 nsec1 (or 64-char hex) string.
            pub fn from_secret(raw: &str, path: impl Into<PathBuf>) -> Result<Self, IdentityError> {
                let secret = parse_secret_key(raw.trim())?;
                Ok(Self::new(Keys::new(secret), path))
            }

            pub fn from_recovery_phrase(
                raw: &str,
                path: impl Into<PathBuf>,
            ) -> Result<Self, IdentityError> {
                let keys = recovery_phrase_to_keys(raw.trim())
                    .map_err(|error| IdentityError::InvalidKey(error.to_string()))?;
                Ok(Self::new(keys, path))
            }

            /// Persist to `self.path()` with owner-only mode on Unix.
            pub fn save(&self) -> Result<(), IdentityError> {
                if let Some(parent) = self.path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let bech = self
                    .keys
                    .secret_key()
                    .to_bech32()
                    .map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
                write_secret_file(&self.path, bech.as_bytes())?;
                Ok(())
            }
        }
    };
}

keypair_struct!(AppKey);
keypair_struct!(RecoveryKey);

impl AppKey {
    /// Load or generate this install's `AppKey`. Creates the
    /// parent dir if missing. Persisted to disk on first generate.
    pub fn load_or_generate(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let path = path.as_ref();
        if path.exists() {
            return Self::load(path);
        }
        let identity = Self::generate(path.to_path_buf());
        identity.save()?;
        Ok(identity)
    }
}

/// Short alias for the current app install's Nostr keypair.
pub type Identity = AppKey;

fn parse_secret_key(raw: &str) -> Result<SecretKey, IdentityError> {
    let nsec =
        secret_input_to_nsec(raw).map_err(|error| IdentityError::InvalidKey(error.to_string()))?;
    SecretKey::from_bech32(&nsec).map_err(|e| IdentityError::InvalidKey(format!("bech32: {e}")))
}

#[cfg(unix)]
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
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
    fn app_key_generate_produces_distinct_keys() {
        let dir = tempdir().unwrap();
        let a = AppKey::generate(dir.path().join("a"));
        let b = AppKey::generate(dir.path().join("b"));
        assert_ne!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn app_key_round_trip_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        let original = AppKey::generate(&path);
        original.save().unwrap();
        let loaded = AppKey::load(&path).unwrap();
        assert_eq!(original.pubkey_hex(), loaded.pubkey_hex());
    }

    #[test]
    fn app_key_load_or_generate_creates_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        assert!(!path.exists());
        let id = AppKey::load_or_generate(&path).unwrap();
        assert!(path.exists());
        let again = AppKey::load_or_generate(&path).unwrap();
        assert_eq!(id.pubkey_hex(), again.pubkey_hex());
    }

    #[test]
    fn recovery_key_round_trips_through_disk_when_explicitly_saved() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recovery");
        let owner = RecoveryKey::generate(&path);
        owner.save().unwrap();
        let loaded = RecoveryKey::load(&path).unwrap();
        assert_eq!(owner.pubkey_hex(), loaded.pubkey_hex());
    }

    #[test]
    fn from_secret_accepts_bech32_nsec() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        let original = RecoveryKey::generate(path_a.clone());
        original.save().unwrap();
        let nsec = original.keys().secret_key().to_bech32().unwrap();
        let recovered = RecoveryKey::from_secret(&nsec, path_b).unwrap();
        assert_eq!(original.pubkey_hex(), recovered.pubkey_hex());
    }

    #[test]
    fn from_secret_accepts_64_char_hex() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("k");
        let hex = "aa".repeat(32);
        let owner = RecoveryKey::from_secret(&hex, &path).unwrap();
        assert_eq!(owner.keys().secret_key().to_secret_hex(), hex);
    }

    #[test]
    fn from_secret_accepts_12_word_recovery_phrase() {
        let dir = tempdir().unwrap();
        let phrase = crate::recovery_phrase::generate_recovery_phrase().unwrap();
        let original =
            RecoveryKey::from_recovery_phrase(&phrase, dir.path().join("original")).unwrap();

        let recovered = RecoveryKey::from_secret(&phrase, dir.path().join("restored")).unwrap();

        assert_eq!(original.pubkey_hex(), recovered.pubkey_hex());
    }

    #[test]
    fn invalid_key_file_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "this is not a key").unwrap();
        assert!(AppKey::load(&path).is_err());
        assert!(RecoveryKey::load(&path).is_err());
    }

    #[test]
    fn pubkey_bech32_starts_with_npub1() {
        let dir = tempdir().unwrap();
        let id = AppKey::generate(dir.path().join("k"));
        assert!(id.pubkey_bech32().starts_with("npub1"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        AppKey::generate(&path).save().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    }
}
