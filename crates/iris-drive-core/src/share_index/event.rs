//! The signed event that carries a device's list of shared folders to the
//! profile's other devices. Like a drive root it is one replaceable event per
//! device; its content is encrypted with a key wrapped for each of the
//! profile's devices, so relays and other people learn nothing from it.

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
use crate::{AppKey, PRIMARY_DRIVE_ID, ProfileState};

pub const SHARE_INDEX_LABEL: &str = "iris-drive/share-index";
const SIGNED_FILE: &str = "share-index-event.json";
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const NONCE_LEN: usize = 12;

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

#[must_use]
pub fn share_index_d_tag(root_scope_id: &str) -> String {
    format!("iris-drive/{root_scope_id}/share-index")
}

/// Lists from any device, for the profile `root_scope_id`. As with drive
/// roots the author is checked when the event is applied, so devices linked
/// later are heard without subscribing again.
#[must_use]
pub fn share_index_filter(root_scope_id: &str) -> Filter {
    Filter::new().kind(Kind::from(KIND_DRIVE_ROOT)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::D),
        share_index_d_tag(root_scope_id),
    )
}

#[must_use]
pub fn is_share_index_event(event: &Event) -> bool {
    event.kind.as_u16() == KIND_DRIVE_ROOT
        && event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("l")
                && values.get(1).map(String::as_str) == Some(SHARE_INDEX_LABEL)
        })
}

fn invalid(error: impl std::fmt::Display) -> ShareIndexEventError {
    ShareIndexEventError::Invalid(error.to_string())
}

pub fn build_share_index_event<'a>(
    keys: &Keys,
    root_scope_id: &str,
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
            Tag::identifier(share_index_d_tag(root_scope_id)),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
                [SHARE_INDEX_LABEL],
            ),
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

/// The event last signed for this device's list, so the same list is the
/// same event wherever it is sent, and whether relays were given it.
#[derive(Serialize, Deserialize)]
struct Signed {
    version: String,
    event_json: String,
    #[serde(default)]
    published: bool,
}

fn load_signed(config_dir: &Path) -> Option<Signed> {
    let bytes = std::fs::read(config_dir.join(SIGNED_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_signed(config_dir: &Path, signed: &Signed) -> Result<()> {
    let path = config_dir.join(SIGNED_FILE);
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

/// This device's list as a signed event for the profile's current devices,
/// or `None` while it has no shared folder to tell them about. The second
/// value says whether relays already have this very event.
pub fn current_share_index_event(
    config_dir: &Path,
    config: &AppConfig,
) -> Result<Option<(Event, bool)>> {
    let Some(state) = config
        .profile
        .as_ref()
        .filter(|state| state.can_write_roots())
    else {
        return Ok(None);
    };
    let index = ShareIndex::load(config_dir)?;
    if index.entries.is_empty() {
        return Ok(None);
    }
    let recipients = recipients(config, state);
    let root_scope_id = state.root_scope_id();
    let version = hex::encode(Sha256::digest(serde_json::to_vec(&(
        &root_scope_id,
        &index.entries,
        &recipients,
    ))?));
    if let Some(signed) = load_signed(config_dir).filter(|signed| signed.version == version) {
        let event = Event::from_json(&signed.event_json).context("reading shared folder list")?;
        return Ok(Some((event, signed.published)));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let created_at = crate::drive_root_clock::reserve(
        config_dir,
        &share_index_d_tag(&root_scope_id),
        &version,
        now,
    )
    .context("reserving shared folder list time")?;
    let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let event = build_share_index_event(
        app_key.keys(),
        &root_scope_id,
        index.entries.values(),
        &recipients,
        created_at,
    )?;
    save_signed(
        config_dir,
        &Signed {
            version,
            event_json: event.as_json(),
            published: false,
        },
    )?;
    Ok(Some((event, false)))
}

/// Note that relays were given `event`, unless a newer list replaced it.
pub(super) fn mark_published(config_dir: &Path, event: &Event) -> Result<()> {
    let Some(mut signed) = load_signed(config_dir) else {
        return Ok(());
    };
    if signed.published || signed.event_json != event.as_json() {
        return Ok(());
    }
    signed.published = true;
    save_signed(config_dir, &signed)
}
