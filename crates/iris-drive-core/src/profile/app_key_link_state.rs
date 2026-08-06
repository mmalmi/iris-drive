use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_INBOUND_APP_KEY_LINK_REQUESTS: usize = 32;
pub const MAX_HANDLED_APP_KEY_LINK_REQUESTS: usize = 128;
pub const MAX_PENDING_DEVICE_APPROVAL_RECEIPTS: usize = 32;
pub const MAX_PERSISTED_DEVICE_APPROVAL_RECEIPTS: usize = 8;

/// Exact approval receipts retained by a joining install until their signed
/// applied-ACKs can be replayed. Old configs stored one JSON string; new
/// configs serialize the same field as a bounded array.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct PersistedDeviceApprovalReceipts(Vec<String>);

impl<'de> Deserialize<'de> for PersistedDeviceApprovalReceipts {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(String),
            Many(Vec<String>),
        }

        let events = match OneOrMany::deserialize(deserializer)? {
            OneOrMany::One(event) => vec![event],
            OneOrMany::Many(events) => events,
        };
        let mut receipts = Self::default();
        for event in events {
            receipts.insert(event);
        }
        Ok(receipts)
    }
}

impl PersistedDeviceApprovalReceipts {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn is_some(&self) -> bool {
        !self.is_empty()
    }

    #[must_use]
    pub fn as_ref(&self) -> Option<&String> {
        self.0.last()
    }

    #[must_use]
    pub fn as_deref(&self) -> Option<&str> {
        self.as_ref().map(String::as_str)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn insert(&mut self, event_json: String) -> bool {
        if self.0.contains(&event_json) {
            return false;
        }
        self.0.push(event_json);
        let overflow = self
            .0
            .len()
            .saturating_sub(MAX_PERSISTED_DEVICE_APPROVAL_RECEIPTS);
        if overflow > 0 {
            self.0.drain(..overflow);
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAppKeyLinkRequest {
    #[serde(alias = "admin_device_pubkey")]
    pub admin_app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_key_secret: String,
    #[serde(
        default,
        skip_serializing_if = "PersistedDeviceApprovalReceipts::is_empty"
    )]
    pub approval_receipt_event: PersistedDeviceApprovalReceipts,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundAppKeyLinkRequest {
    #[serde(alias = "device_pubkey")]
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_url: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandledAppKeyLinkRequest {
    #[serde(alias = "device_pubkey")]
    pub app_key_pubkey: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingDeviceApprovalReceipt {
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    #[serde(
        default,
        alias = "request_relay",
        skip_serializing_if = "String::is_empty"
    )]
    pub relay_url: String,
    pub event_json: String,
}
