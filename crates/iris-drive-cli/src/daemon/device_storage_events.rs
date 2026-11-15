/// Another device's report of how much of the Drive it holds. A device this
/// one has not told its own report yet, as one linked since, is told now.
fn apply_device_storage_relay_event(
    config_dir: &Path,
    event: &nostr_sdk::Event,
) -> EventApplyOutcome {
    use iris_drive_core::device_storage::{self, DeviceStorageApply};
    let applied = match device_storage::take_in_relay_event(config_dir, event) {
        Ok(applied) => applied,
        Err(error) => {
            emit_daemon_status_event(
                config_dir,
                json!({"event": "device_storage_error", "id": event.id.to_hex(), "error": format!("{error:#}")}),
            );
            return EventApplyOutcome::Unchanged;
        }
    };
    if applied.outcome != DeviceStorageApply::Applied {
        return EventApplyOutcome::Unchanged;
    }
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "device_storage",
            "outcome": "applied",
            "author": pubkey_npub(&event.pubkey.to_hex()),
            "created_at": event.created_at.as_secs(),
        }),
    );
    EventApplyOutcome::Changed
}
