//! The signed events that carry a device's list of shared folders to the
//! profile's other devices. Like a drive root each is a replaceable event of
//! the device; the content is encrypted with a fresh key that is wrapped for
//! each of the profile's devices, so relays and other people learn nothing
//! of the folders from it.
//!
//! A short list is one event. A long one is split over several, by folder, so
//! no event outgrows what relays accept and a change resends one part only.
//! A reader merges every event it is given, whatever the split.

use std::collections::BTreeMap;
use std::path::Path;

use aes_gcm::aead::{Aead, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{
    Alphabet, Event, EventBuilder, Filter, JsonUtil, Keys, Kind, PublicKey, SingleLetterTag, Tag,
    TagKind,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{MAX_SHARE_INDEX_ENTRIES, ShareIndex, ShareIndexEntry};
use crate::config::AppConfig;
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::paths::key_path_in;
use crate::{AppKey, NostrIdentityId, PRIMARY_DRIVE_ID, ProfileState};

pub const SHARE_INDEX_LABEL: &str = "iris-drive/share-index";
/// The events this device last signed for its list.
pub const SHARE_INDEX_EVENT_FILE: &str = "share-index-event.json";
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const NONCE_LEN: usize = 12;
/// How much of the list one event carries before it is split, as JSON. The
/// event that results stays well under the 64 KB many relays accept.
const PART_BYTES: usize = 16 * 1024;
/// The most parts a list is split into.
const MAX_PARTS: u32 = 1024;

#[derive(Debug, Error)]
pub enum ShareIndexEventError {
    /// The list was not wrapped for this device: it was sent before the
    /// device was linked, and its sender sends it again once it knows.
    #[error("the shared folder list is not readable by this device yet")]
    KeyUnavailable,
    #[error("shared folder list: {0}")]
    Invalid(String),
}

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u8,
    /// The list's key, wrapped for each of the profile's devices.
    key_wraps: BTreeMap<String, String>,
    /// `nonce.ciphertext`, base64url.
    data: String,
}

#[derive(Serialize, Deserialize)]
struct Doc {
    entries: Vec<ShareIndexEntry>,
}

/// The `d` tag of part `part` of a device's list. The first part has the
/// plain name, so a list that fits one event is a single address.
#[must_use]
pub fn share_index_d_tag(root_scope_id: &str, part: u32) -> String {
    if part == 0 {
        format!("iris-drive/{root_scope_id}/share-index")
    } else {
        format!("iris-drive/{root_scope_id}/share-index/{part}")
    }
}

fn scope_tag_kind() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::I)
}

fn label_tag_kind() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::L)
}

/// Every part of every device's list, for the profile `root_scope_id`. As
/// with drive roots the author is checked when the event is applied, so
/// devices linked later are heard without subscribing again.
#[must_use]
pub fn share_index_filter(root_scope_id: &str) -> Filter {
    Filter::new()
        .kind(Kind::from(KIND_DRIVE_ROOT))
        .custom_tag(label_tag_kind(), SHARE_INDEX_LABEL)
        .custom_tag(scope_tag_kind(), root_scope_id)
}

fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

#[must_use]
pub fn is_share_index_event(event: &Event) -> bool {
    event.kind.as_u16() == KIND_DRIVE_ROOT && tag_value(event, "l") == Some(SHARE_INDEX_LABEL)
}

/// The profile an event says its list belongs to.
#[must_use]
pub fn share_index_event_scope(event: &Event) -> Option<&str> {
    tag_value(event, "i")
}

fn invalid(error: impl std::fmt::Display) -> ShareIndexEventError {
    ShareIndexEventError::Invalid(error.to_string())
}

pub fn build_share_index_event<'a>(
    keys: &Keys,
    root_scope_id: &str,
    part: u32,
    entries: impl IntoIterator<Item = &'a ShareIndexEntry>,
    recipients: &[String],
    created_at: u64,
) -> Result<Event, ShareIndexEventError> {
    let doc = Doc {
        entries: entries.into_iter().cloned().collect(),
    };
    let plaintext = serde_json::to_vec(&doc).map_err(invalid)?;
    let mut key = [0_u8; 32];
    OsRng.fill_bytes(&mut key);
    let mut nonce = [0_u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = Aes256Gcm::new_from_slice(&key)
        .map_err(invalid)?
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(invalid)?;
    let mut key_wraps = BTreeMap::new();
    let own = keys.public_key().to_hex();
    for recipient in recipients.iter().chain(std::iter::once(&own)) {
        if key_wraps.contains_key(recipient) {
            continue;
        }
        let pubkey = PublicKey::from_hex(recipient).map_err(invalid)?;
        let wrap = nip44::encrypt(
            keys.secret_key(),
            &pubkey,
            hex::encode(key),
            Nip44Version::V2,
        )
        .map_err(invalid)?;
        key_wraps.insert(recipient.clone(), wrap);
    }
    let content = serde_json::to_string(&Wire {
        v: 1,
        key_wraps,
        data: format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ),
    })
    .map_err(invalid)?;
    EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content)
        .tags([
            Tag::identifier(share_index_d_tag(root_scope_id, part)),
            Tag::custom(TagKind::SingleLetter(label_tag_kind()), [SHARE_INDEX_LABEL]),
            Tag::custom(TagKind::SingleLetter(scope_tag_kind()), [root_scope_id]),
        ])
        .custom_created_at(nostr_sdk::Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(invalid)
}

/// Check the event's signature and open it with this device's key.
pub fn parse_share_index_event(
    event: &Event,
    keys: &Keys,
) -> Result<Vec<ShareIndexEntry>, ShareIndexEventError> {
    if !is_share_index_event(event) {
        return Err(invalid("not a shared folder list"));
    }
    if event.content.len() > MAX_CONTENT_BYTES {
        return Err(invalid("too large"));
    }
    event.verify().map_err(invalid)?;
    let wire: Wire = serde_json::from_str(&event.content).map_err(invalid)?;
    if wire.v != 1 {
        return Err(invalid(format!("unsupported version {}", wire.v)));
    }
    let wrap = wire
        .key_wraps
        .get(&keys.public_key().to_hex())
        .ok_or(ShareIndexEventError::KeyUnavailable)?;
    let key_hex = nip44::decrypt(keys.secret_key(), &event.pubkey, wrap).map_err(invalid)?;
    let key = hex::decode(key_hex).map_err(invalid)?;
    let (nonce, ciphertext) = wire
        .data
        .split_once('.')
        .ok_or_else(|| invalid("malformed"))?;
    let nonce = URL_SAFE_NO_PAD.decode(nonce).map_err(invalid)?;
    if nonce.len() != NONCE_LEN {
        return Err(invalid("malformed"));
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(ciphertext).map_err(invalid)?;
    let plaintext = Aes256Gcm::new_from_slice(&key)
        .map_err(invalid)?
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| invalid("could not be decrypted"))?;
    let doc: Doc = serde_json::from_slice(&plaintext).map_err(invalid)?;
    if doc.entries.len() > MAX_SHARE_INDEX_ENTRIES {
        return Err(invalid("too many folders"));
    }
    Ok(doc.entries)
}

/// Which part of a list split `parts` ways a folder goes in. `parts` is a
/// power of two, so doubling it only moves folders into the new parts.
fn part_of(share_id: NostrIdentityId, parts: u32) -> u32 {
    let digest = Sha256::digest(share_id.to_string().as_bytes());
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % parts
}

/// Split a list so that no part outgrows one event, in at least `at_least`
/// parts. A part can be empty.
pub(super) fn split_into_parts<'a>(
    entries: &[&'a ShareIndexEntry],
    at_least: u32,
) -> Vec<Vec<&'a ShareIndexEntry>> {
    let mut parts = at_least.clamp(1, MAX_PARTS).next_power_of_two();
    loop {
        let mut split = vec![Vec::new(); parts as usize];
        let mut sizes = vec![0_usize; parts as usize];
        for entry in entries.iter().copied() {
            let part = part_of(entry.share_id, parts) as usize;
            sizes[part] += serde_json::to_vec(entry).map_or(0, |json| json.len() + 1);
            split[part].push(entry);
        }
        if parts >= MAX_PARTS || sizes.iter().all(|size| *size <= PART_BYTES) {
            return split;
        }
        parts *= 2;
    }
}

/// The event last signed for one part of this device's list, so the same
/// part is the same event wherever it is sent, and whether relays have it.
#[derive(Serialize, Deserialize)]
struct Signed {
    version: String,
    event_json: String,
    #[serde(default)]
    published: bool,
}

/// What this device last signed, by part. The number of parts never
/// shrinks, so a part once published is always replaced, never orphaned.
#[derive(Default, Serialize, Deserialize)]
struct SignedParts {
    #[serde(default)]
    parts: BTreeMap<u32, Signed>,
    #[serde(default)]
    count: u32,
}

fn load_signed(config_dir: &Path) -> SignedParts {
    std::fs::read(config_dir.join(SHARE_INDEX_EVENT_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_signed(config_dir: &Path, signed: &SignedParts) -> Result<()> {
    let path = config_dir.join(SHARE_INDEX_EVENT_FILE);
    crate::atomic_file::atomic_write_private(&path, &serde_json::to_vec(signed)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// The profile's devices, the same ones a drive root is wrapped for.
fn recipients(config: &AppConfig, state: &ProfileState) -> Vec<String> {
    config.drive(PRIMARY_DRIVE_ID).map_or_else(
        || state.active_root_writer_app_key_pubkeys(),
        |drive| crate::drive_root_recipient_app_key_pubkeys(state, drive),
    )
}

/// This device's list as signed events for the profile's current devices:
/// one per part, each with whether relays already have that very event.
/// Empty while the device has no shared folder to tell them about.
pub fn current_share_index_events(
    config_dir: &Path,
    config: &AppConfig,
) -> Result<Vec<(Event, bool)>> {
    let Some(state) = config
        .profile
        .as_ref()
        .filter(|state| state.can_write_roots())
    else {
        return Ok(Vec::new());
    };
    let index = ShareIndex::load_for(config_dir, config)?;
    if index.entries.is_empty() {
        return Ok(Vec::new());
    }
    let recipients = recipients(config, state);
    let root_scope_id = state.root_scope_id();
    let mut signed = load_signed(config_dir);
    let listed = index.entries.values().collect::<Vec<_>>();
    let split = split_into_parts(&listed, signed.count);
    let count = u32::try_from(split.len()).unwrap_or(MAX_PARTS);
    let mut resigned = signed.count != count;
    signed.count = count;
    let mut events = Vec::new();
    let mut app_key = None;
    for (part, entries) in (0_u32..).zip(split) {
        if entries.is_empty() && !signed.parts.contains_key(&part) {
            continue;
        }
        let version = hex::encode(Sha256::digest(serde_json::to_vec(&(
            &root_scope_id,
            part,
            &entries,
            &recipients,
        ))?));
        if let Some(current) = signed.parts.get(&part).filter(|s| s.version == version) {
            let event =
                Event::from_json(&current.event_json).context("reading shared folder list")?;
            events.push((event, current.published));
            continue;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let created_at = crate::drive_root_clock::reserve(
            config_dir,
            &share_index_d_tag(&root_scope_id, part),
            &version,
            now,
        )
        .context("reserving shared folder list time")?;
        if app_key.is_none() {
            app_key = Some(AppKey::load(key_path_in(config_dir)).context("loading app key")?);
        }
        let Some(keys) = app_key.as_ref() else {
            continue;
        };
        let event = build_share_index_event(
            keys.keys(),
            &root_scope_id,
            part,
            entries,
            &recipients,
            created_at,
        )?;
        signed.parts.insert(
            part,
            Signed {
                version,
                event_json: event.as_json(),
                published: false,
            },
        );
        resigned = true;
        events.push((event, false));
    }
    if resigned {
        save_signed(config_dir, &signed)?;
    }
    Ok(events)
}

/// Note that relays were given `event`, unless a newer one replaced it.
pub(super) fn mark_published(config_dir: &Path, event: &Event) -> Result<()> {
    let mut signed = load_signed(config_dir);
    let json = event.as_json();
    let Some(part) = signed
        .parts
        .values_mut()
        .find(|part| !part.published && part.event_json == json)
    else {
        return Ok(());
    };
    part.published = true;
    save_signed(config_dir, &signed)
}
