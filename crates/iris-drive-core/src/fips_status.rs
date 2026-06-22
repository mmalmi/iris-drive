use std::collections::BTreeSet;

use serde_json::{Value, json};

#[allow(clippy::too_many_lines)]
#[must_use]
pub fn normalize_fips_status_value(
    fips_status: Option<&Value>,
    running: bool,
    fresh: bool,
    error: Value,
    authorized_peer_fallback: &[String],
) -> Value {
    let mut authorized_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    if authorized_peers.is_empty() {
        authorized_peers = authorized_peer_fallback.to_vec();
        authorized_peers.sort();
        authorized_peers.dedup();
    }

    let direct_devices = if fresh {
        fips_direct_devices_from_status(fips_status)
    } else {
        Vec::new()
    };
    let mesh_devices = if fresh {
        fips_mesh_devices_from_status(fips_status)
    } else {
        Vec::new()
    };
    let online_devices = if fresh {
        fips_online_devices_from_status(fips_status)
    } else {
        Vec::new()
    };

    let authorized_set = authorized_peers.iter().cloned().collect::<BTreeSet<_>>();
    let direct_set = direct_devices.iter().cloned().collect::<BTreeSet<_>>();
    let online_set = online_devices.iter().cloned().collect::<BTreeSet<_>>();
    let roster_direct_device_count = direct_set.intersection(&authorized_set).count();
    let roster_online_device_count = online_set.intersection(&authorized_set).count();
    let other_peer_count = online_set.difference(&authorized_set).count();
    let state = fips_state(fips_status.is_some(), running, fresh, &error);
    let state_label = fips_state_label(state);
    let roster_label = fips_roster_label(roster_online_device_count, authorized_peers.len());
    let nostr_discovery_app = fips_status
        .and_then(|status| status.get("nostr_discovery_app"))
        .and_then(Value::as_str)
        .or_else(|| {
            fips_status
                .and_then(|status| status.get("discovery_scope"))
                .and_then(Value::as_str)
        });

    let endpoint_npub = fips_status
        .and_then(|status| status.get("endpoint_npub"))
        .and_then(Value::as_str);
    let discovery_scope = fips_status
        .and_then(|status| status.get("discovery_scope"))
        .and_then(Value::as_str);
    let mut object = serde_json::Map::new();
    object.insert("enabled".to_owned(), json!(fips_status.is_some()));
    object.insert("running".to_owned(), json!(running));
    object.insert("fresh".to_owned(), json!(fresh));
    object.insert(
        "updated_at".to_owned(),
        fips_status
            .and_then(|status| status.get("updated_at"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    object.insert("state".to_owned(), json!(state));
    object.insert("state_label".to_owned(), json!(state_label));
    object.insert("roster_label".to_owned(), json!(roster_label));
    object.insert("endpoint_npub".to_owned(), json!(endpoint_npub));
    object.insert("discovery_scope".to_owned(), json!(discovery_scope));
    object.insert("nostr_discovery_app".to_owned(), json!(nostr_discovery_app));
    object.insert(
        "udp_enabled".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("udp_enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    );
    object.insert(
        "udp_bind_addr".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("udp_bind_addr"))
                .and_then(Value::as_str)
        ),
    );
    object.insert(
        "udp_public".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("udp_public"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    );
    object.insert(
        "udp_external_addr".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("udp_external_addr"))
                .and_then(Value::as_str)
        ),
    );
    object.insert(
        "webrtc_enabled".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("webrtc_enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    );
    object.insert(
        "webrtc_max_connections".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("webrtc_max_connections"))
                .and_then(Value::as_u64)
        ),
    );
    object.insert(
        "open_discovery_max_pending".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("open_discovery_max_pending"))
                .and_then(Value::as_u64)
        ),
    );
    object.insert(
        "mesh_peer_count".to_owned(),
        json!(
            fips_status
                .and_then(|status| status.get("mesh_peer_count"))
                .and_then(Value::as_u64)
                .unwrap_or(mesh_devices.len() as u64)
        ),
    );
    object.insert("mesh_device_count".to_owned(), json!(mesh_devices.len()));
    object.insert(
        "roster_peer_count".to_owned(),
        json!(authorized_peers.len()),
    );
    object.insert(
        "roster_online_device_count".to_owned(),
        json!(roster_online_device_count),
    );
    object.insert(
        "roster_online_peer_count".to_owned(),
        json!(roster_online_device_count),
    );
    object.insert(
        "roster_direct_device_count".to_owned(),
        json!(roster_direct_device_count),
    );
    object.insert(
        "roster_connected_peer_count".to_owned(),
        json!(roster_direct_device_count),
    );
    object.insert(
        "authorized_peer_count".to_owned(),
        json!(authorized_peers.len()),
    );
    object.insert(
        "online_device_count".to_owned(),
        json!(online_devices.len()),
    );
    object.insert("online_peer_count".to_owned(), json!(online_devices.len()));
    object.insert(
        "direct_device_count".to_owned(),
        json!(direct_devices.len()),
    );
    object.insert("direct_peer_count".to_owned(), json!(direct_devices.len()));
    object.insert(
        "connected_peer_count".to_owned(),
        json!(direct_devices.len()),
    );
    object.insert("other_peer_count".to_owned(), json!(other_peer_count));
    object.insert("authorized_peers".to_owned(), json!(authorized_peers));
    object.insert("online_devices".to_owned(), json!(online_devices));
    object.insert("online_peers".to_owned(), json!(online_devices));
    object.insert("direct_devices".to_owned(), json!(direct_devices));
    object.insert("direct_peers".to_owned(), json!(direct_devices));
    object.insert("connected_peers".to_owned(), json!(direct_devices));
    object.insert("mesh_devices".to_owned(), json!(mesh_devices));
    object.insert("mesh_peers".to_owned(), json!(mesh_devices));
    object.insert(
        "peer_statuses".to_owned(),
        normalized_fips_peer_statuses(fips_status.and_then(|status| status.get("peer_statuses"))),
    );
    object.insert(
        "relay_statuses".to_owned(),
        fips_status
            .and_then(|status| status.get("relay_statuses"))
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    object.insert("error".to_owned(), error);
    Value::Object(object)
}

#[must_use]
pub fn fips_state(enabled: bool, running: bool, fresh: bool, error: &Value) -> &'static str {
    if fips_error_is_present(error) {
        return "error";
    }
    if enabled && fresh {
        return "running";
    }
    if enabled || running {
        return "stale";
    }
    "paused"
}

#[must_use]
pub fn fips_state_label(state: &str) -> &'static str {
    match state {
        "error" => "Error",
        "running" => "Running",
        "stale" => "Stale",
        _ => "Paused",
    }
}

#[must_use]
pub fn fips_roster_label(online_count: usize, roster_count: usize) -> String {
    format!("{online_count}/{roster_count} online")
}

#[must_use]
pub fn normalized_fips_peer_statuses(value: Option<&Value>) -> Value {
    Value::Array(
        value
            .and_then(Value::as_array)
            .map(|statuses| {
                statuses
                    .iter()
                    .filter_map(|status| {
                        let object = status.as_object()?;
                        let mut normalized = object.clone();
                        normalized.insert(
                            "connection_label".to_owned(),
                            Value::String(fips_peer_connection_label(status)),
                        );
                        Some(Value::Object(normalized))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

#[must_use]
pub fn fips_peer_connection_label(status: &Value) -> String {
    let transport = status
        .get("transport_type")
        .and_then(Value::as_str)
        .map(str::to_uppercase);
    let srtt_ms = status.get("srtt_ms").and_then(Value::as_u64);
    match (transport, srtt_ms) {
        (Some(transport), Some(srtt_ms)) => format!("{transport}, {srtt_ms} ms"),
        (Some(transport), None) => transport,
        (None, Some(srtt_ms)) => format!("{srtt_ms} ms"),
        _ => "Discovered".to_owned(),
    }
}

#[must_use]
pub fn fips_online_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
    let mut peers =
        string_set_from_json_array(fips_status.and_then(|status| status.get("online_devices")));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("online_peers")),
    ));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("direct_devices")),
    ));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("direct_peers")),
    ));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("connected_peers")),
    ));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("mesh_devices")),
    ));
    peers.extend(string_set_from_json_array(
        fips_status.and_then(|status| status.get("mesh_peers")),
    ));
    if let Some(active_peers) = active_peer_ids_from_statuses(fips_status) {
        peers = peers.intersection(&active_peers).cloned().collect();
    }
    peers.into_iter().collect()
}

#[must_use]
pub fn fips_direct_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
    let mut direct_devices =
        string_set_from_json_array(fips_status.and_then(|status| status.get("direct_devices")));
    if direct_devices.is_empty() {
        direct_devices =
            string_set_from_json_array(fips_status.and_then(|status| status.get("direct_peers")));
    }
    if direct_devices.is_empty() {
        direct_devices = string_set_from_json_array(
            fips_status.and_then(|status| status.get("connected_peers")),
        );
    }
    if let Some(active_peers) = active_peer_ids_from_statuses(fips_status) {
        direct_devices = direct_devices
            .intersection(&active_peers)
            .cloned()
            .collect();
    }
    direct_devices.into_iter().collect()
}

fn active_peer_ids_from_statuses(fips_status: Option<&Value>) -> Option<BTreeSet<String>> {
    let statuses = fips_status
        .and_then(|status| status.get("peer_statuses"))
        .and_then(Value::as_array)?;
    Some(
        statuses
            .iter()
            .filter(|status| fips_peer_has_live_transport(status))
            .filter_map(|status| status.get("npub").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn fips_peer_has_live_transport(status: &Value) -> bool {
    if status
        .get("transport_type")
        .and_then(Value::as_str)
        .is_some_and(|transport| !transport.trim().is_empty())
    {
        return true;
    }
    if status
        .get("transport_addr")
        .and_then(Value::as_str)
        .is_some_and(|transport| !transport.trim().is_empty())
    {
        return true;
    }
    if status.get("srtt_ms").and_then(Value::as_u64).is_some() {
        return true;
    }
    for key in ["bytes_recv", "bytes_sent", "packets_recv", "packets_sent"] {
        if status.get(key).and_then(Value::as_u64).unwrap_or(0) > 0 {
            return true;
        }
    }
    false
}

#[must_use]
pub fn fips_mesh_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
    let mut mesh_devices =
        string_set_from_json_array(fips_status.and_then(|status| status.get("mesh_devices")));
    if mesh_devices.is_empty() {
        mesh_devices =
            string_set_from_json_array(fips_status.and_then(|status| status.get("mesh_peers")));
    }
    if let Some(active_peers) = active_peer_ids_from_statuses(fips_status) {
        mesh_devices = mesh_devices.intersection(&active_peers).cloned().collect();
    }
    mesh_devices.into_iter().collect()
}

#[must_use]
pub fn online_device_ids(direct_devices: &[String], mesh_devices: &[String]) -> Vec<String> {
    direct_devices
        .iter()
        .chain(mesh_devices)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[must_use]
pub fn string_vec_from_json_array(value: Option<&Value>) -> Vec<String> {
    string_set_from_json_array(value).into_iter().collect()
}

#[must_use]
pub fn string_set_from_json_array(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn fips_error_is_present(error: &Value) -> bool {
    match error {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalized_fips_status_merges_device_sets_and_labels_peers() {
        let raw = json!({
            "authorized_peers": ["npub1b", "npub1c"],
            "connected_peers": ["npub1b"],
            "mesh_peers": ["npub1c", "npub1x"],
            "peer_statuses": [{
                "npub": "npub1b",
                "transport_type": "tcp",
                "srtt_ms": 12
            }, {
                "npub": "npub1c",
                "transport_type": "webrtc"
            }, {
                "npub": "npub1x",
                "transport_type": "udp",
                "bytes_recv": 1
            }]
        });

        let normalized = normalize_fips_status_value(Some(&raw), true, true, json!(null), &[]);

        assert_eq!(normalized["state"], "running");
        assert_eq!(normalized["state_label"], "Running");
        assert_eq!(normalized["roster_label"], "2/2 online");
        assert_eq!(normalized["direct_devices"], json!(["npub1b"]));
        assert_eq!(normalized["mesh_devices"], json!(["npub1c", "npub1x"]));
        assert_eq!(
            normalized["online_devices"],
            json!(["npub1b", "npub1c", "npub1x"])
        );
        assert_eq!(normalized["roster_online_device_count"], 2);
        assert_eq!(normalized["other_peer_count"], 1);
        assert_eq!(
            normalized["peer_statuses"][0]["connection_label"],
            "TCP, 12 ms"
        );
    }

    #[test]
    fn status_without_live_transport_does_not_count_peer_online() {
        let raw = json!({
            "authorized_peers": ["npub1phone"],
            "connected_peers": ["npub1phone"],
            "online_devices": ["npub1phone"],
            "peer_statuses": [{
                "npub": "npub1phone",
                "bytes_recv": 0,
                "bytes_sent": 0,
                "packets_recv": 0,
                "packets_sent": 0,
                "srtt_ms": null,
                "transport_addr": null,
                "transport_type": null
            }]
        });

        let normalized = normalize_fips_status_value(Some(&raw), true, true, json!(null), &[]);

        assert_eq!(normalized["direct_devices"], json!([]));
        assert_eq!(normalized["online_devices"], json!([]));
        assert_eq!(normalized["roster_online_device_count"], 0);
        assert_eq!(
            normalized["peer_statuses"][0]["connection_label"],
            "Discovered"
        );
    }

    #[test]
    fn stale_or_error_fips_status_suppresses_online_sets() {
        let raw = json!({
            "connected_peers": ["npub1b"],
            "mesh_peers": ["npub1c"]
        });

        let stale = normalize_fips_status_value(Some(&raw), true, false, json!(null), &[]);
        assert_eq!(stale["state"], "stale");
        assert_eq!(stale["state_label"], "Stale");
        assert_eq!(stale["online_devices"], json!([]));
        assert_eq!(stale["direct_devices"], json!([]));
        assert_eq!(stale["mesh_devices"], json!([]));

        let failed = normalize_fips_status_value(Some(&raw), false, false, json!("boom"), &[]);
        assert_eq!(failed["state"], "error");
        assert_eq!(failed["state_label"], "Error");
        assert_eq!(failed["error"], "boom");
    }
}
