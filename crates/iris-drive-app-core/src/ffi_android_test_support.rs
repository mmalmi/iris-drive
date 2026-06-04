use std::path::Path;

use anyhow::Context;
use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;
use nostr_sdk::JsonUtil;
use serde_json::{Value, json};

pub(crate) fn native_apply_owner_snapshot_for_test_json(
    owner_data_dir: &str,
    linked_data_dir: &str,
) -> Value {
    match native_apply_owner_snapshot_for_test(owner_data_dir, linked_data_dir) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

fn native_apply_owner_snapshot_for_test(
    owner_data_dir: &str,
    linked_data_dir: &str,
) -> anyhow::Result<Value> {
    let owner_dir = Path::new(owner_data_dir);
    let linked_dir = Path::new(linked_data_dir);

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir))?;
    let app_keys_event = nostr_sdk::Event::from_json(
        &owner_config
            .account
            .as_ref()
            .context("owner account missing")?
            .app_keys_event
            .as_ref()
            .context("owner app keys event missing")?
            .event_json,
    )
    .context("parsing owner app keys event")?;
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir))?;
    iris_drive_core::relay_sync::apply_remote_app_keys_event(&mut linked_config, &app_keys_event)
        .context("applying owner app keys event")?;
    linked_config
        .save(config_path_in(linked_dir))
        .context("saving linked config after app keys")?;

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir))?;
    let owner_account_state = owner_config
        .account
        .as_ref()
        .context("owner account missing")?;
    let owner = iris_drive_core::Account::load(owner_account_state.clone(), owner_dir)
        .context("loading owner account keys")?;
    let drive = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .context("owner primary drive missing")?;
    let root = drive
        .device_roots
        .get(&owner_account_state.device_pubkey)
        .context("owner device root missing")?;
    let authorized = owner_account_state
        .app_keys
        .as_ref()
        .context("owner app keys missing")?
        .devices
        .iter()
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    let drive_root_event = iris_drive_core::nostr_events::build_drive_root_event(
        owner.device.keys(),
        &owner_account_state.root_scope_id(),
        iris_drive_core::PRIMARY_DRIVE_ID,
        root,
        &authorized,
    )
    .context("building owner drive-root event")?;

    copy_blocks_for_test(owner_dir, linked_dir).context("copying owner blocks to linked device")?;
    let drive_roots = apply_drive_root_events_for_test(linked_dir, &[drive_root_event])
        .context("applying owner drive-root event")?;
    Ok(json!({
        "error": "",
        "drive_root_events_seen": drive_roots.seen,
        "drive_root_events_applied": drive_roots.applied,
        "drive_root_events_skipped": drive_roots.skipped,
    }))
}

fn apply_drive_root_events_for_test(
    config_dir: &Path,
    events: &[nostr_sdk::Event],
) -> anyhow::Result<iris_drive_core::DriveRootEventApplyReport> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let report = iris_drive_core::apply_drive_root_events(config_dir, &mut config, events)?;
    config.save(config_path_in(config_dir))?;
    Ok(report)
}

fn copy_blocks_for_test(from: &Path, to: &Path) -> anyhow::Result<()> {
    fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
        for entry in
            std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?
        {
            let entry = entry?;
            let from_path = entry.path();
            let to_path = to.join(entry.file_name());
            if from_path.is_dir() {
                copy_dir(&from_path, &to_path)?;
            } else {
                std::fs::copy(&from_path, &to_path).with_context(|| {
                    format!("copying {} to {}", from_path.display(), to_path.display())
                })?;
            }
        }
        Ok(())
    }

    copy_dir(&from.join("blocks"), &to.join("blocks"))
}
