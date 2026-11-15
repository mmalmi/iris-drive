//! How much of the Drive each of the profile's devices holds, how much room
//! it has and which versions of the Drive it has taken in, as the Storage
//! page shows it. Each device measures itself and sends the others an
//! encrypted report; Iris Drive web sends and reads the same report.

pub mod disk;
pub mod event;
pub mod measure;
pub mod report;
pub mod reporter;
pub mod rows;
pub mod state;
pub mod view;

#[cfg(test)]
mod tests;

pub use event::{
    DEVICE_STORAGE_LABEL, build_device_storage_event, device_storage_d_tag, device_storage_filter,
    is_device_storage_event_for, looks_like_device_storage_event, open_device_storage_event,
};
pub use report::{
    DeviceStorageFiles, DeviceStorageFolder, DeviceStorageReport, read_device_storage_report,
    report_moved,
};
pub use reporter::{DeviceStorageNudge, DeviceStorageReporter, seen_versions};
pub use state::{
    DEVICE_STORAGE_FILE, DeviceStorageApplied, DeviceStorageApply, DeviceStorageState,
    apply_device_storage_event, profile_devices, take_in_relay_event,
};

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
