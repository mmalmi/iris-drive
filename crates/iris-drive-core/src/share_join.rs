//! Join links: the link-first way to share a folder.
//!
//! Access to a folder is granted to a person's device keys, so a link cannot
//! carry access by itself. A join link carries a key of its own instead. The
//! visitor signs a join request with it, which proves they hold the link, and
//! an admin's Drive adds the visitor's devices when it sees the request.
//!
//! The wire format matches Iris Drive web (`src/drive/shareJoinLink.ts`), so a
//! link made in either opens in the other.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::{
    Alphabet, Event, EventBuilder, Filter, Keys, Kind, SecretKey, SingleLetterTag, Tag, TagKind,
};
use serde::{Deserialize, Serialize};

use crate::NostrIdentityId;
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::sharing::{
    ResolvedShareRecipient, ShareMemberStatus, ShareRecipientProfileEvidence, ShareRole,
    SharedFolder, SharingError, parse_share_access_snapshot_event, project_share_access,
    resolve_share_recipient_from_evidence, shared_folder_app_key_can_admin,
};

pub mod apply;
pub mod store;
pub mod subscriptions;
pub use apply::{
    ShareJoinEventOutcome, ShareJoinSyncReport, apply_share_join_event, complete_share_join,
    is_share_join_event, pending_share_join_requests, publish_approved_share,
    publish_pending_share_join_requests, record_share_join, share_join_filters,
    sync_share_joins_once,
};
pub use store::{PendingShareJoin, ShareJoinStore};
pub use subscriptions::{
    ShareRelaySubscriptionState, refresh_share_relay_subscriptions, share_relay_filters,
};

pub const SHARE_JOIN_LABEL: &str = "iris-drive/share-join";
pub const SHARE_JOIN_ROUTE: &str = "/share-join/";
pub const SHARE_JOIN_WEB_ORIGIN: &str = "https://drive.iris.to";
const MAX_JOIN_REQUEST_BYTES: usize = 256 * 1024;
const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareJoinLink {
    pub v: u8,
    pub share_id: NostrIdentityId,
    pub owner_profile_id: NostrIdentityId,
    /// The admin device whose signed access list the visitor trusts.
    pub admin: String,
    pub name: String,
    pub role: ShareRole,
    /// The link's own secret key, hex.
    pub key: String,
    /// A document to open inside the folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

impl ShareJoinLink {
    /// A new link for `role` on `folder`, answered by the admin device `admin`.
    pub fn new(folder: &SharedFolder, admin: &str, role: ShareRole) -> Result<Self, SharingError> {
        if role == ShareRole::Admin {
            return Err(SharingError::Invite(
                "links can only add viewers or editors".into(),
            ));
        }
        Ok(Self {
            v: 1,
            share_id: folder.share_id,
            owner_profile_id: folder.owner_profile_id,
            admin: admin.to_ascii_lowercase(),
            name: folder.display_name.clone(),
            role,
            key: Keys::generate().secret_key().to_secret_hex(),
            doc: None,
        })
    }

    /// The public key requests made with this link are signed by.
    pub fn pubkey(&self) -> Result<String, SharingError> {
        Ok(self.keys()?.public_key().to_hex())
    }

    fn keys(&self) -> Result<Keys, SharingError> {
        SecretKey::from_hex(&self.key)
            .map(Keys::new)
            .map_err(|error| SharingError::Invite(error.to_string()))
    }

    #[must_use]
    pub fn url(&self) -> String {
        self.url_at(SHARE_JOIN_WEB_ORIGIN)
    }

    #[must_use]
    pub fn url_at(&self, origin: &str) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!(
            "{}/#{SHARE_JOIN_ROUTE}{}",
            origin.trim_end_matches('/'),
            URL_SAFE_NO_PAD.encode(json)
        )
    }

    pub fn parse(input: &str) -> Result<Self, SharingError> {
        let input = input.trim();
        let payload = input
            .split_once(SHARE_JOIN_ROUTE)
            .map_or(input, |(_, payload)| payload);
        let payload = payload
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .trim_end_matches('=');
        let invalid = || SharingError::Invite("This link is not valid".into());
        let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?;
        let link: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if link.v != 1
            || link.role == ShareRole::Admin
            || !is_hex32(&link.admin)
            || !is_hex32(&link.key)
            || link.doc.as_deref().is_some_and(|doc| doc.contains('/'))
        {
            return Err(invalid());
        }
        Ok(link)
    }

    /// Whether `input` looks like a join link rather than another kind of invite.
    #[must_use]
    pub fn is_link(input: &str) -> bool {
        input.contains(SHARE_JOIN_ROUTE)
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            "Shared folder".to_owned()
        } else {
            name.chars().take(MAX_NAME_CHARS).collect()
        }
    }
}

fn is_hex32(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn join_request_d_tag(share_id: NostrIdentityId, requester: &str) -> String {
    format!("{SHARE_JOIN_LABEL}/{share_id}/{requester}")
}

fn label_tag_kind() -> TagKind<'static> {
    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L))
}

/// One request per requesting device; sending it again replaces the earlier one.
pub fn build_share_join_request(
    link: &ShareJoinLink,
    evidence: &ShareRecipientProfileEvidence,
    requested_at: i64,
) -> Result<Event, SharingError> {
    let requester = evidence
        .representative_pubkey
        .as_deref()
        .filter(|pubkey| is_hex32(pubkey))
        .ok_or_else(|| SharingError::Invite("Missing device key".into()))?;
    let content =
        serde_json::to_string(evidence).map_err(|error| SharingError::Invite(error.to_string()))?;
    EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content)
        .tags([
            Tag::identifier(join_request_d_tag(link.share_id, requester)),
            Tag::custom(label_tag_kind(), [SHARE_JOIN_LABEL]),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::I)),
                [link.share_id.to_string()],
            ),
        ])
        .custom_created_at(nostr_sdk::Timestamp::from(
            u64::try_from(requested_at).unwrap_or(0),
        ))
        .sign_with_keys(&link.keys()?)
        .map_err(|error| SharingError::Invite(error.to_string()))
}

/// Requests made with any of `link_pubkeys`.
#[must_use]
pub fn share_join_request_filter(link_pubkeys: &[String]) -> Option<Filter> {
    let authors = link_pubkeys
        .iter()
        .filter_map(|pubkey| nostr_sdk::PublicKey::from_hex(pubkey).ok())
        .collect::<Vec<_>>();
    (!authors.is_empty()).then(|| {
        Filter::new()
            .kind(Kind::from(KIND_DRIVE_ROOT))
            .authors(authors)
            .custom_tag(SingleLetterTag::lowercase(Alphabet::L), SHARE_JOIN_LABEL)
    })
}

#[must_use]
pub fn is_share_join_request_event(event: &Event) -> bool {
    event.kind.as_u16() == KIND_DRIVE_ROOT
        && event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("l")
                && values.get(1).map(String::as_str) == Some(SHARE_JOIN_LABEL)
        })
}

/// Check a request against the link it claims to come from and resolve the
/// person who made it. Every signature inside is checked again here.
pub fn share_recipient_from_join_request(
    link: &ShareJoinLink,
    event: &Event,
) -> Result<(ResolvedShareRecipient, String), SharingError> {
    let not_from_link = || SharingError::Invite("Request was not made with this link".into());
    if event.pubkey.to_hex() != link.pubkey()?
        || !is_share_join_request_event(event)
        || event.content.len() > MAX_JOIN_REQUEST_BYTES
        || event.verify().is_err()
    {
        return Err(not_from_link());
    }
    let mut evidence: ShareRecipientProfileEvidence = serde_json::from_str(&event.content)
        .map_err(|error| SharingError::Invite(error.to_string()))?;
    let requester = evidence
        .representative_pubkey
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !is_hex32(&requester)
        || event.tags.identifier() != Some(join_request_d_tag(link.share_id, &requester).as_str())
    {
        return Err(SharingError::Invite(
            "Request does not match this folder".into(),
        ));
    }
    evidence.representative_pubkey = Some(requester.clone());
    evidence.representative_npub = None;
    evidence.display_name = evidence
        .display_name
        .map(|name| name.trim().chars().take(80).collect::<String>())
        .filter(|name| !name.is_empty());
    let resolved = resolve_share_recipient_from_evidence(&evidence, None)?;
    if !resolved.app_pubkeys.contains(&requester) {
        return Err(SharingError::RecipientResolution(
            "Requesting device is not part of the profile".into(),
        ));
    }
    Ok((resolved, requester))
}

/// Whether a request still needs an admin to act on it.
#[must_use]
pub fn join_request_needs_approval(
    folder: &SharedFolder,
    profile_id: NostrIdentityId,
    requester: &str,
) -> bool {
    if project_share_access(&folder.access)
        .members
        .get(&profile_id.to_string())
        .is_some_and(|member| member.status == ShareMemberStatus::Revoked)
    {
        return false;
    }
    !folder.projection().active_facets.contains_key(requester)
}

/// What a device already trusts about a folder it does not have yet: which
/// folder it is, and the admin device whose signed access list to start
/// from. A join link carries one; so does the profile's list of its shared
/// folders, for the profile's other devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareAnchor {
    pub share_id: NostrIdentityId,
    pub owner_profile_id: NostrIdentityId,
    pub admin: String,
    pub name: String,
}

impl ShareAnchor {
    /// Whether `event` is the access list of this folder as signed by its
    /// trusted admin, whoever it lets in.
    #[must_use]
    pub fn signed(&self, event: &Event) -> bool {
        event.pubkey.to_hex() == self.admin
            && parse_share_access_snapshot_event(event)
                .is_ok_and(|snapshot| snapshot.content.resource_id == self.share_id)
    }
}

impl ShareJoinLink {
    #[must_use]
    pub fn anchor(&self) -> ShareAnchor {
        ShareAnchor {
            share_id: self.share_id,
            owner_profile_id: self.owner_profile_id,
            admin: self.admin.clone(),
            name: self.display_name(),
        }
    }
}

/// The visitor's side: the admin's signed access list as a folder this device
/// can open, once the list includes this device. Only a list signed by the
/// admin named in the link is trusted as the starting point.
#[must_use]
pub fn shared_folder_from_access_event_for_join(
    link: &ShareJoinLink,
    event: &Event,
    app_pubkey: &str,
) -> Option<SharedFolder> {
    shared_folder_from_access_event(&link.anchor(), event, app_pubkey)
}

/// The trusted admin's signed access list as a folder this device can open,
/// once the list includes this device.
#[must_use]
pub fn shared_folder_from_access_event(
    anchor: &ShareAnchor,
    event: &Event,
    app_pubkey: &str,
) -> Option<SharedFolder> {
    if event.pubkey.to_hex() != anchor.admin {
        return None;
    }
    let snapshot = parse_share_access_snapshot_event(event).ok()?;
    if snapshot.content.resource_id != anchor.share_id {
        return None;
    }
    let mut folder = SharedFolder {
        share_id: anchor.share_id,
        owner_profile_id: anchor.owner_profile_id,
        source_path: String::new(),
        display_name: anchor.name.clone(),
        // Replaced below by the role the access list gives this device.
        local_role: ShareRole::Reader,
        access: snapshot.content,
        pending_invites: std::collections::BTreeMap::default(),
        app_key_roots: std::collections::BTreeMap::default(),
    };
    if !shared_folder_app_key_can_admin(&folder, &anchor.admin) {
        return None;
    }
    let member = folder.active_member_for_app_key(app_pubkey)?;
    if !folder.projection().active_facets.contains_key(app_pubkey) {
        return None;
    }
    folder.local_role = member.role;
    Some(folder)
}

#[cfg(test)]
#[path = "share_join/tests.rs"]
mod tests;
