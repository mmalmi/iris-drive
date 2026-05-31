use super::*;
use std::io::{Error, ErrorKind};

#[test]
fn retry_interrupted_io_retries_until_success() {
    let mut attempts = 0;

    let value = retry_interrupted_io(|| {
        attempts += 1;
        if attempts < 3 {
            Err(Error::from(ErrorKind::Interrupted))
        } else {
            Ok(42)
        }
    })
    .unwrap();

    assert_eq!(value, 42);
    assert_eq!(attempts, 3);
}

#[test]
fn retry_interrupted_io_returns_non_interrupted_errors() {
    let error = retry_interrupted_io(|| -> std::io::Result<()> {
        Err(Error::from(ErrorKind::PermissionDenied))
    })
    .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn block_stats_entry_limit_marks_truncated() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..3 {
        std::fs::write(dir.path().join(format!("block-{index}")), b"block").unwrap();
    }

    let stats = collect_file_stats_with_entry_limit(dir.path(), Some(2)).unwrap();

    assert!(stats.truncated);
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.total_bytes, 10);
}

#[test]
fn local_gateway_status_includes_nhash_resolver_host_when_enabled() {
    let status = local_gateway_urls_for_root(None, 17_321, true);
    assert_eq!(status["enabled"], true);
    assert_eq!(
        status["nhash_resolver_url"],
        "http://nhash.iris.localhost:17321/"
    );
}

#[test]
fn local_gateway_status_reports_disabled_resolver() {
    let status = local_gateway_urls_for_root(None, 17_321, false);
    assert_eq!(status["enabled"], false);
    assert_eq!(status["host"], "nhash.iris.localhost");
    assert!(status.get("portal_url").is_none());
}

#[test]
fn status_lists_default_blossom_server_as_backup_target() {
    let config = AppConfig::default();
    let targets = backup_targets_status(&config);

    let target = targets
        .iter()
        .find(|target| target["kind"] == "blossom" && target["target"] == "https://upload.iris.to")
        .expect("default Blossom server should be visible in backup targets");

    assert_eq!(target["enabled"], true);
    assert_eq!(target["label"], "Blossom remote");
}
