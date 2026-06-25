//! Shared AppKey-link input parsing and validation.
//!
//! Keep route recognition and profile-scoped link-target rules here so CLI and
//! native shells render the same state instead of reimplementing identity
//! admission policy per platform.

use anyhow::{Context, Result, anyhow};
use hashtree_core::nhash_decode;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use crate::IrisProfileId;
use crate::app_key_link_invite::{
    APP_KEY_LINK_INVITE_PREFIX, APP_KEY_LINK_INVITE_WEB_PREFIX, parse_app_key_link_invite,
};
use crate::app_key_link_transport::{app_key_approval_query, parse_app_key_approval_request};
use crate::app_key_summary::pubkey_npub;
use crate::gateway::{
    DEFAULT_GATEWAY_PORT, IRIS_SITES_PORTAL_NPUB, is_dns_site_label, local_mutable_site_url,
    local_nhash_url, local_portal_npub_path_url,
};

const MANUAL_LINK_REQUIRES_PROFILE_AND_ADMIN: &str = "manual device linking requires an IrisProfile UUID and --admin-app-key; otherwise paste an admin invite URL";

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
    pub profile_id: IrisProfileId,
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
    "expected device key or IrisProfile invite link".clone_into(&mut classification.error);
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
            .ok_or_else(|| anyhow!("device invite is missing IrisProfile id"))?;
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
            .parse::<IrisProfileId>()
            .context("parsing IrisProfile UUID")?,
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
    let query = app_key_approval_query(input)?;
    let profile =
        raw_query_value(query, "profile").or_else(|| raw_query_value(query, "profile_id"));
    let app_key = raw_query_value(query, "app_key").or_else(|| raw_query_value(query, "appKey"));
    let is_complete = profile
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && app_key
            .as_deref()
            .is_some_and(app_key_pubkey_input_is_complete)
        && raw_query_value(query, "invite")
            .or_else(|| raw_query_value(query, "invite_pubkey"))
            .or_else(|| raw_query_value(query, "invitePubkey"))
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
                "device request was not recognized".clone_into(&mut classification.error);
            }
            Err(error) => classification.error = error.to_string(),
        }
    }
    classification.has_invite_pubkey = parse_app_key_approval_request(input)
        .ok()
        .flatten()
        .is_some_and(|request| !request.invite_pubkey.trim().is_empty());
    Some(classification)
}

fn classify_invite_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let is_canonical = [APP_KEY_LINK_INVITE_PREFIX, APP_KEY_LINK_INVITE_WEB_PREFIX]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || link_route_matches(&lower, "https://drive.iris.to/invite", true);
    if !is_canonical {
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

fn invite_link_input_is_complete(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    for prefix in [APP_KEY_LINK_INVITE_PREFIX, APP_KEY_LINK_INVITE_WEB_PREFIX] {
        if lower.starts_with(prefix) {
            return input[prefix.len()..].len() >= 32;
        }
    }
    false
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
        let invite_key = Keys::generate().public_key();
        let invite = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite_key.to_hex())
            .expect("invite");

        let invite_classification = classify_link_input(&invite);
        assert_eq!(invite_classification.kind, "invite");
        assert!(invite_classification.is_complete);
        assert!(invite_classification.is_valid);
        assert_eq!(
            invite_classification.admin_app_key_pubkey,
            admin.to_bech32().expect("npub")
        );
        assert!(invite_classification.has_invite_pubkey);

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

        let request = encode_app_key_approval_request(
            profile_id,
            &admin.to_hex(),
            &invite_key.to_hex(),
            None,
        );
        let approval = classify_link_input(&request);
        assert_eq!(approval.kind, "app_key_approval");
        assert!(approval.is_complete);
        assert!(approval.is_valid);
        assert_eq!(approval.app_key_pubkey, app_key_npub);
        assert!(approval.has_invite_pubkey);
    }

    #[test]
    fn classify_invite_routes_distinguishes_partial_and_nearby_links() {
        let short = classify_link_input("https://drive.iris.to/invite/demo");
        assert_eq!(short.kind, "invite");
        assert!(!short.is_complete);
        assert!(!short.is_valid);

        let unrelated = classify_link_input("https://drive.iris.to/app-key-linker?owner=npub1x");
        assert_eq!(unrelated.kind, "iris_web");
        assert!(unrelated.is_valid);

        let custom_scheme_invite = classify_link_input("iris-drive://invite/demo");
        assert_eq!(custom_scheme_invite.kind, "unknown");
    }

    #[test]
    fn classify_share_dialog_links_returns_folder_and_name() {
        let app_link =
            classify_link_input("iris-drive://share?path=My%20Drive%2FProjects&name=Projects");
        assert_eq!(app_link.kind, "share_dialog");
        assert!(app_link.is_complete);
        assert!(app_link.is_valid);
        assert_eq!(app_link.share_source_path, "My Drive/Projects");
        assert_eq!(app_link.share_display_name, "Projects");

        let hinted = classify_link_input(
            "https://drive.iris.to/share?path=Projects%2FAlpha&name=Alpha&recipient_npub=npub1alice&recipient_name=Alice&recipient_profile=123e4567-e89b-42d3-a456-426614174000",
        );
        assert_eq!(hinted.share_recipient_npub_hint, "npub1alice");
        assert_eq!(hinted.share_recipient_display_name, "Alice");
        assert_eq!(
            hinted.share_recipient_profile_id,
            "123e4567-e89b-42d3-a456-426614174000"
        );

        let web_link = classify_link_input("https://drive.iris.to/share?path=%2FShared%20Source");
        assert_eq!(web_link.kind, "share_dialog");
        assert!(web_link.is_complete);
        assert!(web_link.is_valid);
        assert_eq!(web_link.share_source_path, "/Shared Source");
        assert!(web_link.share_display_name.is_empty());

        let missing_path = classify_link_input("iris-drive://share?name=Nope");
        assert_eq!(missing_path.kind, "share_dialog");
        assert!(!missing_path.is_complete);
        assert!(!missing_path.is_valid);
    }

    #[test]
    fn classify_drive_nhash_file_link_opens_immutable_content() {
        let input = "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/freenet.pdf?fullscreen=1";
        let file = classify_link_input(input);

        assert_eq!(file.kind, "nhash_file");
        assert!(file.is_complete);
        assert!(file.is_valid);
        assert_eq!(
            file.content_nhash,
            "nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t"
        );
        assert_eq!(file.content_path_hint, "freenet.pdf");
        assert_eq!(file.open_display_name, "freenet.pdf");
        assert_eq!(
            file.local_open_url,
            "http://nhash.iris.localhost:17321/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/freenet.pdf"
        );

        let encoded = classify_link_input(
            "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/Freenet%20paper.pdf",
        );
        assert_eq!(encoded.content_path_hint, "Freenet paper.pdf");
        assert_eq!(encoded.open_display_name, "Freenet paper.pdf");

        let traversal = classify_link_input(
            "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/../secret.pdf",
        );
        assert_eq!(traversal.kind, "nhash_file");
        assert!(!traversal.is_valid);
    }

    #[test]
    fn classify_drive_npub_file_link_opens_mutable_content_path() {
        let input = format!(
            "https://drive.iris.to/#/{}/sites/docs/Freenet%20paper.pdf?fullscreen=1",
            crate::gateway::IRIS_SITES_PORTAL_NPUB
        );
        let file = classify_link_input(&input);

        assert_eq!(file.kind, "mutable_file");
        assert!(file.is_complete);
        assert!(file.is_valid);
        assert_eq!(file.content_path_hint, "docs/Freenet paper.pdf");
        assert_eq!(file.open_display_name, "Freenet paper.pdf");
        assert_eq!(
            file.local_open_url,
            format!(
                "http://iris.localhost:17321/{}/sites/docs/Freenet%20paper.pdf",
                crate::gateway::IRIS_SITES_PORTAL_NPUB
            )
        );
    }

    #[test]
    fn classify_public_iris_app_link_opens_isolated_local_origin() {
        let app = classify_link_input("https://calendar.iris.to/events/today?view=week#selected");

        assert_eq!(app.kind, "iris_web");
        assert!(app.is_complete);
        assert!(app.is_valid);
        assert_eq!(app.open_display_name, "calendar");
        assert_eq!(
            app.local_open_url,
            format!(
                "http://calendar.{}.iris.localhost:17321/events/today?view=week#selected",
                crate::gateway::IRIS_SITES_PORTAL_NPUB
            )
        );

        let portal = classify_link_input("https://iris.to/?launcher=1");
        assert_eq!(portal.kind, "iris_web");
        assert!(portal.is_valid);
        assert_eq!(portal.open_display_name, "Iris Apps");
        assert_eq!(
            portal.local_open_url,
            format!(
                "http://sites.{}.iris.localhost:17321/?launcher=1",
                crate::gateway::IRIS_SITES_PORTAL_NPUB
            )
        );
    }

    #[test]
    fn classify_local_iris_origin_stays_browser_only() {
        let local = classify_link_input("http://audio.npub1owner.iris.localhost:17321/album");

        assert_eq!(local.kind, "iris_web");
        assert!(local.is_valid);
        assert_eq!(
            local.local_open_url,
            "http://audio.npub1owner.iris.localhost:17321/album"
        );
    }

    #[test]
    fn classify_public_iris_app_link_rejects_non_isolated_host_labels() {
        let nested = classify_link_input("https://admin.calendar.iris.to/");

        assert_eq!(nested.kind, "iris_web");
        assert!(nested.is_complete);
        assert!(!nested.is_valid);
        assert!(nested.local_open_url.is_empty());
        assert!(
            nested
                .error
                .contains("Iris app host is not an isolated app label")
        );
    }

    #[test]
    fn resolve_app_key_link_target_accepts_invite_or_manual_profile_with_admin() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate().public_key();
        let invite_key = Keys::generate().public_key();
        let invite = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite_key.to_hex())
            .expect("invite");

        let from_invite = resolve_app_key_link_target(&invite, None).expect("invite target");
        assert_eq!(from_invite.profile_id, profile_id);
        assert_eq!(from_invite.admin_app_key_hex, admin.to_hex());
        assert_eq!(from_invite.invite_pubkey, invite_key.to_hex());

        let from_manual =
            resolve_app_key_link_target(&profile_id.to_string(), Some(&admin.to_hex()))
                .expect("manual target");
        assert_eq!(from_manual.profile_id, profile_id);
        assert_eq!(from_manual.admin_app_key_hex, admin.to_hex());
        assert!(from_manual.invite_pubkey.is_empty());
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
