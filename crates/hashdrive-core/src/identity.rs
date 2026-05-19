//! Nostr-key identity management.
//!
//! Each hashdrive install has one Nostr keypair stored in `key` under
//! the config dir. Generation is deferred until first use so the file is
//! never created accidentally.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use nostr_sdk::{Keys, SecretKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid key file: {0}")]
    InvalidKey(String),
}

/// A loaded identity. Wraps `nostr_sdk::Keys` and adds the path the key
/// came from / would be persisted to.
#[derive(Debug, Clone)]
pub struct Identity {
    keys: Keys,
    key_path: PathBuf,
}

impl Identity {
    pub fn new(keys: Keys, key_path: impl Into<PathBuf>) -> Self {
        Self {
            keys,
            key_path: key_path.into(),
        }
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// Hex-encoded public key (32 bytes).
    pub fn pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    /// Bech32-encoded npub for human-facing display.
    pub fn pubkey_bech32(&self) -> String {
        self.keys.public_key().to_bech32().unwrap_or_default()
    }

    /// Generate a fresh keypair without persisting anything.
    pub fn generate(key_path: impl Into<PathBuf>) -> Self {
        Self::new(Keys::generate(), key_path)
    }

    /// Load an identity from disk. Errors if the file does not exist or
    /// contains an unrecognized key format.
    pub fn load(key_path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let key_path = key_path.as_ref();
        let raw = fs::read_to_string(key_path)?;
        let trimmed = raw.trim();
        let secret = parse_secret_key(trimmed)?;
        Ok(Self::new(Keys::new(secret), key_path.to_path_buf()))
    }

    /// Load an identity if one exists at `key_path`; otherwise generate
    /// a fresh one and persist it. The parent dir is created if missing.
    pub fn load_or_generate(key_path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let key_path = key_path.as_ref();
        if key_path.exists() {
            return Self::load(key_path);
        }
        let identity = Self::generate(key_path.to_path_buf());
        identity.save()?;
        Ok(identity)
    }

    /// Persist the secret key in bech32 form to `key_path`. Restricts file
    /// mode to owner-only on Unix.
    pub fn save(&self) -> Result<(), IdentityError> {
        if let Some(parent) = self.key_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bech = self
            .keys
            .secret_key()
            .to_bech32()
            .map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
        write_secret_file(&self.key_path, bech.as_bytes())?;
        Ok(())
    }
}

fn parse_secret_key(raw: &str) -> Result<SecretKey, IdentityError> {
    if raw.starts_with("nsec1") {
        return SecretKey::from_bech32(raw)
            .map_err(|e| IdentityError::InvalidKey(format!("bech32: {e}")));
    }
    // Accept raw 64-char hex too, for cases where someone hand-edits.
    if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return SecretKey::from_hex(raw)
            .map_err(|e| IdentityError::InvalidKey(format!("hex: {e}")));
    }
    Err(IdentityError::InvalidKey(
        "expected nsec1... or 64-char hex".into(),
    ))
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
    fn generate_produces_distinct_keys() {
        let dir = tempdir().unwrap();
        let a = Identity::generate(dir.path().join("a"));
        let b = Identity::generate(dir.path().join("b"));
        assert_ne!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn round_trip_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        let original = Identity::generate(&path);
        original.save().unwrap();
        let loaded = Identity::load(&path).unwrap();
        assert_eq!(original.pubkey_hex(), loaded.pubkey_hex());
    }

    #[test]
    fn load_or_generate_creates_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        assert!(!path.exists());
        let id = Identity::load_or_generate(&path).unwrap();
        assert!(path.exists());
        // calling again returns the same identity
        let again = Identity::load_or_generate(&path).unwrap();
        assert_eq!(id.pubkey_hex(), again.pubkey_hex());
    }

    #[test]
    fn load_or_generate_respects_existing_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        let id_a = Identity::generate(&path);
        id_a.save().unwrap();
        let id_b = Identity::load_or_generate(&path).unwrap();
        assert_eq!(id_a.pubkey_hex(), id_b.pubkey_hex());
    }

    #[test]
    fn invalid_key_file_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "this is not a key").unwrap();
        assert!(Identity::load(&path).is_err());
    }

    #[test]
    fn pubkey_bech32_starts_with_npub1() {
        let dir = tempdir().unwrap();
        let id = Identity::generate(dir.path().join("k"));
        assert!(id.pubkey_bech32().starts_with("npub1"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("key");
        Identity::generate(&path).save().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    }
}
