//! Shared AppKey-link input parsing and validation.
//!
//! Keep route recognition and profile-scoped link-target rules here so CLI and
//! native shells render the same state instead of reimplementing identity
//! admission policy per platform.

use anyhow::{Context, Result, anyhow};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use crate::IrisProfileId;
use crate::app_key_link_invite::{
    APP_KEY_LINK_INVITE_PREFIX, APP_KEY_LINK_INVITE_WEB_PREFIX, parse_app_key_link_invite,
};
use crate::app_key_link_transport::{app_key_approval_query, parse_app_key_approval_request};
use crate::device_summary::pubkey_npub;

const APP_KEY_LINK_INVITE_SINGLE_SLASH_PREFIX: &str = "iris-drive:/invite/";
const MANUAL_LINK_REQUIRES_PROFILE_AND_ADMIN: &str = "manual AppKey linking requires an IrisProfile UUID and --admin-app-key; otherwise paste an admin invite URL";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkInputClassification {
    pub kind: String,
    pub is_complete: bool,
    pub is_valid: bool,
    pub normalized_input: String,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub has_link_secret: bool,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppKeyLinkTarget {
    pub profile_id: IrisProfileId,
    pub admin_app_key_hex: String,
    pub link_secret: String,
}

#[must_use]
pub fn classify_link_input(input: &str) -> LinkInputClassification {
    let trimmed = input.trim();
    let mut classification = LinkInputClassification {
        kind: "empty".to_owned(),
        normalized_input: trimmed.to_owned(),
        ..LinkInputClassification::default()
    };
    if trimmed.is_empty() {
        return classification;
    }
    if trimmed.contains(char::is_whitespace) {
        "unknown".clone_into(&mut classification.kind);
        "link input must not contain whitespace".clone_into(&mut classification.error);
        return classification;
    }

    if let Some(result) = classify_app_key_approval_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_invite_link_input(trimmed) {
        return result;
    }
    if looks_like_app_key_pubkey_input(trimmed) {
        "app_key_pubkey".clone_into(&mut classification.kind);
        classification.is_complete = app_key_pubkey_input_is_complete(trimmed);
        if classification.is_complete {
            match normalize_app_key_pubkey(trimmed) {
                Ok(app_key_hex) => {
                    classification.is_valid = true;
                    classification.admin_app_key_pubkey = pubkey_npub(&app_key_hex);
                    classification
                        .normalized_input
                        .clone_from(&classification.admin_app_key_pubkey);
                }
                Err(error) => {
                    classification.error = error.to_string();
                }
            }
        }
        return classification;
    }

    "unknown".clone_into(&mut classification.kind);
    "expected AppKey pubkey or IrisProfile invite link".clone_into(&mut classification.error);
    classification
}

pub fn resolve_app_key_link_target(
    input: &str,
    manual_admin_app_key: Option<&str>,
) -> Result<AppKeyLinkTarget> {
    if let Some(invite) = parse_app_key_link_invite(input)? {
        if manual_admin_app_key.is_some() {
            return Err(anyhow!(
                "--admin-app-key is only valid with a manual IrisProfile UUID, not an invite URL"
            ));
        }
        let profile_id = invite
            .profile_id
            .ok_or_else(|| anyhow!("AppKey invite is missing IrisProfile id"))?;
        return Ok(AppKeyLinkTarget {
            profile_id,
            admin_app_key_hex: invite.admin_app_key_hex,
            link_secret: invite.link_secret,
        });
    }

    let Some(manual_admin_app_key) = manual_admin_app_key else {
        return Err(anyhow!(MANUAL_LINK_REQUIRES_PROFILE_AND_ADMIN));
    };
    let trimmed = input.trim();
    if normalize_app_key_pubkey(trimmed).is_ok() {
        return Err(anyhow!(MANUAL_LINK_REQUIRES_PROFILE_AND_ADMIN));
    }
    Ok(AppKeyLinkTarget {
        profile_id: trimmed
            .parse::<IrisProfileId>()
            .context("parsing IrisProfile UUID")?,
        admin_app_key_hex: normalize_app_key_pubkey(manual_admin_app_key)
            .context("parsing admin AppKey pubkey")?,
        link_secret: String::new(),
    })
}

pub fn normalize_app_key_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("public key is required"));
    }
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

fn classify_app_key_approval_link_input(input: &str) -> Option<LinkInputClassification> {
    let query = app_key_approval_query(input)?;
    let profile =
        raw_query_value(query, "profile").or_else(|| raw_query_value(query, "profile_id"));
    let app_key = raw_query_value(query, "app_key").or_else(|| raw_query_value(query, "appKey"));
    let is_complete = profile
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && app_key
            .as_deref()
            .is_some_and(app_key_pubkey_input_is_complete);
    let mut classification = LinkInputClassification {
        kind: "app_key_approval".to_owned(),
        normalized_input: input.to_owned(),
        is_complete,
        ..LinkInputClassification::default()
    };
    if is_complete {
        match parse_app_key_approval_request(input) {
            Ok(Some(request)) => {
                classification.is_valid = true;
                classification.app_key_pubkey = pubkey_npub(&request.app_key_hex);
            }
            Ok(None) => {
                "AppKey-link request was not recognized".clone_into(&mut classification.error);
            }
            Err(error) => classification.error = error.to_string(),
        }
    }
    classification.has_link_secret = parse_app_key_approval_request(input)
        .ok()
        .flatten()
        .is_some_and(|request| !request.link_secret.trim().is_empty());
    Some(classification)
}

fn classify_invite_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let is_canonical = [
        APP_KEY_LINK_INVITE_PREFIX,
        APP_KEY_LINK_INVITE_SINGLE_SLASH_PREFIX,
        APP_KEY_LINK_INVITE_WEB_PREFIX,
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || link_route_matches(&lower, "iris-drive://invite", true)
        || link_route_matches(&lower, "iris-drive:/invite", true)
        || link_route_matches(&lower, "https://drive.iris.to/invite", true);
    let is_json = input.starts_with('{');
    if !(is_canonical || is_json) {
        return None;
    }

    let mut classification = LinkInputClassification {
        kind: "invite".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: invite_link_input_is_complete(input),
        ..LinkInputClassification::default()
    };
    match parse_app_key_link_invite(input) {
        Ok(Some(invite)) => {
            classification.is_complete = true;
            classification.is_valid = true;
            classification.admin_app_key_pubkey = pubkey_npub(&invite.admin_app_key_hex);
            classification.has_link_secret = !invite.link_secret.trim().is_empty();
        }
        Ok(None) => "AppKey invite was not recognized".clone_into(&mut classification.error),
        Err(error) if classification.is_complete => {
            classification.error = error.to_string();
        }
        Err(_) => {}
    }
    Some(classification)
}

fn link_route_matches(input: &str, route: &str, allow_path_suffix: bool) -> bool {
    let Some(rest) = input.strip_prefix(route) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('?') || (allow_path_suffix && rest.starts_with('/'))
}

fn invite_link_input_is_complete(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    for prefix in [
        APP_KEY_LINK_INVITE_PREFIX,
        APP_KEY_LINK_INVITE_SINGLE_SLASH_PREFIX,
        APP_KEY_LINK_INVITE_WEB_PREFIX,
    ] {
        if lower.starts_with(prefix) {
            return input[prefix.len()..].len() >= 32;
        }
    }
    input.starts_with('{') && input.ends_with('}')
}

fn looks_like_app_key_pubkey_input(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.starts_with("npub1")
        || (input.len() <= 64 && input.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn app_key_pubkey_input_is_complete(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("npub1") {
        return input.len() >= 63;
    }
    input.len() == 64 && input.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn raw_query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name && !value.is_empty()).then(|| value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_key_link_invite::encode_app_key_link_invite;
    use crate::app_key_link_transport::encode_app_key_approval_request;
    use nostr_sdk::Keys;
    use nostr_sdk::nips::nip19::ToBech32;

    #[test]
    fn classify_link_input_is_shared_for_invites_app_keys_and_approval_links() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate().public_key();
        let invite =
            encode_app_key_link_invite(profile_id, &admin.to_hex(), "secret").expect("invite");

        let invite_classification = classify_link_input(&invite);
        assert_eq!(invite_classification.kind, "invite");
        assert!(invite_classification.is_complete);
        assert!(invite_classification.is_valid);
        assert_eq!(
            invite_classification.admin_app_key_pubkey,
            admin.to_bech32().expect("npub")
        );
        assert!(invite_classification.has_link_secret);

        let app_key_npub = admin.to_bech32().expect("npub");
        let app_key = classify_link_input(&app_key_npub);
        assert_eq!(app_key.kind, "app_key_pubkey");
        assert!(app_key.is_complete);
        assert!(app_key.is_valid);
        assert_eq!(app_key.normalized_input, app_key_npub);
        assert_eq!(app_key.admin_app_key_pubkey, app_key_npub);

        let short_app_key = classify_link_input("npub1short");
        assert_eq!(short_app_key.kind, "app_key_pubkey");
        assert!(!short_app_key.is_complete);
        assert!(!short_app_key.is_valid);

        let request = encode_app_key_approval_request(profile_id, &admin.to_hex(), "secret", None);
        let approval = classify_link_input(&request);
        assert_eq!(approval.kind, "app_key_approval");
        assert!(approval.is_complete);
        assert!(approval.is_valid);
        assert_eq!(approval.app_key_pubkey, app_key_npub);
        assert!(approval.has_link_secret);
    }

    #[test]
    fn classify_invite_routes_distinguishes_partial_and_nearby_links() {
        let short = classify_link_input("https://drive.iris.to/invite/demo");
        assert_eq!(short.kind, "invite");
        assert!(!short.is_complete);
        assert!(!short.is_valid);

        let unrelated = classify_link_input("https://drive.iris.to/app-key-linker?owner=npub1x");
        assert_eq!(unrelated.kind, "unknown");
    }

    #[test]
    fn resolve_app_key_link_target_accepts_invite_or_manual_profile_with_admin() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate().public_key();
        let invite =
            encode_app_key_link_invite(profile_id, &admin.to_hex(), "secret").expect("invite");

        let from_invite = resolve_app_key_link_target(&invite, None).expect("invite target");
        assert_eq!(from_invite.profile_id, profile_id);
        assert_eq!(from_invite.admin_app_key_hex, admin.to_hex());
        assert_eq!(from_invite.link_secret, "secret");

        let from_manual =
            resolve_app_key_link_target(&profile_id.to_string(), Some(&admin.to_hex()))
                .expect("manual target");
        assert_eq!(from_manual.profile_id, profile_id);
        assert_eq!(from_manual.admin_app_key_hex, admin.to_hex());
        assert!(from_manual.link_secret.is_empty());
    }

    #[test]
    fn resolve_app_key_link_target_rejects_bare_app_key_as_identity() {
        let admin = Keys::generate().public_key();
        let error =
            resolve_app_key_link_target(&admin.to_bech32().expect("npub"), Some(&admin.to_hex()))
                .expect_err("bare app key is not an identity target");

        assert!(error.to_string().contains("IrisProfile UUID"));
    }
}
