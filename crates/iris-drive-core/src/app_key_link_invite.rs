//! Canonical AppKey-link invite payloads.
//!
//! An admin `AppKey` shares one invite link. Importing it creates a local
//! pending link request addressed to that admin `AppKey`.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};

use crate::IrisProfileId;

pub const APP_KEY_LINK_INVITE_PREFIX: &str = "https://drive.iris.to/invite/";
pub const APP_KEY_LINK_INVITE_WEB_PREFIX: &str = APP_KEY_LINK_INVITE_PREFIX;
pub const APP_KEY_LINK_INVITE_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppKeyLinkInvitePayload {
    v: u8,
    profile_id: IrisProfileId,
    admin_app_key_npub: String,
    invite_npub: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAppKeyLinkInvite {
    pub profile_id: Option<IrisProfileId>,
    pub admin_app_key_hex: String,
    pub invite_pubkey: String,
}

pub fn encode_app_key_link_invite(
    profile_id: IrisProfileId,
    admin_app_key_hex: &str,
    invite_pubkey: &str,
) -> Result<String> {
    let payload = AppKeyLinkInvitePayload {
        v: APP_KEY_LINK_INVITE_VERSION,
        profile_id,
        admin_app_key_npub: pubkey_to_npub(admin_app_key_hex)
            .context("encoding invite admin AppKey")?,
        invite_npub: pubkey_to_npub(invite_pubkey).context("encoding invite pubkey")?,
    };
    let bytes = serde_json::to_vec(&payload).context("encoding app-key link invite JSON")?;
    Ok(format!(
        "{APP_KEY_LINK_INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub fn parse_app_key_link_invite(input: &str) -> Result<Option<ParsedAppKeyLinkInvite>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Some(payload) = canonical_invite_payload(trimmed) {
        if payload.trim().is_empty() {
            return Err(anyhow!("app-key link invite payload is empty"));
        }
        if looks_like_invite_placeholder(payload) {
            return Err(anyhow!(
                "app-key link invite is a placeholder; paste the full https://drive.iris.to/invite/... value"
            ));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .context("decoding app-key link invite payload")?;
        let invite: AppKeyLinkInvitePayload =
            serde_json::from_slice(&decoded).context("parsing app-key link invite payload")?;
        return normalize_invite_payload(&invite).map(Some);
    }
    Ok(None)
}

#[must_use]
pub fn app_key_link_invite_web_url(invite_url: &str) -> String {
    invite_url.replacen(
        APP_KEY_LINK_INVITE_PREFIX,
        APP_KEY_LINK_INVITE_WEB_PREFIX,
        1,
    )
}

fn canonical_invite_payload(input: &str) -> Option<&str> {
    input.strip_prefix(APP_KEY_LINK_INVITE_PREFIX)
}

fn normalize_invite_payload(invite: &AppKeyLinkInvitePayload) -> Result<ParsedAppKeyLinkInvite> {
    if invite.v != APP_KEY_LINK_INVITE_VERSION {
        return Err(anyhow!(
            "unsupported app-key link invite version {}; expected {}",
            invite.v,
            APP_KEY_LINK_INVITE_VERSION
        ));
    }
    Ok(ParsedAppKeyLinkInvite {
        profile_id: Some(invite.profile_id),
        admin_app_key_hex: normalize_pubkey_hex(&invite.admin_app_key_npub)
            .context("parsing invite admin AppKey")?,
        invite_pubkey: normalize_pubkey_hex(&invite.invite_npub)
            .context("parsing invite pubkey")?,
    })
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
    fn canonical_invite_round_trips_profile_admin_and_invite_pubkey() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate().public_key();
        let invite = Keys::generate().public_key();

        let url = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite.to_hex())
            .expect("encode invite");
        let parsed = parse_app_key_link_invite(&url)
            .expect("parse invite")
            .expect("invite");

        assert!(url.starts_with(APP_KEY_LINK_INVITE_PREFIX));
        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.admin_app_key_hex, admin.to_hex());
        assert_eq!(parsed.invite_pubkey, invite.to_hex());
        assert!(!url.contains("owner"));
        assert!(!url.contains("local-owner"));
        assert!(!url.contains("device-"));
    }

    #[test]
    fn custom_scheme_invite_is_not_canonical_input() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate().public_key();
        let invite = Keys::generate().public_key();
        let url = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite.to_hex())
            .expect("encode invite");
        let custom_scheme = url.replacen(APP_KEY_LINK_INVITE_PREFIX, "iris-drive://invite/", 1);

        assert!(
            parse_app_key_link_invite(&custom_scheme)
                .expect("parse invite")
                .is_none()
        );
    }

    #[test]
    fn old_link_secret_https_invite_payload_is_rejected() {
        let invite = "https://drive.iris.to/invite/eyJ2IjoxLCJwcm9maWxlSWQiOiIzYzA4OWRmOC0yMjFlLTQ3M2MtOTFlYy1mNzcxYzAxNWM4YmQiLCJhZG1pbkFwcEtleU5wdWIiOiJucHViMXE1bDB2bmVhamg2Mjg5dmduZ3N3dTV3bTI2cWFtc2p3dHlqajJncWxwa3VqNXptZGVkeXMweTZtdDciLCJsaW5rU2VjcmV0IjoiazV3NUpMR2hUQ09sSWdPQUtpRnUtUSJ9";

        assert!(parse_app_key_link_invite(invite).is_err());
    }

    #[test]
    fn old_owner_query_invite_is_not_canonical_input() {
        let admin = Keys::generate().public_key();
        let admin_npub = admin.to_bech32().expect("admin npub");
        let url = format!("iris-drive://link-device?admin={admin_npub}&secret=s");

        assert!(
            parse_app_key_link_invite(&url)
                .expect("parse invite")
                .is_none()
        );
    }
}
