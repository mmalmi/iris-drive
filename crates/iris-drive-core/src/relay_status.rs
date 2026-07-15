use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::relay_config::normalize_relay_url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayStatusSummary {
    pub url: String,
    pub status: String,
    pub status_label: String,
    pub health: String,
}

#[must_use]
pub fn normalized_relay_statuses_for_relays(
    relays: &[String],
    daemon_status: Option<&serde_json::Value>,
) -> Vec<RelayStatusSummary> {
    let mut by_url = BTreeMap::new();
    if let Some(statuses) = daemon_status
        .and_then(|status| status.get("relay_statuses"))
        .and_then(serde_json::Value::as_array)
    {
        for status in statuses {
            let Some(url) = status.get("url").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Ok(url) = normalize_relay_url(url) else {
                continue;
            };
            let value = status
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            by_url.insert(url, value.to_owned());
        }
    }

    relays
        .iter()
        .filter_map(|relay| {
            let Ok(url) = normalize_relay_url(relay) else {
                return None;
            };
            let status = by_url.get(&url).map_or("configured", String::as_str);
            Some(RelayStatusSummary {
                url,
                status: status.to_owned(),
                status_label: relay_status_label(status),
                health: relay_status_health(status).to_owned(),
            })
        })
        .collect()
}

#[must_use]
pub fn relay_status_label(status: &str) -> String {
    if status == "configured" {
        "saved".to_owned()
    } else {
        status.to_owned()
    }
}

#[must_use]
pub fn relay_status_health(status: &str) -> &'static str {
    match status {
        "connected" => "online",
        "connecting" => "connecting",
        "blocked" | "offline" | "terminated" => "error",
        "configured" => "configured",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_statuses_have_rust_owned_labels_and_health() {
        assert_eq!(relay_status_label("configured"), "saved");
        assert_eq!(relay_status_label("connected"), "connected");
        assert_eq!(relay_status_health("connected"), "online");
        assert_eq!(relay_status_health("connecting"), "connecting");
        assert_eq!(relay_status_health("blocked"), "error");
        assert_eq!(relay_status_health("offline"), "error");
        assert_eq!(relay_status_health("terminated"), "error");
        assert_eq!(relay_status_health("configured"), "configured");
        assert_eq!(relay_status_health("mystery"), "unknown");
    }

    #[test]
    fn relay_status_merging_keeps_configured_relays_and_runtime_state() {
        let relays = vec![
            "wss://relay.example/".to_owned(),
            "wss://relay.two".to_owned(),
        ];
        let daemon_status = serde_json::json!({
            "relay_statuses": [
                {"url": "wss://relay.example", "status": "connected"},
                {"url": "wss://unconfigured.example", "status": "connected"}
            ]
        });

        let statuses = normalized_relay_statuses_for_relays(&relays, Some(&daemon_status));

        assert_eq!(
            statuses,
            vec![
                RelayStatusSummary {
                    url: "wss://relay.example".to_owned(),
                    status: "connected".to_owned(),
                    status_label: "connected".to_owned(),
                    health: "online".to_owned(),
                },
                RelayStatusSummary {
                    url: "wss://relay.two".to_owned(),
                    status: "configured".to_owned(),
                    status_label: "saved".to_owned(),
                    health: "configured".to_owned(),
                },
            ]
        );
    }
}
