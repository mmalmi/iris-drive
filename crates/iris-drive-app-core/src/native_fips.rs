use std::collections::BTreeSet;

pub(crate) fn online_device_ids(direct_devices: &[String], mesh_devices: &[String]) -> Vec<String> {
    direct_devices
        .iter()
        .chain(mesh_devices)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
