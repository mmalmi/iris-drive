use super::*;

pub(crate) fn peer_statuses(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Vec<serde_json::Value> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let primary_drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID);

    let daemon_running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let connected_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("connected_peers")));
    let mesh_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("mesh_peers")));
    let authorized_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    let block_sync_by_root = daemon_status
        .and_then(|status| status.get("block_sync_by_root"))
        .filter(|value| value.is_object());

    snapshot
        .devices
        .iter()
        .map(|device| {
            let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey));
            let root_cid = root.map(|root| root.root_cid.clone());
            let root_private = root_cid.as_deref().and_then(root_is_private);
            let root_available = root_cid
                .as_deref()
                .map(|root| root_file_count(config_dir, root).is_some());
            let device_npub = account_npub(&device.pubkey);
            let is_current_device = device.pubkey == account.device_pubkey;
            let fips_direct_online = connected_fips.contains(&device_npub);
            let fips_mesh_online = mesh_fips.contains(&device_npub);
            let fips_online = if is_current_device {
                daemon_running
            } else {
                fips_direct_online || fips_mesh_online
            };
            let fips_online_via = if is_current_device && fips_online {
                Some("local")
            } else if fips_direct_online {
                Some("direct")
            } else if fips_mesh_online {
                Some("mesh")
            } else {
                None
            };
            let sync_state = device_sync_state(is_current_device, root.is_some(), root_available);
            let last_block_sync = root_cid
                .as_ref()
                .and_then(|root| block_sync_by_root.and_then(|map| map.get(root)).cloned());
            json!({
                "device_pubkey": device.pubkey,
                "device_npub": device_npub,
                "label": device.label,
                "role": device_role_label(device.role),
                "authorized": true,
                "is_current_device": is_current_device,
                "added_at": device.added_at,
                "fips_authorized": authorized_fips.contains(&device_npub),
                "fips_online": fips_online,
                "fips_direct_online": fips_direct_online,
                "fips_mesh_online": fips_mesh_online,
                "fips_online_via": fips_online_via,
                "has_root": root.is_some(),
                "root_cid": root_cid,
                "root_private": root_private,
                "root_available": root_available,
                "sync_state": sync_state,
                "last_block_sync": last_block_sync,
                "published_at": root.map(|root| root.published_at),
                "dck_generation": root.map(|root| root.dck_generation),
                "device_seq": root.map(|root| root.device_seq),
            })
        })
        .collect()
}
