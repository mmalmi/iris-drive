use std::process::Stdio;
use std::time::Duration;

use iris_drive_core::{
    AppConfig, AppKeyRootRef, Daemon, Drive, Profile, config_lock::ConfigMutationLock,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::timeout;

use super::{Cid, PRIMARY_DRIVE_ID, config_path_in, run_json, tempdir};

#[tokio::test]
async fn missing_peer_root_yields_writer_lock_and_preserves_concurrent_edits() {
    assert_pending_write_recovered(false).await;
}

#[tokio::test]
async fn staged_provider_retry_keeps_the_newest_local_write_and_peer_head() {
    assert_pending_write_recovered(true).await;
}

async fn assert_pending_write_recovered(staged: bool) {
    let config_dir = tempdir().unwrap();
    let dir = config_dir.path();
    let mut profile = Profile::create(dir, Some("writer".into())).unwrap();
    let peer = iris_drive_core::identity::Identity::generate(dir.join("peer.key")).pubkey_hex();
    profile.approve_app_key(&peer, Some("peer".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    config.save(config_path_in(dir)).unwrap();
    std::fs::write(
        iris_drive_core::daemon_liveness::daemon_lock_path(dir),
        std::process::id().to_string(),
    )
    .unwrap();
    let files = tempdir().unwrap();
    std::fs::write(files.path().join("existing.txt"), b"original").unwrap();
    run_json(dir, &["import", files.path().to_str().unwrap()]);

    let daemon = Daemon::open(dir).unwrap();
    let base = daemon
        .provider_projection()
        .await
        .unwrap()
        .1
        .root_cid
        .to_string();
    let (file, size) = daemon.tree().put(b"concurrent").await.unwrap();
    let empty = daemon.tree().put_directory(Vec::new()).await.unwrap();
    let peer_root = daemon
        .tree()
        .set_entry(
            &empty,
            &[],
            "peer.txt",
            &file,
            size,
            hashtree_core::LinkType::Blob,
        )
        .await
        .unwrap();
    let set_peer = |root: &Cid, sequence| {
        let mut config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
        let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
        drive.app_key_roots.insert(
            peer.clone(),
            AppKeyRootRef::legacy(root.to_string(), sequence, 1),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(dir)).unwrap();
    };
    set_peer(&Cid::public([42; 32]), 1);
    let source = files.path().join("incoming.txt");
    std::fs::write(&source, b"new").unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_idrive"));
    command
        .env("IRIS_DRIVE_CONFIG_DIR", dir)
        .env("RUST_LOG", "warn")
        .args(["provider", "write"]);
    if staged {
        command.args(["--base-root-cid", &base]);
    }
    let mut writer = command
        .arg("incoming.txt")
        .arg(&source)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut errors = BufReader::new(writer.stderr.take().unwrap()).lines();
    timeout(Duration::from_secs(10), async {
        while let Some(line) = errors.next_line().await.unwrap() {
            if line.contains("Missing chunk") {
                return;
            }
        }
        panic!("provider never observed the missing peer root");
    })
    .await
    .unwrap();

    let sync_lock = timeout(Duration::from_secs(2), ConfigMutationLock::acquire(dir))
        .await
        .expect("provider held the sync lock while waiting for the missing root")
        .unwrap();
    let stage_path = iris_drive_core::paths::provider_root_staging_path_in(dir);
    assert_eq!(stage_path.exists(), staged);
    if staged {
        // Another authorized writer updates the staged file while the first
        // publisher waits. Replaying the first write would overwrite these bytes.
        let mut stage: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&stage_path).unwrap()).unwrap();
        let current = Cid::parse(stage["root_cid"].as_str().unwrap()).unwrap();
        let (hash, size) = daemon.tree().put(b"newer local edit").await.unwrap();
        let updated = daemon
            .tree()
            .set_entry(
                &current,
                &[],
                "incoming.txt",
                &hash,
                size,
                hashtree_core::LinkType::Blob,
            )
            .await
            .unwrap();
        stage["root_cid"] = serde_json::json!(updated.to_string());
        iris_drive_core::atomic_write(&stage_path, &serde_json::to_vec(&stage).unwrap()).unwrap();
    }
    set_peer(&peer_root, 2);
    drop(sync_lock);

    assert!(
        timeout(Duration::from_secs(8), writer.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(!stage_path.exists());
    for (name, expected) in [
        ("existing.txt", "original"),
        ("peer.txt", "concurrent"),
        (
            "incoming.txt",
            if staged { "newer local edit" } else { "new" },
        ),
    ] {
        let output = files.path().join("readback");
        run_json(dir, &["provider", "read", name, output.to_str().unwrap()]);
        assert_eq!(std::fs::read(&output).unwrap(), expected.as_bytes());
    }
    let config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
    assert_eq!(
        config.drive(PRIMARY_DRIVE_ID).unwrap().app_key_roots[&peer].root_cid,
        peer_root.to_string()
    );
}
