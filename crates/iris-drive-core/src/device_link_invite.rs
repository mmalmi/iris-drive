//! Canonical device-link invite payloads.
//!
//! The owner/admin device shares one invite link. Importing it creates a local
//! pending link request that is sent to the admin over FIPS when available.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};

pub const DEVICE_LINK_INVITE_PREFIX: &str = "iris-drive://invite/";
pub const DEVICE_LINK_INVITE_WEB_PREFIX: &str = "https://drive.iris.to/invite/";
pub const DEVICE_LINK_INVITE_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceLinkInvitePayload {
    v: u8,
    #[serde(alias = "owner")]
    owner_npub: String,
    #[serde(alias = "admin")]
    admin_device_npub: String,
    #[serde(alias = "secret", alias = "link_secret")]
    link_secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDeviceLinkInvite {
    pub owner_hex: String,
    pub admin_device_hex: String,
    pub link_secret: String,
}

pub fn encode_device_link_invite(
    owner_hex: &str,
    admin_device_hex: &str,
    link_secret: &str,
) -> Result<String> {
    let payload = DeviceLinkInvitePayload {
        v: DEVICE_LINK_INVITE_VERSION,
        owner_npub: pubkey_to_npub(owner_hex).context("encoding invite owner")?,
        admin_device_npub: pubkey_to_npub(admin_device_hex)
            .context("encoding invite admin device")?,
        link_secret: link_secret.trim().to_string(),
    };
    if payload.link_secret.is_empty() {
        return Err(anyhow!("device link invite is missing secret"));
    }
    let bytes = serde_json::to_vec(&payload).context("encoding device link invite JSON")?;
    Ok(format!(
        "{DEVICE_LINK_INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub fn parse_device_link_invite(input: &str) -> Result<Option<ParsedDeviceLinkInvite>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Some(payload) = canonical_invite_payload(trimmed) {
        if payload.trim().is_empty() {
            return Err(anyhow!("device link invite payload is empty"));
        }
        if looks_like_invite_placeholder(payload) {
            return Err(anyhow!(
                "device link invite is a placeholder; paste the full iris-drive://invite/... value"
            ));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .context("decoding device link invite payload")?;
        let invite: DeviceLinkInvitePayload =
            serde_json::from_slice(&decoded).context("parsing device link invite payload")?;
        return normalize_invite_payload(invite).map(Some);
    }
    if trimmed.starts_with('{') {
        let invite: DeviceLinkInvitePayload =
            serde_json::from_str(trimmed).context("parsing device link invite JSON")?;
        return normalize_invite_payload(invite).map(Some);
    }
    if let Some(query) = legacy_invite_query(trimmed) {
        return parse_legacy_query_invite(query).map(Some);
    }
    Ok(None)
}

pub fn device_link_invite_web_url(invite_url: &str) -> String {
    invite_url.replacen(DEVICE_LINK_INVITE_PREFIX, DEVICE_LINK_INVITE_WEB_PREFIX, 1)
}

fn canonical_invite_payload(input: &str) -> Option<&str> {
    input
        .strip_prefix(DEVICE_LINK_INVITE_PREFIX)
        .or_else(|| input.strip_prefix(DEVICE_LINK_INVITE_WEB_PREFIX))
}

fn normalize_invite_payload(invite: DeviceLinkInvitePayload) -> Result<ParsedDeviceLinkInvite> {
    if invite.v != DEVICE_LINK_INVITE_VERSION {
        return Err(anyhow!(
            "unsupported device link invite version {}; expected {}",
            invite.v,
            DEVICE_LINK_INVITE_VERSION
        ));
    }
    let link_secret = invite.link_secret.trim().to_string();
    if link_secret.is_empty() {
        return Err(anyhow!("device link invite is missing secret"));
    }
    Ok(ParsedDeviceLinkInvite {
        owner_hex: normalize_pubkey_hex(&invite.owner_npub).context("parsing invite owner")?,
        admin_device_hex: normalize_pubkey_hex(&invite.admin_device_npub)
            .context("parsing invite admin device")?,
        link_secret,
    })
}

fn parse_legacy_query_invite(query: &str) -> Result<ParsedDeviceLinkInvite> {
    let mut owner = None;
    let mut admin = None;
    let mut link_secret = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "owner" if !value.trim().is_empty() => owner = Some(value),
            "admin" | "admin_device" if !value.trim().is_empty() => admin = Some(value),
            "secret" | "link_secret" if !value.trim().is_empty() => link_secret = Some(value),
            _ => {}
        }
    }

    let owner = owner.ok_or_else(|| anyhow!("device link invite is missing owner"))?;
    let admin = admin.ok_or_else(|| anyhow!("device link invite is missing admin"))?;
    let link_secret = link_secret.ok_or_else(|| anyhow!("device link invite is missing secret"))?;
    Ok(ParsedDeviceLinkInvite {
        owner_hex: normalize_pubkey_hex(&owner).context("parsing invite owner")?,
        admin_device_hex: normalize_pubkey_hex(&admin).context("parsing invite admin device")?,
        link_secret: link_secret.trim().to_string(),
    })
}

fn legacy_invite_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix("iris-drive://link-device") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("https://drive.iris.to/link-device") {
        return rest.strip_prefix('?');
    }
    None
}

fn normalize_pubkey_hex(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pubkey = PublicKey::from_bech32(trimmed).context("parsing npub")?;
        return Ok(pubkey.to_hex());
    }
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    Err(anyhow!(
        "expected npub1... or 64-char hex pubkey, got {trimmed}"
    ))
}

fn pubkey_to_npub(pubkey_hex: &str) -> Result<String> {
    let pubkey = PublicKey::parse(pubkey_hex).context("parsing hex pubkey")?;
    pubkey.to_bech32().context("encoding npub")
}

fn percent_decode_component(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .context("invalid percent escape")?;
                let byte = u8::from_str_radix(hex, 16).context("invalid percent escape")?;
                out.push(byte);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).context("invalid utf-8 in percent escape")
}

fn looks_like_invite_placeholder(payload: &str) -> bool {
    let trimmed = payload.trim();
    trimmed.contains("...")
        || trimmed.contains('…')
        || matches!(trimmed, "<code>" | "<payload>" | "<invite>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::Keys;

    #[test]
    fn canonical_invite_round_trips_owner_admin_and_secret() {
        let owner = Keys::generate().public_key();
        let admin = Keys::generate().public_key();

        let url = encode_device_link_invite(&owner.to_hex(), &admin.to_hex(), " join-secret ")
            .expect("encode invite");
        let parsed = parse_device_link_invite(&url)
            .expect("parse invite")
            .expect("invite");

        assert!(url.starts_with(DEVICE_LINK_INVITE_PREFIX));
        assert_eq!(parsed.owner_hex, owner.to_hex());
        assert_eq!(parsed.admin_device_hex, admin.to_hex());
        assert_eq!(parsed.link_secret, "join-secret");
        assert!(!url.contains("local-owner"));
        assert!(!url.contains("device-"));
    }

    #[test]
    fn legacy_query_invite_still_imports() {
        let owner = Keys::generate().public_key();
        let admin = Keys::generate().public_key();
        let owner_npub = owner.to_bech32().expect("owner npub");
        let admin_npub = admin.to_bech32().expect("admin npub");
        let url =
            format!("iris-drive://link-device?owner={owner_npub}&admin={admin_npub}&secret=s");

        let parsed = parse_device_link_invite(&url)
            .expect("parse invite")
            .expect("invite");

        assert_eq!(parsed.owner_hex, owner.to_hex());
        assert_eq!(parsed.admin_device_hex, admin.to_hex());
        assert_eq!(parsed.link_secret, "s");
    }
}
