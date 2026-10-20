use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_KEY_LINK_AUDIT_MAX_EVENTS: usize = 80;
const APP_KEY_LINK_AUDIT_FILE: &str = "native-app-key-link-audit.json";
static APP_KEY_LINK_AUDIT_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppKeyLinkAuditEvent {
    pub(super) timestamp_ms: u64,
    pub(super) phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) authorization_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) receipt_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ack_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error_class: Option<String>,
}

impl AppKeyLinkAuditEvent {
    pub(super) fn new(phase: &str) -> Self {
        Self {
            timestamp_ms: unix_now_millis(),
            phase: phase.to_owned(),
            ..Self::default()
        }
    }
}

pub(super) fn app_key_link_audit_path(config_dir: &Path) -> PathBuf {
    config_dir.join(APP_KEY_LINK_AUDIT_FILE)
}

pub(super) fn append_app_key_link_audit(
    config_dir: &Path,
    event: AppKeyLinkAuditEvent,
) -> Result<(), String> {
    // Relay redundancy is published from a spawned task while the main
    // exchange continues. Serialize the read/replace transaction so those
    // two product timelines cannot overwrite each other's evidence.
    let _write = APP_KEY_LINK_AUDIT_WRITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = app_key_link_audit_path(config_dir);
    let mut events = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<AppKeyLinkAuditEvent>>(&bytes).ok())
        .unwrap_or_default();
    if events.len() >= APP_KEY_LINK_AUDIT_MAX_EVENTS {
        events.drain(..=events.len() - APP_KEY_LINK_AUDIT_MAX_EVENTS);
    }
    events.push(event);
    std::fs::create_dir_all(config_dir)
        .map_err(|error| format!("creating app-key-link audit directory: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&events)
            .map_err(|error| format!("encoding app-key-link audit: {error}"))?,
    )
    .map_err(|error| format!("writing app-key-link audit: {error}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("committing app-key-link audit: {error}"))
}

fn unix_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_is_bounded_and_contains_only_structured_safe_fields() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..=APP_KEY_LINK_AUDIT_MAX_EVENTS {
            let mut event = AppKeyLinkAuditEvent::new("ack_publish");
            event.event_id = Some(format!("event-{index}"));
            event.error_class = Some("relay_timeout".to_owned());
            append_app_key_link_audit(dir.path(), event).unwrap();
        }
        let data = std::fs::read(app_key_link_audit_path(dir.path())).unwrap();
        let events: Vec<AppKeyLinkAuditEvent> = serde_json::from_slice(&data).unwrap();
        assert_eq!(events.len(), APP_KEY_LINK_AUDIT_MAX_EVENTS);
        assert_eq!(events.first().unwrap().event_id.as_deref(), Some("event-1"));
        let encoded = String::from_utf8(data).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("content"));
        assert!(!encoded.contains("pubkey"));
    }
}
