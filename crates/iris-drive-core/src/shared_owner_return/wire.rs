use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, TreeEntry};
use nostr_sdk::nips::nip44;
use nostr_sdk::{
    Alphabet, Event, EventBuilder, Filter, JsonUtil, Keys, Kind, PublicKey, SingleLetterTag, Tag,
    TagKind, Timestamp,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::{Uuid, Version};

use crate::NostrIdentityId;
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::share_added::device_note_d_tag;

pub const OWNER_META: &str = "iris_file_owner";
pub const RETURN_META: &str = "iris_file_return";
pub const RETURNED_FROM_META: &str = "iris_returned_from";
const OWNER_LABEL: &str = "iris-drive-file-owner-v1";
const RECEIPT_LABEL: &str = "iris-drive-file-return-stored-v1";
const REQUEST_LABEL: &str = "iris-drive-file-return-request-v1";
pub const NOTE_LABEL: &str = "iris-drive-file-return-v1";
pub(super) const MAX_SAFE_SEQUENCE: u64 = (1 << 53) - 1;
const MAX_EVENT_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOwner {
    pub v: u8,
    pub share: NostrIdentityId,
    pub item: String,
    pub owner: NostrIdentityId,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReturn {
    pub v: u8,
    pub share: NostrIdentityId,
    pub item: String,
    pub owner: NostrIdentityId,
    pub claim: String,
    pub revision: String,
    pub path: Vec<String>,
    pub nonce: String,
    pub seen: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReturnPhase {
    Request,
    Stored,
}

impl ReturnPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Stored => "stored",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnNote {
    pub v: u8,
    pub phase: ReturnPhase,
    pub request: Event,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Event>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    v: u8,
    share: NostrIdentityId,
    item: String,
    owner: NostrIdentityId,
    claim: String,
    request: String,
    revision: String,
}

fn receipt_fields(event: &Event) -> Result<StoredReceipt> {
    let request = read_request(event)?;
    Ok(StoredReceipt {
        v: 1,
        share: request.share,
        item: request.item,
        owner: request.owner,
        claim: request.claim,
        request: event.id.to_hex(),
        revision: request.revision,
    })
}

pub(super) fn verify_receipt(receipt: &Event, request: &Event) -> Result<()> {
    let stored: StoredReceipt = read_meta(receipt, RECEIPT_LABEL)?;
    ensure!(
        stored == receipt_fields(request)?,
        "Saved receipt names a different return"
    );
    Ok(())
}

pub fn revision(cid: &Cid) -> Result<String> {
    let key = cid
        .key
        .context("Shared owner return requires an encrypted file")?;
    let material = format!(
        "iris-drive-file-revision-v1\n{}\n{}",
        hex::encode(cid.hash),
        key.to_hex(),
    );
    Ok(hex::encode(Sha256::digest(material.as_bytes())))
}

pub(super) fn entry_revision(entry: &TreeEntry) -> Result<String> {
    ensure!(
        !entry.link_type.is_directory_like(),
        "Owner return applies to regular files"
    );
    revision(&Cid {
        hash: entry.hash,
        key: entry.key,
    })
}

pub(super) fn signed_entry_owner(
    keys: &Keys,
    share: NostrIdentityId,
    owner: NostrIdentityId,
    entry: &TreeEntry,
    now: u64,
) -> Result<Event> {
    let content = FileOwner {
        v: 1,
        share,
        item: Uuid::new_v4().to_string(),
        owner,
        origin: entry_revision(entry)?,
    };
    sign_meta(keys, OWNER_LABEL, &content, now)
}

pub fn sign_return_request(keys: &Keys, request: &FileReturn, now: u64) -> Result<Event> {
    validate_request(request)?;
    sign_meta(keys, REQUEST_LABEL, request, now)
}

pub fn read_owner(event: &Event) -> Result<FileOwner> {
    let owner: FileOwner = read_meta(event, OWNER_LABEL)?;
    ensure!(
        owner.v == 1 && uuid_v4(&owner.item) && lower_hex(&owner.origin),
        "Invalid shared file owner claim"
    );
    Ok(owner)
}

pub fn read_request(event: &Event) -> Result<FileReturn> {
    let value = read_meta(event, REQUEST_LABEL)?;
    validate_request(&value)?;
    Ok(value)
}

fn validate_request(value: &FileReturn) -> Result<()> {
    ensure!(
        value.v == 1
            && uuid_v4(&value.item)
            && uuid_v4(&value.nonce)
            && lower_hex(&value.revision)
            && lower_hex(&value.claim)
            && !value.path.is_empty()
            && value.path.iter().all(|part| valid_component(part))
            && value
                .seen
                .iter()
                .all(|(author, seq)| lower_hex(author) && (1..=MAX_SAFE_SEQUENCE).contains(seq)),
        "Invalid shared file return request"
    );
    Ok(())
}

pub(super) fn signed_meta_at(entry: &TreeEntry, field: &str) -> Result<Option<Event>> {
    let Some(value) = entry.meta.as_ref().and_then(|meta| meta.get(field)) else {
        return Ok(None);
    };
    let encoded = value
        .as_str()
        .context("Shared ownership metadata must be a JSON string")?;
    ensure!(
        encoded.len() <= MAX_EVENT_BYTES,
        "Shared ownership metadata is too large"
    );
    Ok(Some(Event::from_json(encoded)?))
}

pub fn build_return_note(
    author: &Keys,
    recipient: PublicKey,
    phase: ReturnPhase,
    request: &Event,
    now: u64,
) -> Result<Event> {
    let fields = read_request(request)?;
    let payload = serde_json::to_string(&ReturnNote {
        v: 1,
        phase,
        request: request.clone(),
        receipt: (phase == ReturnPhase::Stored)
            .then(|| sign_meta(author, RECEIPT_LABEL, &receipt_fields(request)?, now))
            .transpose()?,
    })?;
    let d_tag = note_coordinate(&fields, request, phase, &recipient.to_hex());
    let wrapped = nip44::encrypt(author.secret_key(), &recipient, payload, nip44::Version::V2)?;
    ensure!(
        wrapped.len() <= MAX_EVENT_BYTES,
        "Shared return note is too large"
    );
    Ok(EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), wrapped)
        .allow_self_tagging()
        .tags([
            Tag::identifier(d_tag),
            label_tag(NOTE_LABEL),
            Tag::public_key(recipient),
        ])
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(author)?)
}

pub fn read_return_note(event: &Event, recipient: &Keys) -> Result<ReturnNote> {
    ensure!(
        event.kind.as_u16() == KIND_DRIVE_ROOT && event.content.len() <= MAX_EVENT_BYTES,
        "Invalid shared return note"
    );
    event.verify()?;
    let tags = event.tags.iter().map(Tag::as_slice).collect::<Vec<_>>();
    ensure!(
        tags.len() == 3 && tags.iter().all(|tag| tag.len() == 2),
        "Invalid shared return note tags"
    );
    let address = recipient.public_key().to_hex();
    ensure!(
        has_exact_tag(&tags, "l", NOTE_LABEL) && has_exact_tag(&tags, "p", &address),
        "Shared return note has the wrong recipient or label"
    );
    let encoded = nip44::decrypt(recipient.secret_key(), &event.pubkey, &event.content)?;
    let note: ReturnNote = serde_json::from_str(&encoded)?;
    ensure!(note.v == 1, "Unsupported shared return note");
    let fields = read_request(&note.request)?;
    let coordinate = note_coordinate(&fields, &note.request, note.phase, &address);
    ensure!(
        has_exact_tag(&tags, "d", &coordinate),
        "Shared return note has the wrong coordinate"
    );
    match (note.phase, &note.receipt) {
        (ReturnPhase::Request, None) => ensure!(
            event.pubkey == note.request.pubkey,
            "Return request note has the wrong author"
        ),
        (ReturnPhase::Stored, Some(receipt)) => {
            ensure!(
                event.pubkey == receipt.pubkey,
                "Saved receipt has the wrong author"
            );
            verify_receipt(receipt, &note.request)?;
        }
        _ => anyhow::bail!("Invalid receipt for return note phase"),
    }
    Ok(note)
}

#[must_use]
pub fn return_note_filter(device: PublicKey) -> Filter {
    Filter::new()
        .kind(Kind::from(KIND_DRIVE_ROOT))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::L), NOTE_LABEL)
        .pubkey(device)
}

fn note_coordinate(
    request: &FileReturn,
    event: &Event,
    phase: ReturnPhase,
    address: &str,
) -> String {
    device_note_d_tag(
        NOTE_LABEL,
        &[
            &request.share.to_string(),
            &request.item,
            &event.id.to_hex(),
            phase.label(),
        ],
        address,
    )
}

fn sign_meta(keys: &Keys, label: &str, content: &impl Serialize, now: u64) -> Result<Event> {
    Ok(
        EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), serde_json::to_string(content)?)
            .tag(label_tag(label))
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(keys)?,
    )
}

fn read_meta<T: DeserializeOwned>(event: &Event, label: &str) -> Result<T> {
    ensure!(
        event.kind.as_u16() == KIND_DRIVE_ROOT && event.content.len() <= MAX_EVENT_BYTES,
        "Invalid shared ownership event"
    );
    let tags = event.tags.iter().map(Tag::as_slice).collect::<Vec<_>>();
    ensure!(
        tags.len() == 1 && has_exact_tag(&tags, "l", label),
        "Invalid shared ownership event tags"
    );
    event.verify()?;
    Ok(serde_json::from_str(&event.content)?)
}

fn has_exact_tag(tags: &[&[String]], name: &str, value: &str) -> bool {
    tags.iter().filter(|tag| **tag == [name, value]).count() == 1
}

fn label_tag(label: &str) -> Tag {
    Tag::custom(
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
        [label],
    )
}

fn lower_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn uuid_v4(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version() == Some(Version::Random) && id.hyphenated().to_string() == value
    })
}

pub(super) fn valid_component(part: &str) -> bool {
    !part.is_empty()
        && !matches!(part, "." | ".." | crate::merge::META_DIR)
        && !part.contains(['/', '\\', '\0'])
}
