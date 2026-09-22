use std::path::Path;

use anyhow::{Context, Result, bail};
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use nostr_sdk::{Keys, PublicKey, SecretKey};
use sha2::{Digest, Sha256};

use crate::{AppKey, paths::key_path_in};

/// Backup links convey only an address. Adding the address explicitly is still
/// required on both devices; possession never grants storage or reading rights.
pub const BACKUP_INVITE_PREFIX: &str = "iris-drive://backup?npub=";

/// Derive a stable backup-only endpoint from this install's secret. This key is
/// deliberately distinct from both the app's data endpoint and social identity.
pub fn backup_keys(device: &AppKey) -> Result<Keys> {
    let mut hash = Sha256::new();
    hash.update(b"iris-drive/friend-backup/identity/v1\0");
    hash.update(device.keys().secret_key().as_secret_bytes());
    let secret =
        SecretKey::from_slice(&hash.finalize()).context("deriving friend backup identity")?;
    Ok(Keys::new(secret))
}

pub fn backup_npub(config_dir: &Path) -> Result<String> {
    let device = AppKey::load(key_path_in(config_dir))?;
    backup_keys(&device)?
        .public_key()
        .to_bech32()
        .context("encoding user ID")
}

/// Accept an explicitly exchanged backup npub or its convenience link. Only
/// this strict one-field link shape is recognized, with no authority fields.
pub fn parse_backup_contact(input: &str) -> Result<String> {
    let input = input.trim();
    if input.len() > 256 {
        bail!("user ID or backup link is too long");
    }
    let raw = input.strip_prefix(BACKUP_INVITE_PREFIX).unwrap_or(input);
    if !raw.to_ascii_lowercase().starts_with("npub1") {
        bail!("use a friend's user ID or backup link");
    }
    let key = PublicKey::from_bech32(raw).context("invalid user ID")?;
    key.to_bech32().context("encoding user ID")
}

pub fn encode_backup_invite(input: &str) -> Result<String> {
    Ok(format!(
        "{BACKUP_INVITE_PREFIX}{}",
        parse_backup_contact(input)?
    ))
}
