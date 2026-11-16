use super::*;

#[test]
fn every_step_is_worded_by_the_core() {
    let tag = "v0.1.47";
    let cases = [
        (
            UpdateProgress::new(UpdateStage::FindingRelease, ""),
            "Finding the latest release",
        ),
        (
            UpdateProgress::new(UpdateStage::FindingRelease, tag),
            "Finding release v0.1.47",
        ),
        (
            UpdateProgress::downloading(tag, 12_345_678, Some(134_157_973)),
            "Downloading v0.1.47: 12 MB of 134 MB",
        ),
        (
            UpdateProgress::downloading(tag, 2_500_000, None),
            "Downloading v0.1.47: 2.5 MB",
        ),
        (
            UpdateProgress::new(UpdateStage::Verifying, tag),
            "Checking the v0.1.47 download",
        ),
        (
            UpdateProgress::new(UpdateStage::Installing, tag),
            "Installing v0.1.47",
        ),
        (
            UpdateProgress::new(UpdateStage::Restarting, tag),
            "Restarting Iris Drive",
        ),
        (
            UpdateProgress::new(UpdateStage::OpeningInstaller, tag),
            "Opening the v0.1.47 installer",
        ),
        (
            UpdateProgress::new(UpdateStage::InstallerOpened, tag),
            "The v0.1.47 installer is open. Finish installing there.",
        ),
        (
            UpdateProgress::failed(tag, "Administrator install was cancelled.\nmore"),
            "Couldn't install v0.1.47: Administrator install was cancelled",
        ),
        (UpdateProgress::new(UpdateStage::Idle, tag), ""),
    ];
    for (progress, text) in cases {
        assert_eq!(progress.text, text, "{:?}", progress.stage);
    }
}

#[test]
fn shells_show_every_step_after_the_press_until_idle() {
    for stage in [
        UpdateStage::FindingRelease,
        UpdateStage::Verifying,
        UpdateStage::Installing,
        UpdateStage::Restarting,
        UpdateStage::OpeningInstaller,
        UpdateStage::InstallerOpened,
        UpdateStage::Failed,
    ] {
        let progress = UpdateProgress::new(stage, "v1");
        assert!(progress.shown, "{stage:?}");
        assert_eq!(
            progress.fraction, None,
            "only a download has a bar: {stage:?}"
        );
    }
    assert!(UpdateProgress::downloading("v1", 1, Some(2)).shown);
    assert!(!UpdateProgress::default().shown);
}

#[test]
fn only_running_steps_are_active_and_only_failures_offer_retry() {
    for stage in [
        UpdateStage::FindingRelease,
        UpdateStage::Downloading,
        UpdateStage::Verifying,
        UpdateStage::Installing,
        UpdateStage::Restarting,
        UpdateStage::OpeningInstaller,
    ] {
        let progress = UpdateProgress::new(stage, "v1");
        assert!(progress.active && !progress.can_retry, "{stage:?}");
    }
    for stage in [UpdateStage::Idle, UpdateStage::InstallerOpened] {
        let progress = UpdateProgress::new(stage, "v1");
        assert!(!progress.active && !progress.can_retry, "{stage:?}");
    }
    let failed = UpdateProgress::failed("v1", "");
    assert!(!failed.active && failed.can_retry);
    assert_eq!(failed.text, "Couldn't install v1: something went wrong");
}

#[test]
fn download_events_become_a_bounded_fraction() {
    let started = UpdateProgress::from_download_event(
        "v1",
        &DownloadEvent::Started {
            content_length: Some(200),
        },
        None,
    );
    assert_eq!(
        (started.fraction, started.total_bytes),
        (Some(0.0), Some(200))
    );
    let halfway = UpdateProgress::from_download_event(
        "v1",
        &DownloadEvent::Progress {
            chunk_len: 100,
            downloaded: 100,
        },
        Some(200),
    );
    assert_eq!(halfway.fraction, Some(0.5));
    let unknown = UpdateProgress::from_download_event(
        "v1",
        &DownloadEvent::Started {
            content_length: None,
        },
        None,
    );
    assert_eq!(unknown.fraction, None);
    let finished =
        UpdateProgress::from_download_event("v1", &DownloadEvent::Finished { total: 200 }, None);
    assert_eq!(finished.fraction, Some(1.0));
    assert_eq!(
        UpdateProgress::downloading("v1", 500, Some(200)).fraction,
        Some(1.0),
        "a fraction never exceeds one"
    );
}

#[test]
fn shells_report_their_steps_per_profile_and_keep_the_release_tag() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    assert_eq!(update_progress(first.path()).stage, UpdateStage::Idle);
    report_update_progress(first.path(), UpdateProgress::downloading("v2", 1, Some(2)));
    let installing = report_update_stage(first.path(), UpdateStage::Installing, "");
    assert_eq!(installing.text, "Installing v2");
    assert_eq!(update_progress(first.path()), installing);
    assert_eq!(update_progress(second.path()).stage, UpdateStage::Idle);
    let failed = report_update_stage(first.path(), UpdateStage::Failed, "disk full");
    assert_eq!(failed.text, "Couldn't install v2: disk full");
    assert!(failed.can_retry);
    assert_eq!(
        UpdateStage::parse(" opening_installer "),
        Some(UpdateStage::OpeningInstaller)
    );
    assert_eq!(UpdateStage::parse("bogus"), None);
}

#[test]
fn long_reasons_are_bounded() {
    let reason = "x".repeat(MAX_REASON_CHARS + 50);
    let failed = UpdateProgress::failed("v1", &reason);
    assert!(failed.text.ends_with('…'));
    assert!(failed.text.chars().count() < MAX_REASON_CHARS + 40);
}
