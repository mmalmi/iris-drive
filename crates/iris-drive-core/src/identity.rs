//! Per-device and owner Nostr keypairs.
//!
//! Iris Drive has two key kinds, persisted to separate files:
//!
//! - [`DeviceIdentity`] (`<config>/key`) — generated on every install,
//!   present on every device. Identifies this machine. Used to sign
//!   per-device drive-tree roots.
//! - [`OwnerKey`] (`<config>/owner_key`) — the user's long-lived
//!   identity key. Present only on devices that have owner-signing
//!   authority (i.e. the user clicked "Create" or "Restore" rather
//!   than "Link this device"). Used to sign the `AppKeys` roster.
//!
//! Single-device mode is the v1 default: `idrive init` generates both
//! keys, the same install holds both files, and the `AppKeys` roster
//! lists this one device. Linked devices skip the owner key.

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

keypair_struct!(DeviceIdentity);
keypair_struct!(OwnerKey);

impl DeviceIdentity {
    /// Load or generate the device's per-machine identity. Creates the
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

impl OwnerKey {
    /// Owner key creation is deliberately explicit — there's no
    /// `load_or_generate`. Use `OwnerKey::generate(path).save()?` for the
    /// "Create" flow or `OwnerKey::from_secret(nsec, path)?.save()?` for
    /// "Restore". Linked devices never call either.
    pub fn exists_at(path: impl AsRef<Path>) -> bool {
        path.as_ref().exists()
    }
}

/// Back-compat alias for the historical name. New code should use
/// `DeviceIdentity` directly.
pub type Identity = DeviceIdentity;

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
    fn device_generate_produces_distinct_keys() {
        let dir = tempdir().unwrap();
        let a = DeviceIdentity::generate(dir.path().join("a"));
        let b = DeviceIdentity::generate(dir.path().join("b"));
        assert_ne!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn device_round_trip_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        let original = DeviceIdentity::generate(&path);
        original.save().unwrap();
        let loaded = DeviceIdentity::load(&path).unwrap();
        assert_eq!(original.pubkey_hex(), loaded.pubkey_hex());
    }

    #[test]
    fn device_load_or_generate_creates_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        assert!(!path.exists());
        let id = DeviceIdentity::load_or_generate(&path).unwrap();
        assert!(path.exists());
        let again = DeviceIdentity::load_or_generate(&path).unwrap();
        assert_eq!(id.pubkey_hex(), again.pubkey_hex());
    }

    #[test]
    fn owner_does_not_auto_generate() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("owner_key");
        let owner = OwnerKey::generate(&path);
        owner.save().unwrap();
        let loaded = OwnerKey::load(&path).unwrap();
        assert_eq!(owner.pubkey_hex(), loaded.pubkey_hex());
    }

    #[test]
    fn owner_exists_at_reports_correctly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("owner_key");
        assert!(!OwnerKey::exists_at(&path));
        OwnerKey::generate(&path).save().unwrap();
        assert!(OwnerKey::exists_at(&path));
    }

    #[test]
    fn from_secret_accepts_bech32_nsec() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        let original = OwnerKey::generate(path_a.clone());
        original.save().unwrap();
        let nsec = original.keys().secret_key().to_bech32().unwrap();
        let recovered = OwnerKey::from_secret(&nsec, path_b).unwrap();
        assert_eq!(original.pubkey_hex(), recovered.pubkey_hex());
    }

    #[test]
    fn from_secret_accepts_64_char_hex() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("k");
        let hex = "aa".repeat(32);
        let owner = OwnerKey::from_secret(&hex, &path).unwrap();
        assert_eq!(owner.keys().secret_key().to_secret_hex(), hex);
    }

    #[test]
    fn from_secret_accepts_12_word_recovery_phrase() {
        let dir = tempdir().unwrap();
        let phrase = crate::recovery_phrase::generate_recovery_phrase().unwrap();
        let original =
            OwnerKey::from_recovery_phrase(&phrase, dir.path().join("original")).unwrap();

        let recovered = OwnerKey::from_secret(&phrase, dir.path().join("restored")).unwrap();

        assert_eq!(original.pubkey_hex(), recovered.pubkey_hex());
    }

    #[test]
    fn invalid_key_file_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "this is not a key").unwrap();
        assert!(DeviceIdentity::load(&path).is_err());
        assert!(OwnerKey::load(&path).is_err());
    }

    #[test]
    fn pubkey_bech32_starts_with_npub1() {
        let dir = tempdir().unwrap();
        let id = DeviceIdentity::generate(dir.path().join("k"));
        assert!(id.pubkey_bech32().starts_with("npub1"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        DeviceIdentity::generate(&path).save().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    }
}
