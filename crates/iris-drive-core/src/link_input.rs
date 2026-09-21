//! Shared AppKey-link input parsing and validation.
//!
//! Keep route recognition and profile-scoped link-target rules here so CLI and
//! native shells render the same state instead of reimplementing identity
//! admission policy per platform.

use anyhow::{Context, Result, anyhow};
use hashtree_core::nhash_decode;
use nostr_identity::NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use crate::NostrIdentityId;
use crate::app_key_link_invite::{APP_KEY_LINK_INVITE_PREFIX, parse_app_key_link_invite};
use crate::app_key_link_transport::{
    app_key_approval_input_has_prefix, parse_app_key_approval_bootstrap,
};
use crate::app_key_summary::pubkey_npub;
use crate::gateway::{
    DEFAULT_GATEWAY_PORT, IRIS_SITES_PORTAL_NPUB, is_dns_site_label, local_mutable_site_url,
    local_nhash_url, local_portal_npub_path_url,
};

const MANUAL_LINK_REQUIRES_PROFILE_AND_ADMIN: &str = "manual device linking requires an NostrIdentity UUID and --admin-app-key; otherwise paste an admin invite URL";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkInputClassification {
    pub kind: String,
    pub is_complete: bool,
    pub is_valid: bool,
    pub normalized_input: String,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub has_invite_pubkey: bool,
    pub share_source_path: String,
    pub share_display_name: String,
    pub share_recipient_npub_hint: String,
    pub share_recipient_display_name: String,
    pub share_recipient_profile_id: String,
    pub content_nhash: String,
    pub content_path_hint: String,
    pub open_display_name: String,
    pub local_open_url: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppKeyLinkTarget {
    pub profile_id: NostrIdentityId,
    pub admin_app_key_hex: String,
    pub invite_pubkey: String,
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

    if trimmed.starts_with("iris-drive://backup") {
        "backup_invite".clone_into(&mut classification.kind);
        match crate::friend_backup::encode_backup_invite(trimmed) {
            Ok(invite) => {
                classification.normalized_input = invite;
                classification.is_complete = true;
                classification.is_valid = true;
            }
            Err(error) => classification.error = error.to_string(),
        }
        return classification;
    }

    if let Some(result) = classify_app_key_approval_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_invite_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_share_dialog_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_drive_nhash_file_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_drive_mutable_file_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_iris_web_link_input(trimmed) {
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
    "expected device key or NostrIdentity invite link".clone_into(&mut classification.error);
    classification
}

pub fn resolve_app_key_link_target(
    input: &str,
    manual_admin_app_key: Option<&str>,
) -> Result<AppKeyLinkTarget> {
    if let Some(invite) = parse_app_key_link_invite(input)? {
        if manual_admin_app_key.is_some() {
            return Err(anyhow!(
                "--admin-app-key is only valid with a manual NostrIdentity UUID, not an invite URL"
            ));
        }
        let profile_id = invite
            .profile_id
            .ok_or_else(|| anyhow!("device invite is missing NostrIdentity id"))?;
        return Ok(AppKeyLinkTarget {
            profile_id,
            admin_app_key_hex: invite.admin_app_key_hex,
            invite_pubkey: invite.invite_pubkey,
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
            .parse::<NostrIdentityId>()
            .context("parsing NostrIdentity UUID")?,
        admin_app_key_hex: normalize_app_key_pubkey(manual_admin_app_key)
            .context("parsing admin device key")?,
        invite_pubkey: String::new(),
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
    if !app_key_approval_input_has_prefix(input) {
        return None;
    }
    let mut classification = LinkInputClassification {
        kind: "app_key_approval".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: true,
        ..LinkInputClassification::default()
    };
    match parse_app_key_approval_bootstrap(input) {
        Ok(Some(bootstrap)) => {
            classification.is_valid = true;
            classification.app_key_pubkey = bootstrap.device_app_key_npub;
        }
        Ok(None) => {
            "device request was not recognized".clone_into(&mut classification.error);
        }
        Err(error) => classification.error = error.to_string(),
    }
    Some(classification)
}

fn classify_invite_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let lower = lower.strip_prefix("nostr:").unwrap_or(&lower);
    let payload = [
        APP_KEY_LINK_INVITE_PREFIX,
        NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX,
    ]
    .into_iter()
    .find_map(|prefix| lower.strip_prefix(prefix));
    if payload.is_none() && !link_route_matches(lower, "https://drive.iris.to/invite", true) {
        return None;
    }

    let mut classification = LinkInputClassification {
        kind: "invite".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: payload.is_some_and(|payload| {
            payload.split(['?', '#']).next().unwrap_or_default().len() >= 32
        }),
        ..LinkInputClassification::default()
    };
    match parse_app_key_link_invite(input) {
        Ok(Some(invite)) => {
            classification.is_complete = true;
            classification.is_valid = true;
            classification.admin_app_key_pubkey = pubkey_npub(&invite.admin_app_key_hex);
            classification.has_invite_pubkey = !invite.invite_pubkey.trim().is_empty();
        }
        Ok(None) => "device invite was not recognized".clone_into(&mut classification.error),
        Err(error) if classification.is_complete => {
            classification.error = error.to_string();
        }
        Err(_) => {}
    }
    Some(classification)
}

fn classify_share_dialog_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let is_share_dialog = link_route_matches(&lower, "iris-drive://share", false)
        || link_route_matches(&lower, "iris-drive:/share", false)
        || link_route_matches(&lower, "https://drive.iris.to/share", false);
    if !is_share_dialog {
        return None;
    }

    let query = input.split_once('?').map_or("", |(_, query)| query);
    let path = match decoded_first_query_value_or_default(query, &["path"]) {
        Ok(path) => path,
        Err(error) => {
            return Some(share_dialog_error(input, &error));
        }
    };
    let display_name = match decoded_first_query_value_or_default(query, &["name", "display_name"])
    {
        Ok(display_name) => display_name,
        Err(error) => {
            return Some(share_dialog_error(input, &error));
        }
    };
    let (recipient_npub_hint, recipient_display_name, recipient_profile_id) =
        match share_dialog_recipient_hints(query) {
            Ok(hints) => hints,
            Err(error) => {
                return Some(share_dialog_error(input, &error));
            }
        };

    let is_complete = !path.is_empty();
    let error = if is_complete {
        String::new()
    } else {
        "share source path is required".to_owned()
    };
    Some(LinkInputClassification {
        kind: "share_dialog".to_owned(),
        normalized_input: input.to_owned(),
        is_complete,
        is_valid: is_complete,
        share_source_path: path,
        share_display_name: display_name,
        share_recipient_npub_hint: recipient_npub_hint,
        share_recipient_display_name: recipient_display_name,
        share_recipient_profile_id: recipient_profile_id,
        error,
        ..LinkInputClassification::default()
    })
}

fn share_dialog_recipient_hints(query: &str) -> Result<(String, String, String)> {
    Ok((
        decoded_first_query_value_or_default(query, &["recipient_npub"])?,
        decoded_first_query_value_or_default(query, &["recipient_name", "recipient_display_name"])?,
        decoded_first_query_value_or_default(
            query,
            &["recipient_profile", "recipient_profile_id"],
        )?,
    ))
}

fn share_dialog_error(input: &str, error: &anyhow::Error) -> LinkInputClassification {
    LinkInputClassification {
        kind: "share_dialog".to_owned(),
        normalized_input: input.to_owned(),
        error: error.to_string(),
        ..LinkInputClassification::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DriveNhashFileLink {
    nhash: String,
    path_hint: String,
    display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DriveMutableFileLink {
    npub: String,
    tree_name: String,
    path_segments: Vec<String>,
    path_hint: String,
    display_name: String,
}

fn classify_drive_nhash_file_link_input(input: &str) -> Option<LinkInputClassification> {
    let route = drive_iris_to_fragment_or_path_route(input)?;
    if !drive_route_could_be_nhash_file(route) {
        return None;
    }

    let mut classification = LinkInputClassification {
        kind: "nhash_file".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: true,
        ..LinkInputClassification::default()
    };

    match parse_drive_nhash_file_route(route) {
        Ok(link) => {
            classification.is_valid = true;
            classification.content_nhash = link.nhash;
            classification.content_path_hint = link.path_hint;
            classification.open_display_name = link.display_name;
            classification.local_open_url = local_nhash_url(
                DEFAULT_GATEWAY_PORT,
                &classification.content_nhash,
                (!classification.content_path_hint.is_empty())
                    .then_some(classification.content_path_hint.as_str()),
            );
        }
        Err(error) => {
            classification.error = error.to_string();
            if classification.error.contains("missing nhash") {
                classification.is_complete = false;
            }
        }
    }

    Some(classification)
}

fn drive_iris_to_fragment_or_path_route(input: &str) -> Option<&str> {
    let lower = input.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://drive.iris.to/")?;
    let after_origin = &input[input.len() - rest.len()..];
    Some(after_origin.strip_prefix("#/").unwrap_or(after_origin))
}

fn drive_route_could_be_nhash_file(route: &str) -> bool {
    let path = route.split_once('?').map_or(route, |(path, _)| path);
    let Some(first) = path.split('/').find(|segment| !segment.is_empty()) else {
        return false;
    };
    first.eq_ignore_ascii_case("nhash") || first.to_ascii_lowercase().starts_with("nhash1")
}

fn parse_drive_nhash_file_route(route: &str) -> Result<DriveNhashFileLink> {
    let path = route.split_once('?').map_or(route, |(path, _)| path);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next().ok_or_else(|| anyhow!("missing nhash"))?;
    let raw_nhash = if first.eq_ignore_ascii_case("nhash") {
        segments.next().ok_or_else(|| anyhow!("missing nhash"))?
    } else {
        first
    };
    let nhash = percent_decode_path_component(raw_nhash)?
        .trim()
        .to_ascii_lowercase();
    if !nhash.starts_with("nhash1") {
        return Err(anyhow!("expected nhash1... content id"));
    }
    nhash_decode(&nhash).context("invalid nhash")?;

    let path_segments = segments
        .map(percent_decode_path_component)
        .collect::<Result<Vec<_>>>()?;
    validate_drive_content_path_segments(&path_segments)?;
    let path_hint = path_segments.join("/");
    let display_name = path_segments
        .last()
        .filter(|segment| !segment.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| nhash.clone());

    Ok(DriveNhashFileLink {
        nhash,
        path_hint,
        display_name,
    })
}

fn classify_drive_mutable_file_link_input(input: &str) -> Option<LinkInputClassification> {
    let route = drive_iris_to_fragment_or_path_route(input)?;
    if !drive_route_could_be_mutable_file(route) {
        return None;
    }

    let mut classification = LinkInputClassification {
        kind: "mutable_file".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: true,
        ..LinkInputClassification::default()
    };

    match parse_drive_mutable_file_route(route) {
        Ok(link) => {
            classification.is_valid = true;
            classification.content_path_hint = link.path_hint;
            classification.open_display_name = link.display_name;
            classification.local_open_url = local_portal_npub_path_url(
                DEFAULT_GATEWAY_PORT,
                &link.npub,
                &link.tree_name,
                &link.path_segments,
            );
        }
        Err(error) => {
            classification.error = error.to_string();
            if classification.error.contains("missing") {
                classification.is_complete = false;
            }
        }
    }

    Some(classification)
}

fn drive_route_could_be_mutable_file(route: &str) -> bool {
    let path = route.split_once('?').map_or(route, |(path, _)| path);
    let Some(first) = path.split('/').find(|segment| !segment.is_empty()) else {
        return false;
    };
    first.to_ascii_lowercase().starts_with("npub1")
}

fn parse_drive_mutable_file_route(route: &str) -> Result<DriveMutableFileLink> {
    let path = route.split_once('?').map_or(route, |(path, _)| path);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let raw_npub = segments.next().ok_or_else(|| anyhow!("missing npub"))?;
    let decoded_npub = percent_decode_path_component(raw_npub)?
        .trim()
        .to_ascii_lowercase();
    if !decoded_npub.starts_with("npub1") {
        return Err(anyhow!("expected npub1... content owner"));
    }
    let pubkey = PublicKey::from_bech32(&decoded_npub).context("invalid npub")?;
    let npub = pubkey_npub(&pubkey.to_hex());

    let tree_name = percent_decode_path_component(
        segments
            .next()
            .ok_or_else(|| anyhow!("missing hashtree name"))?,
    )?
    .trim()
    .to_owned();
    if tree_name.is_empty() {
        return Err(anyhow!("missing hashtree name"));
    }
    validate_drive_content_path_segments(std::slice::from_ref(&tree_name))?;

    let path_segments = segments
        .map(percent_decode_path_component)
        .collect::<Result<Vec<_>>>()?;
    if path_segments.is_empty() {
        return Err(anyhow!("missing content path"));
    }
    validate_drive_content_path_segments(&path_segments)?;
    let path_hint = path_segments.join("/");
    let display_name = path_segments
        .last()
        .filter(|segment| !segment.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| tree_name.clone());

    Ok(DriveMutableFileLink {
        npub,
        tree_name,
        path_segments,
        path_hint,
        display_name,
    })
}

fn validate_drive_content_path_segments(path_segments: &[String]) -> Result<()> {
    for segment in path_segments {
        if segment == "."
            || segment == ".."
            || segment.contains('\0')
            || segment.contains('/')
            || segment.contains('\\')
        {
            return Err(anyhow!("invalid content path hint"));
        }
    }
    Ok(())
}

fn classify_iris_web_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("http://") {
        return classify_local_iris_web_link_input(input);
    }
    if lower.starts_with("https://") {
        return classify_public_iris_web_link_input(input);
    }
    None
}

fn classify_local_iris_web_link_input(input: &str) -> Option<LinkInputClassification> {
    let (host, _) = http_host_and_tail(input, "http://")?;
    let host = strip_port(&host);
    let lower_host = host.to_ascii_lowercase();
    let is_isolated_local_origin = lower_host == "iris.localhost"
        || lower_host.ends_with(".iris.localhost")
        || lower_host.ends_with(".hash.localhost");
    if !is_isolated_local_origin {
        return None;
    }
    Some(LinkInputClassification {
        kind: "iris_web".to_owned(),
        is_complete: true,
        is_valid: true,
        normalized_input: input.to_owned(),
        open_display_name: iris_web_display_name(&lower_host),
        local_open_url: input.to_owned(),
        ..LinkInputClassification::default()
    })
}

fn classify_public_iris_web_link_input(input: &str) -> Option<LinkInputClassification> {
    let (host, tail) = http_host_and_tail(input, "https://")?;
    let host = strip_port(&host);
    let lower_host = host.to_ascii_lowercase();
    let tree_name = if lower_host == "iris.to" {
        "sites".to_owned()
    } else {
        lower_host.strip_suffix(".iris.to")?.to_owned()
    };
    if !is_dns_site_label(&tree_name) {
        return Some(LinkInputClassification {
            kind: "iris_web".to_owned(),
            is_complete: true,
            normalized_input: input.to_owned(),
            open_display_name: tree_name,
            error: "Iris app host is not an isolated app label".to_owned(),
            ..LinkInputClassification::default()
        });
    }
    Some(LinkInputClassification {
        kind: "iris_web".to_owned(),
        is_complete: true,
        is_valid: true,
        normalized_input: input.to_owned(),
        open_display_name: iris_web_display_name(&tree_name),
        local_open_url: local_iris_web_url(&tree_name, tail),
        ..LinkInputClassification::default()
    })
}

fn http_host_and_tail<'a>(input: &'a str, scheme: &str) -> Option<(String, &'a str)> {
    let rest = input.get(scheme.len()..)?;
    if rest.starts_with('/') || rest.starts_with('@') {
        return None;
    }
    let split_at = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..split_at].trim_end_matches('.').to_owned();
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some((host, &rest[split_at..]))
}

fn strip_port(host: &str) -> &str {
    host.rsplit_once(':')
        .and_then(|(name, port)| port.parse::<u16>().ok().map(|_| name))
        .unwrap_or(host)
}

fn local_iris_web_url(tree_name: &str, tail: &str) -> String {
    let mut url = local_mutable_site_url(DEFAULT_GATEWAY_PORT, IRIS_SITES_PORTAL_NPUB, tree_name);
    let tail = if tail.is_empty() { "/" } else { tail };
    if let Some(rest) = tail.strip_prefix('/') {
        url.push_str(rest);
    } else {
        url.push_str(tail);
    }
    url
}

fn iris_web_display_name(label: &str) -> String {
    if label == "iris.localhost" || label == "sites" {
        return "Iris Apps".to_owned();
    }
    label
        .split('.')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(label)
        .to_owned()
}

fn decoded_first_query_value_or_default(query: &str, names: &[&str]) -> Result<String> {
    match decoded_first_query_value(query, names)? {
        Some(value) => Ok(value.trim().to_owned()),
        None => Ok(String::new()),
    }
}

fn decoded_first_query_value(query: &str, names: &[&str]) -> Result<Option<String>> {
    for name in names {
        if let Some(value) = decoded_query_value(query, name)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn link_route_matches(input: &str, route: &str, allow_path_suffix: bool) -> bool {
    let Some(rest) = input.strip_prefix(route) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('?') || (allow_path_suffix && rest.starts_with('/'))
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

fn decoded_query_value(query: &str, name: &str) -> Result<Option<String>> {
    raw_query_value(query, name)
        .map(|value| percent_decode_query_component(&value))
        .transpose()
}

fn percent_decode_path_component(value: &str) -> Result<String> {
    percent_decode_component(value, false)
}

fn percent_decode_query_component(value: &str) -> Result<String> {
    percent_decode_component(value, true)
}

fn percent_decode_component(value: &str, plus_is_space: bool) -> Result<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if plus_is_space => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let hex = value
                    .get(index + 1..index + 3)
                    .ok_or_else(|| anyhow!("invalid percent escape"))?;
                let byte = u8::from_str_radix(hex, 16).context("invalid percent escape")?;
                out.push(byte);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).context("invalid utf-8 in percent escape")
}

#[cfg(test)]
mod tests;
