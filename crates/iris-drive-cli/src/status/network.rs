use super::*;

pub(crate) fn fips_network_diagnostics(config: &AppConfig, daemon_status: Option<&Value>) -> Value {
    let running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh = daemon_status
        .and_then(|status| status.get("fresh"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let mut authorized_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    if authorized_peers.is_empty() {
        authorized_peers = configured_fips_authorized_peer_npubs(config);
    }
    let direct_devices = fips_direct_devices_from_status(fips_status);
    let mesh_devices = fips_mesh_devices_from_status(fips_status);
    let online_devices = fips_online_devices_from_status(fips_status);
    let authorized_set = authorized_peers.iter().cloned().collect::<BTreeSet<_>>();
    let direct_set = direct_devices.iter().cloned().collect::<BTreeSet<_>>();
    let online_set = online_devices.iter().cloned().collect::<BTreeSet<_>>();
    let roster_direct_device_count = direct_set.intersection(&authorized_set).count();
    let roster_online_device_count = online_set.intersection(&authorized_set).count();
    let other_peer_count = online_set.difference(&authorized_set).count();
    let error = daemon_status
        .and_then(|status| status.get("fips_block_sync_error"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let nostr_discovery_app = fips_status
        .and_then(|status| status.get("nostr_discovery_app"))
        .and_then(Value::as_str)
        .or_else(|| {
            fips_status
                .and_then(|status| status.get("discovery_scope"))
                .and_then(Value::as_str)
        });

    json!({
        "enabled": fips_status.is_some(),
        "running": running,
        "fresh": fresh,
        "endpoint_npub": fips_status
            .and_then(|status| status.get("endpoint_npub"))
            .and_then(Value::as_str),
        "discovery_scope": fips_status
            .and_then(|status| status.get("discovery_scope"))
            .and_then(Value::as_str),
        "nostr_discovery_app": nostr_discovery_app,
        "udp_enabled": fips_status
            .and_then(|status| status.get("udp_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "udp_bind_addr": fips_status
            .and_then(|status| status.get("udp_bind_addr"))
            .and_then(Value::as_str),
        "udp_public": fips_status
            .and_then(|status| status.get("udp_public"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "udp_external_addr": fips_status
            .and_then(|status| status.get("udp_external_addr"))
            .and_then(Value::as_str),
        "webrtc_enabled": fips_status
            .and_then(|status| status.get("webrtc_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "webrtc_max_connections": fips_status
            .and_then(|status| status.get("webrtc_max_connections"))
            .and_then(Value::as_u64),
        "open_discovery_max_pending": fips_status
            .and_then(|status| status.get("open_discovery_max_pending"))
            .and_then(Value::as_u64),
        "mesh_peer_count": fips_status
            .and_then(|status| status.get("mesh_peer_count"))
            .and_then(Value::as_u64)
            .unwrap_or(mesh_devices.len() as u64),
        "mesh_device_count": mesh_devices.len(),
        "roster_peer_count": authorized_peers.len(),
        "roster_online_device_count": roster_online_device_count,
        "roster_online_peer_count": roster_online_device_count,
        "roster_direct_device_count": roster_direct_device_count,
        "roster_connected_peer_count": roster_direct_device_count,
        "authorized_peer_count": authorized_peers.len(),
        "online_device_count": online_devices.len(),
        "online_peer_count": online_devices.len(),
        "direct_device_count": direct_devices.len(),
        "direct_peer_count": direct_devices.len(),
        "connected_peer_count": direct_devices.len(),
        "other_peer_count": other_peer_count,
        "authorized_peers": authorized_peers,
        "online_devices": online_devices,
        "online_peers": online_devices,
        "direct_devices": direct_devices,
        "direct_peers": direct_devices,
        "connected_peers": direct_devices,
        "mesh_devices": mesh_devices,
        "mesh_peers": mesh_devices,
        "peer_statuses": fips_status
            .and_then(|status| status.get("peer_statuses"))
            .cloned()
            .unwrap_or_else(|| json!([])),
        "relay_statuses": fips_status
            .and_then(|status| status.get("relay_statuses"))
            .cloned()
            .unwrap_or_else(|| json!([])),
        "error": error,
    })
}

fn configured_fips_authorized_peer_npubs(config: &AppConfig) -> Vec<String> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };

    snapshot
        .devices
        .iter()
        .filter(|device| device.pubkey != account.device_pubkey)
        .map(|device| account_npub(&device.pubkey))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn string_vec_from_json_array(value: Option<&Value>) -> Vec<String> {
    string_set_from_json_array(value).into_iter().collect()
}

pub(crate) fn fips_online_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
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
    peers.into_iter().collect()
}

pub(crate) fn fips_direct_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
    let direct_devices =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("direct_devices")));
    if !direct_devices.is_empty() {
        return direct_devices;
    }
    let direct_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("direct_peers")));
    if direct_peers.is_empty() {
        string_vec_from_json_array(fips_status.and_then(|status| status.get("connected_peers")))
    } else {
        direct_peers
    }
}

pub(crate) fn fips_mesh_devices_from_status(fips_status: Option<&Value>) -> Vec<String> {
    let mesh_devices =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("mesh_devices")));
    if mesh_devices.is_empty() {
        string_vec_from_json_array(fips_status.and_then(|status| status.get("mesh_peers")))
    } else {
        mesh_devices
    }
}

pub(crate) fn string_set_from_json_array(value: Option<&Value>) -> BTreeSet<String> {
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
