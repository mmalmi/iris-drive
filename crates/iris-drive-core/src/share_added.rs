//! Telling a device that it was let into a shared folder.
//!
//! Iris Drive web learns of a folder it was added to from a note addressed
//! to the device, readable only by it: relays and anyone else see a label
//! and the device, not which folder, its name or who shares it. The note
//! grants nothing. A device that reads one waits for the access list signed
//! by the note's author and adds the folder once that list includes it.
//!
//! The wire format matches Iris Drive web (`src/drive/deviceNotes.ts` and
//! `buildShareAddedNotices` in `src/drive/shareJoinLink.ts`).

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{
    Alphabet, Event, EventBuilder, Filter, Keys, Kind, PublicKey, SingleLetterTag, Tag, TagKind,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::NostrIdentityId;
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::sharing::{ShareRole, SharedFolder};

pub mod outbox;
pub use outbox::{ShareAddedOutbox, publish_share_added_notices};

pub const SHARE_ADDED_LABEL: &str = "iris-drive/share-added";
const MAX_CONTENT_BYTES: usize = 4096;
const MAX_NAME_CHARS: usize = 200;

/// What a device is told when it is added to a folder directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareAddedNotice {
    pub v: u8,
    pub share_id: NostrIdentityId,
    pub owner_profile_id: NostrIdentityId,
    pub name: String,
    pub role: ShareRole,
}

impl ShareAddedNotice {
    /// What the devices of a member with `role` are told about `folder`.
    /// The role is a hint and Iris Drive web reads none above editor: the
    /// access list a device then waits for gives it the role it really has.
    #[must_use]
    pub fn new(folder: &SharedFolder, role: ShareRole) -> Self {
        Self {
            v: 1,
            share_id: folder.share_id,
            owner_profile_id: folder.owner_profile_id,
            name: folder.display_name.clone(),
            role: match role {
                ShareRole::Reader => ShareRole::Reader,
                ShareRole::Admin | ShareRole::Editor => ShareRole::Editor,
            },
        }
    }
}

/// The `d` tag of a note with `label` about `about`, for one device. It
/// names what the note is about without saying it, so a newer note about the
/// same thing replaces the older one.
#[must_use]
pub fn device_note_d_tag(label: &str, about: &[&str], device_pubkey: &str) -> String {
    let mut lines = vec![label];
    lines.extend_from_slice(about);
    lines.push(device_pubkey);
    hex::encode(Sha256::digest(lines.join("\n")))
}

fn share_added_d_tag(share_id: NostrIdentityId, device_pubkey: &str) -> String {
    device_note_d_tag(SHARE_ADDED_LABEL, &[&share_id.to_string()], device_pubkey)
}

fn label_tag_kind() -> TagKind<'static> {
    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L))
}

/// One notice per device, each readable by that device alone.
pub fn build_share_added_notices(
    signer_keys: &Keys,
    notice: &ShareAddedNotice,
    device_pubkeys: &[String],
    created_at: i64,
) -> Result<Vec<Event>> {
    let payload = serde_json::to_string(notice)?;
    let mut told = BTreeSet::new();
    let mut events = Vec::new();
    for device_pubkey in device_pubkeys {
        if !told.insert(device_pubkey) {
            continue;
        }
        let device = PublicKey::from_hex(device_pubkey)
            .with_context(|| format!("device key {device_pubkey}"))?;
        let content = nip44::encrypt(
            signer_keys.secret_key(),
            &device,
            payload.as_str(),
            Nip44Version::V2,
        )?;
        events.push(
            EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content)
                .tags([
                    Tag::identifier(share_added_d_tag(notice.share_id, device_pubkey)),
                    Tag::custom(label_tag_kind(), [SHARE_ADDED_LABEL]),
                    Tag::public_key(device),
                ])
                .custom_created_at(nostr_sdk::Timestamp::from(
                    u64::try_from(created_at).unwrap_or(0),
                ))
                .sign_with_keys(signer_keys)?,
        );
    }
    Ok(events)
}

/// Notices addressed to `device_pubkey`, as Iris Drive web asks for them.
#[must_use]
pub fn share_added_filter(device_pubkey: &str) -> Option<Filter> {
    let device = PublicKey::from_hex(device_pubkey).ok()?;
    Some(
        Filter::new()
            .kind(Kind::from(KIND_DRIVE_ROOT))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::L), SHARE_ADDED_LABEL)
            .pubkey(device),
    )
}

/// The notice, if the event is one for this device and reads as one. Its
/// author is the admin whose signed access list the device then waits for.
#[must_use]
pub fn read_share_added_notice(event: &Event, device_keys: &Keys) -> Option<ShareAddedNotice> {
    let device_pubkey = device_keys.public_key().to_hex();
    let values = |name: &str| {
        event
            .tags
            .iter()
            .map(Tag::as_slice)
            .filter(|tag| tag.first().map(String::as_str) == Some(name))
            .filter_map(|tag| tag.get(1).map(String::as_str))
            .collect::<Vec<_>>()
    };
    if event.kind.as_u16() != KIND_DRIVE_ROOT
        || event.content.len() > MAX_CONTENT_BYTES
        || !values("l").contains(&SHARE_ADDED_LABEL)
        || values("p") != [device_pubkey.as_str()]
        || event.verify().is_err()
    {
        return None;
    }
    let payload = nip44::decrypt(device_keys.secret_key(), &event.pubkey, &event.content).ok()?;
    let mut notice: ShareAddedNotice = serde_json::from_str(&payload).ok()?;
    if notice.v != 1
        || notice.role == ShareRole::Admin
        || values("d").first().copied()
            != Some(share_added_d_tag(notice.share_id, &device_pubkey).as_str())
    {
        return None;
    }
    notice.name = notice.name.chars().take(MAX_NAME_CHARS).collect();
    Some(notice)
}

#[cfg(test)]
mod tests;
