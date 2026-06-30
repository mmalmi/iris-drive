//! Canonical AppKey-link invite payloads.
//!
//! An admin `AppKey` shares one invite link. Importing it creates a local
//! pending link request addressed to that admin `AppKey`.

use anyhow::{Context, Result};
use nostr_identity::{
    NostrIdentityDeviceLinkInvite, encode_nostr_identity_device_link_invite,
    parse_nostr_identity_device_link_invite,
};

use crate::NostrIdentityId;

pub const APP_KEY_LINK_INVITE_PREFIX: &str = "https://drive.iris.to/invite/";
pub const APP_KEY_LINK_INVITE_WEB_PREFIX: &str = APP_KEY_LINK_INVITE_PREFIX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAppKeyLinkInvite {
    pub profile_id: Option<NostrIdentityId>,
    pub admin_app_key_hex: String,
    pub invite_pubkey: String,
}

pub fn encode_app_key_link_invite(
    profile_id: NostrIdentityId,
    admin_app_key_hex: &str,
    invite_pubkey: &str,
) -> Result<String> {
    encode_nostr_identity_device_link_invite(
        &NostrIdentityDeviceLinkInvite {
            profile_id,
            admin_app_key_pubkey: admin_app_key_hex.to_owned(),
            invite_pubkey: invite_pubkey.to_owned(),
        },
        Some(APP_KEY_LINK_INVITE_PREFIX),
    )
    .context("encoding app-key link invite")
}

pub fn parse_app_key_link_invite(input: &str) -> Result<Option<ParsedAppKeyLinkInvite>> {
    let Some(invite) =
        parse_nostr_identity_device_link_invite(input.trim(), &[APP_KEY_LINK_INVITE_PREFIX])
            .context("parsing app-key link invite")?
    else {
        return Ok(None);
    };
    Ok(Some(ParsedAppKeyLinkInvite {
        profile_id: Some(invite.profile_id),
        admin_app_key_hex: invite.admin_app_key_pubkey,
        invite_pubkey: invite.invite_pubkey,
    }))
}

#[must_use]
pub fn app_key_link_invite_web_url(invite_url: &str) -> String {
    invite_url.replacen(
        APP_KEY_LINK_INVITE_PREFIX,
        APP_KEY_LINK_INVITE_WEB_PREFIX,
        1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_identity::NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX;
    use nostr_sdk::Keys;
    use nostr_sdk::nips::nip19::ToBech32;

    #[test]
    fn canonical_invite_round_trips_profile_admin_and_invite_pubkey() {
        let profile_id = NostrIdentityId::new_v4();
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
    fn shared_scheme_invite_round_trips_too() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate().public_key();
        let invite = Keys::generate().public_key();
        let url = nostr_identity::encode_nostr_identity_device_link_invite(
            &NostrIdentityDeviceLinkInvite {
                profile_id,
                admin_app_key_pubkey: admin.to_hex(),
                invite_pubkey: invite.to_hex(),
            },
            None,
        )
        .expect("encode invite");

        let parsed = parse_app_key_link_invite(&url)
            .expect("parse invite")
            .expect("invite");

        assert!(url.starts_with(NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX));
        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.admin_app_key_hex, admin.to_hex());
        assert_eq!(parsed.invite_pubkey, invite.to_hex());
    }

    #[test]
    fn custom_scheme_invite_is_not_canonical_input() {
        let profile_id = NostrIdentityId::new_v4();
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
