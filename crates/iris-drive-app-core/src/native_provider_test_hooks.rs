use std::path::{Path, PathBuf};
use std::sync::{Mutex, atomic::AtomicU64, atomic::Ordering};

use iris_drive_core::paths::key_path_in;
use serde_json::json;

static PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST: Mutex<Option<PathBuf>> = Mutex::new(None);

static PROVIDER_OPEN_DELAY_MS_FOR_TEST: AtomicU64 = AtomicU64::new(0);

pub(crate) struct ProviderOpenDelayGuard;

impl Drop for ProviderOpenDelayGuard {
    fn drop(&mut self) {
        PROVIDER_OPEN_DELAY_MS_FOR_TEST.store(0, Ordering::SeqCst);
    }
}

pub(crate) fn provider_open_delay_for_test(delay: std::time::Duration) -> ProviderOpenDelayGuard {
    let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
    PROVIDER_OPEN_DELAY_MS_FOR_TEST.store(delay_ms, Ordering::SeqCst);
    ProviderOpenDelayGuard
}

pub(super) fn apply_provider_open_delay_for_test() {
    let delay_ms = PROVIDER_OPEN_DELAY_MS_FOR_TEST.load(Ordering::SeqCst);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
}

pub(crate) struct ProviderPublishLockProbeGuard {
    config_dir: PathBuf,
}

impl Drop for ProviderPublishLockProbeGuard {
    fn drop(&mut self) {
        let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if probe.as_deref() == Some(self.config_dir.as_path()) {
            *probe = None;
        }
    }
}

pub(crate) fn provider_publish_lock_probe_for_test(
    config_dir: &Path,
) -> ProviderPublishLockProbeGuard {
    let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        probe.is_none(),
        "provider publish lock probe already active"
    );
    *probe = Some(config_dir.to_path_buf());
    ProviderPublishLockProbeGuard {
        config_dir: config_dir.to_path_buf(),
    }
}

pub(super) async fn provider_publish_lock_probe_result_for_test(
    config_dir: &Path,
    root: &iris_drive_core::AppKeyRootRef,
    event: Option<&nostr_sdk::Event>,
) -> Option<serde_json::Value> {
    let should_probe = {
        let mut probe = PROVIDER_PUBLISH_LOCK_PROBE_DIR_FOR_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if probe.as_deref() == Some(config_dir) {
            *probe = None;
            true
        } else {
            false
        }
    };
    if !should_probe {
        return None;
    }

    let reacquired = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir),
    )
    .await
    .expect("provider publish began while the config mutation lock was still held")
    .expect("reacquiring provider config mutation lock before network");
    drop(reacquired);

    let event = event.expect("provider publish must prepare its exact Drive-root event under lock");
    let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .expect("loading provider AppKey for publish probe");
    let (_, _, _, event_root) =
        iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, app_key.keys())
            .expect("decrypting prepared provider Drive-root event");
    assert_eq!(event_root.root_cid, root.root_cid);
    assert_eq!(event_root.dck_generation, root.dck_generation);
    assert_eq!(event_root.app_key_seq, root.app_key_seq);
    assert_eq!(event_root.parents, root.parents);
    assert_eq!(event_root.observed, root.observed);
    assert_eq!(event_root.local_only, root.local_only);
    assert!(
        event_root.published_at >= root.published_at,
        "replaceable event timestamp went backwards from the imported root"
    );
    Some(json!({
        "published_drive_root": false,
        "error": "provider publish lock probe skipped network",
        "prepared_root_cid": event_root.root_cid,
    }))
}
