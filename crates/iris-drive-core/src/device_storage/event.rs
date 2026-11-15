//! The event a device sends its report in: one replaceable event per device,
//! encrypted for the profile's devices (`crate::sealed_content`). Its only
//! tag is a `d` tag hashed from what it is and whose profile, so relays can
//! tell neither. Iris Drive web reads and writes the same event.

use nostr_sdk::{Event, EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};

use super::report::{DeviceStorageReport, read_device_storage_report};
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::sealed_content::{self, SealedContentError};

pub const DEVICE_STORAGE_LABEL: &str = "iris-drive/device-storage";
const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// The same for every device of the profile, and telling nothing of it.
#[must_use]
pub fn device_storage_d_tag(profile_id: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{DEVICE_STORAGE_LABEL}\n{profile_id}").as_bytes(),
    ))
}

/// Every device's report for the profile; whose they are is checked on reading.
#[must_use]
pub fn device_storage_filter(profile_id: &str) -> Filter {
    Filter::new()
        .kind(Kind::from(KIND_DRIVE_ROOT))
        .identifier(device_storage_d_tag(profile_id))
}

#[must_use]
pub fn is_device_storage_event_for(event: &Event, profile_id: &str) -> bool {
    event.kind.as_u16() == KIND_DRIVE_ROOT
        && event.tags.identifier() == Some(device_storage_d_tag(profile_id).as_str())
}

/// Whether it looks like a report of any profile: a lone 64-hex `d` tag.
/// Drive roots and shared folder lists always carry more tags.
#[must_use]
pub fn looks_like_device_storage_event(event: &Event) -> bool {
    event.kind.as_u16() == KIND_DRIVE_ROOT
        && event.tags.len() == 1
        && event.tags.identifier().is_some_and(|d_tag| {
            d_tag.len() == 64 && d_tag.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

pub fn build_device_storage_event(
    keys: &Keys,
    profile_id: &str,
    report: &DeviceStorageReport,
    recipients: &[String],
    created_at: u64,
) -> Result<Event, SealedContentError> {
    let plaintext = serde_json::to_vec(report)
        .map_err(|error| SealedContentError::Invalid(error.to_string()))?;
    let content = sealed_content::seal(keys, recipients, &plaintext)?;
    EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content)
        .tags([Tag::identifier(device_storage_d_tag(profile_id))])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(|error| SealedContentError::Invalid(error.to_string()))
}

/// The report in a signed event of the profile, opened with this device's
/// key. Whether its author is one of the profile's devices is the caller's
/// to check.
pub fn open_device_storage_event(
    event: &Event,
    profile_id: &str,
    keys: &Keys,
) -> Result<DeviceStorageReport, SealedContentError> {
    let invalid = |reason: &str| SealedContentError::Invalid(reason.to_owned());
    if !is_device_storage_event_for(event, profile_id) {
        return Err(invalid("not a device storage report of this profile"));
    }
    if event.content.len() > MAX_CONTENT_BYTES {
        return Err(invalid("too large"));
    }
    event
        .verify()
        .map_err(|error| SealedContentError::Invalid(error.to_string()))?;
    let plaintext = sealed_content::open(&event.content, &event.pubkey, keys)?;
    let value = serde_json::from_slice(&plaintext)
        .map_err(|error| SealedContentError::Invalid(error.to_string()))?;
    read_device_storage_report(&value).ok_or_else(|| invalid("not a device storage report"))
}
