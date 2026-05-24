#[allow(clippy::wildcard_imports)]
use super::*;

use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_match_seafile_release_operation_sequence() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::from_millis(100)).await;
    cluster.wait_until_authorized().await;

    seafile_add_delete_add_sequence(&cluster).await;
    seafile_create_update_sequence(&cluster).await;
    seafile_rename_sequence(&cluster).await;
    seafile_download_side_sequence(&cluster).await;
}

async fn seafile_add_delete_add_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/add-delete-add/1.txt", b"aaaaaaaa")
        .await;
    cluster
        .remove_all(Client::Windows, "release/add-delete-add")
        .await;
    cluster
        .write(Client::Windows, "release/add-delete-add/1.txt", b"aaaaaaaa")
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "add-delete-add")
        .await;
}

async fn seafile_create_update_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/test/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/test/1.txt", b"222")
        .await;
    cluster
        .write(Client::Windows, "release/copy-source/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/copy-source/2/2.txt", b"222")
        .await;
    cluster
        .write(Client::Windows, "release/test/copied/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/test/copied/2/2.txt", b"222")
        .await;
    cluster.mkdir(Client::Windows, "release/empty").await;
    cluster
        .write(
            Client::Windows,
            "release/empty/test.md",
            b"dddddddddddddddddddddd",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "create-update-deep-copy")
        .await;
}

async fn seafile_rename_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/rename/1.txt", b"111")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/1.txt",
            "release/rename/2.txt",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/3.txt", b"222")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/2.txt",
            "release/rename/3.txt",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/test.txt", b"test")
        .await;
    cluster.mkdir(Client::Windows, "release/rename/test").await;
    cluster
        .write(Client::Windows, "release/rename/4.txt", b"444")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test.txt",
            "release/rename/test/test.txt",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/3.txt",
            "release/rename/test/3.txt",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/4.txt",
            "release/rename/test/4.txt",
        )
        .await;
    cluster.mkdir(Client::Windows, "release/rename/test2").await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test",
            "release/rename/test2/test",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test2/test",
            "release/rename/test",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/test/4.txt", b"444555")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test",
            "release/rename/test2/test",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test2",
            "release/rename/test3",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "rename chain")
        .await;
}

async fn seafile_download_side_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Ubuntu, "download/1.txt", b"11111")
        .await;
    cluster
        .write(Client::Ubuntu, "download/1.txt", b"22222")
        .await;
    cluster.mkdir(Client::Ubuntu, "download/dir1").await;
    cluster
        .rename(Client::Ubuntu, "download/1.txt", "download/2.txt")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir1", "download/dir2")
        .await;
    cluster
        .write(Client::Ubuntu, "download/dir2/1.txt", b"1111111")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir2", "download/dir3")
        .await;
    cluster.remove(Client::Ubuntu, "download/dir3/1.txt").await;
    cluster
        .write(Client::Ubuntu, "download/dir4/2.txt", b"2222222")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir4", "download/dir3/dir4")
        .await;
    cluster.remove(Client::Ubuntu, "download/2.txt").await;
    cluster.remove_all(Client::Ubuntu, "download/dir3").await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "download-side sequence")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_sync_file_type_replacements() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(Client::Windows, "types/file-to-dir", b"old file")
        .await;
    cluster
        .write(Client::Windows, "types/dir-to-file/old.txt", b"old child")
        .await;
    cluster
        .write(
            Client::Windows,
            "types/non-empty-dir/old.txt",
            b"old non-empty child",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "initial file types")
        .await;

    cluster
        .remove_all(Client::Windows, "types/file-to-dir")
        .await;
    cluster
        .write(Client::Windows, "types/file-to-dir/new.txt", b"new child")
        .await;
    cluster
        .remove_all(Client::Windows, "types/dir-to-file")
        .await;
    cluster
        .write(Client::Windows, "types/dir-to-file", b"new file")
        .await;
    cluster
        .remove_all(Client::Windows, "types/non-empty-dir")
        .await;
    cluster
        .write(Client::Windows, "types/non-empty-dir", b"replacement file")
        .await;

    cluster
        .wait_for_convergence_from(Client::Windows, "file type replacements")
        .await;
    cluster.assert_file(Client::Ubuntu, "types/file-to-dir/new.txt", b"new child");
    cluster.assert_file(Client::Ubuntu, "types/dir-to-file", b"new file");
    cluster.assert_file(Client::Ubuntu, "types/non-empty-dir", b"replacement file");
    cluster.assert_missing(Client::Ubuntu, "types/dir-to-file/old.txt");
    cluster.assert_missing(Client::Ubuntu, "types/non-empty-dir/old.txt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_ignore_noise_and_temporary_files() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(Client::Windows, "noise/keep.txt", b"keep")
        .await;
    cluster
        .write(Client::Windows, "noise/.DS_Store", b"finder")
        .await;
    cluster
        .write(Client::Windows, "noise/._keep.txt", b"resource fork")
        .await;
    cluster
        .write(Client::Windows, "noise/Thumbs.db", b"windows thumbs")
        .await;
    cluster
        .write(Client::Windows, "noise/desktop.ini", b"windows desktop")
        .await;
    cluster
        .write(Client::Windows, "noise/draft~", b"editor backup")
        .await;
    cluster
        .write(Client::Windows, "noise/#draft#", b"emacs backup")
        .await;
    cluster
        .write(Client::Windows, "noise/backup.sbak", b"seafile backup")
        .await;
    cluster
        .write(Client::Windows, ".hashtree/prev", b"internal")
        .await;
    cluster
        .write(Client::Windows, ".Trash-1000/files/removed.txt", b"trash")
        .await;
    cluster
        .write(
            Client::Windows,
            "$RECYCLE.BIN/S-1-5-21/removed.txt",
            b"recycle",
        )
        .await;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        cluster.path(Client::Windows).join("noise/keep.txt"),
        cluster.path(Client::Windows).join("noise/link.txt"),
    )
    .unwrap();

    let expected = visible_dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_visible_snapshot(&expected, "ignored noise")
        .await;
    cluster.assert_file(Client::Ubuntu, "noise/keep.txt", b"keep");
    for ignored in [
        "noise/.DS_Store",
        "noise/._keep.txt",
        "noise/Thumbs.db",
        "noise/desktop.ini",
        "noise/draft~",
        "noise/#draft#",
        "noise/backup.sbak",
        ".hashtree/prev",
        ".Trash-1000/files/removed.txt",
        "$RECYCLE.BIN/S-1-5-21/removed.txt",
        "noise/link.txt",
    ] {
        cluster.assert_missing(Client::Ubuntu, ignored);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_reconnect_sender_and_receiver_without_losing_updates() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    for index in 0..16 {
        let path = format!("reconnect/baseline/file-{index:03}.bin");
        cluster
            .write(Client::Windows, &path, &deterministic_bytes(index, 512))
            .await;
    }
    cluster
        .wait_for_convergence_from(Client::Windows, "reconnect baseline")
        .await;

    cluster.stop_daemon(Client::Ubuntu);
    for index in 0..24 {
        let path = format!("reconnect/receiver-stopped/file-{index:03}.bin");
        cluster
            .write(
                Client::Windows,
                &path,
                &deterministic_bytes(100 + index, 768),
            )
            .await;
    }
    cluster.start_daemon(Client::Ubuntu);
    cluster.wait_until_direct_peers_connected().await;
    cluster
        .wait_for_convergence_from(Client::Windows, "receiver restarted")
        .await;

    cluster.stop_daemon(Client::Windows);
    for index in 0..24 {
        let path = format!("reconnect/sender-stopped/file-{index:03}.bin");
        cluster
            .write(
                Client::Ubuntu,
                &path,
                &deterministic_bytes(200 + index, 768),
            )
            .await;
    }
    cluster.start_daemon(Client::Windows);
    cluster.wait_until_direct_peers_connected().await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "sender restarted")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_preserve_virtual_delete_across_source_restart() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    cluster
        .write(
            Client::Ubuntu,
            "stopped-source-delete/from-ubuntu.txt",
            b"delete me after the source daemon stops",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "stopped source delete baseline")
        .await;

    cluster
        .remove(Client::Ubuntu, "stopped-source-delete/from-ubuntu.txt")
        .await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "stopped source delete")
        .await;
    cluster.stop_daemon(Client::Ubuntu);
    cluster.start_daemon(Client::Ubuntu);
    cluster.wait_until_direct_peers_connected().await;
    cluster
        .wait_for_convergence_from(Client::Windows, "delete after source restart")
        .await;
    cluster.assert_missing(Client::Windows, "stopped-source-delete/from-ubuntu.txt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "deterministic concurrent-root coverage lives in core; live transport partitioning still needs a virtual provider hook"]
async fn live_daemons_preserve_both_same_path_concurrent_edits() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    cluster
        .write(
            Client::Windows,
            "conflicts/concurrent.txt",
            b"baseline before concurrent edit",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "concurrent edit baseline")
        .await;

    cluster.stop_daemon(Client::Windows);
    cluster.stop_daemon(Client::Ubuntu);
    cluster
        .write_local_only(
            Client::Windows,
            "conflicts/concurrent.txt",
            b"windows concurrent edit",
        )
        .await;
    cluster
        .write_local_only(
            Client::Ubuntu,
            "conflicts/concurrent.txt",
            b"ubuntu concurrent edit",
        )
        .await;
    cluster.import_source_dir(Client::Windows);
    cluster.import_source_dir(Client::Ubuntu);

    cluster.start_daemon(Client::Windows);
    cluster.start_daemon(Client::Ubuntu);
    cluster.wait_until_direct_peers_connected().await;

    let expected_hashes = [
        to_hex(&sha256(b"windows concurrent edit")),
        to_hex(&sha256(b"ubuntu concurrent edit")),
    ];
    cluster
        .wait_until("concurrent edit conflict copies", || {
            snapshot_has_hashes_with_prefix(
                &dir_snapshot(cluster.path(Client::Windows)),
                "conflicts/concurrent",
                &expected_hashes,
            ) && snapshot_has_hashes_with_prefix(
                &dir_snapshot(cluster.path(Client::Ubuntu)),
                "conflicts/concurrent",
                &expected_hashes,
            )
        })
        .await;

    let expected = dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_snapshot(&expected, "concurrent edit convergence")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "transfer bench; run with `cargo test -p idrive --test daemon_sync_matrix -- --ignored --nocapture`"]
async fn bench_live_daemon_transfer_many_files() {
    let _guard = live_daemon_test_guard().await;
    let files = env_usize("IRIS_DRIVE_SYNC_BENCH_FILES", 1_000);
    let bytes_per_file = env_usize("IRIS_DRIVE_SYNC_BENCH_FILE_BYTES", 4096);
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    let start = Instant::now();
    for index in 0..files {
        let path = format!("many/{:04}/file-{:06}.bin", index / 100, index);
        let bytes = deterministic_bytes(index, bytes_per_file);
        cluster.write(Client::Windows, &path, &bytes).await;
    }
    let expected = dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_snapshot(&expected, "many-file transfer")
        .await;
    let elapsed = start.elapsed();
    let total_bytes = expected.values().map(FileSnapshot::len).sum::<u64>();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_transfer_many_files",
            "files": files,
            "bytes": total_bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(total_bytes, elapsed),
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "large-file transfer bench; set IRIS_DRIVE_SYNC_BENCH_LARGE_BYTES to size"]
async fn bench_live_daemon_transfer_large_file() {
    let _guard = live_daemon_test_guard().await;
    let bytes = env_usize("IRIS_DRIVE_SYNC_BENCH_LARGE_BYTES", 64 * 1024 * 1024);
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    let payload = deterministic_bytes(7, bytes);
    let start = Instant::now();
    cluster
        .write(Client::Windows, "large/one-file.bin", &payload)
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "large-file transfer")
        .await;
    let elapsed = start.elapsed();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_transfer_large_file",
            "bytes": bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(u64::try_from(bytes).unwrap_or(u64::MAX), elapsed),
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "same-files convergence bench; set IRIS_DRIVE_SYNC_BENCH_FILES and IRIS_DRIVE_SYNC_BENCH_FILE_BYTES"]
async fn bench_live_daemon_same_files_already_present() {
    let _guard = live_daemon_test_guard().await;
    let files = env_usize("IRIS_DRIVE_SYNC_BENCH_FILES", 1_000);
    let bytes_per_file = env_usize("IRIS_DRIVE_SYNC_BENCH_FILE_BYTES", 4096);
    let mut seed_files = Vec::new();
    let mut total_bytes = 0u64;
    for index in 0..files {
        let path = format!("same/{:04}/file-{:06}.bin", index / 100, index);
        let bytes = deterministic_bytes(index, bytes_per_file);
        total_bytes += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        seed_files.push(SeedFile::new(Client::Windows, &path, &bytes));
        seed_files.push(SeedFile::new(Client::Ubuntu, &path, &bytes));
    }

    let start = Instant::now();
    let cluster = SyncCluster::start_with_options(SyncClusterOptions {
        blossom_upload_delay: Duration::ZERO,
        seed_files,
    })
    .await;
    cluster.wait_until_authorized().await;
    cluster
        .wait_for_convergence_from(Client::Windows, "same-files already present")
        .await;
    let elapsed = start.elapsed();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_same_files_already_present",
            "files": files,
            "bytes": total_bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(total_bytes, elapsed),
        })
    );
}
