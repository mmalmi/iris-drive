#[allow(clippy::wildcard_imports)]
use super::*;

use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_three_vm_initial_merge_from_all_peers_and_conflicts() {
    let _guard = live_daemon_test_guard().await;
    let conflict_path = "initial/conflict.txt";
    let seed_files = vec![
        SeedFile::new(Client::Windows, "initial/windows-only.txt", b"windows only"),
        SeedFile::new(Client::Ubuntu, "initial/ubuntu-only.txt", b"ubuntu only"),
        SeedFile::new(Client::MacOS, "initial/macos-only.txt", b"macos only"),
        SeedFile::new(Client::Windows, "initial/same.txt", b"same bytes"),
        SeedFile::new(Client::Ubuntu, "initial/same.txt", b"same bytes"),
        SeedFile::new(Client::MacOS, "initial/same.txt", b"same bytes"),
        SeedFile::new(
            Client::MacOS,
            "initial/unicode/Raksmorgas-动作-Адрес.txt",
            b"unicode path bytes",
        ),
        SeedFile::new(Client::Windows, conflict_path, b"windows conflict"),
        SeedFile::new(Client::Ubuntu, conflict_path, b"ubuntu conflict"),
        SeedFile::new(Client::MacOS, conflict_path, b"macos conflict"),
    ];
    let cluster = SyncCluster::start_with_options(SyncClusterOptions {
        blossom_upload_delay: Duration::ZERO,
        seed_files,
        clients: Client::THREE_VM.to_vec(),
    })
    .await;
    cluster.wait_until_authorized().await;

    let expected_hashes = [
        to_hex(&sha256(b"windows conflict")),
        to_hex(&sha256(b"ubuntu conflict")),
        to_hex(&sha256(b"macos conflict")),
    ];
    wait_for_hashes_with_prefix_all(
        &cluster,
        "initial/conflict",
        &expected_hashes,
        "three-device initial conflict copies",
    )
    .await;

    for client in Client::THREE_VM {
        cluster.assert_file(client, "initial/windows-only.txt", b"windows only");
        cluster.assert_file(client, "initial/ubuntu-only.txt", b"ubuntu only");
        cluster.assert_file(client, "initial/macos-only.txt", b"macos only");
        cluster.assert_file(client, "initial/same.txt", b"same bytes");
        cluster.assert_file(
            client,
            "initial/unicode/Raksmorgas-动作-Адрес.txt",
            b"unicode path bytes",
        );
    }
    let expected = dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_snapshot(&expected, "three-device initial merge")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_three_vm_match_seafile_release_operation_permutations() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start_three(Duration::from_millis(100)).await;
    cluster.wait_until_authorized().await;

    for source in Client::THREE_VM {
        let prefix = format!("release/{}", source.label());
        matrix_progress(format!(
            "seafile permutations source={} add-delete-add",
            source.label()
        ));
        seafile_add_delete_add_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} create-update",
            source.label()
        ));
        seafile_create_update_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} rename",
            source.label()
        ));
        seafile_rename_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} delete",
            source.label()
        ));
        seafile_delete_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} case-rename",
            source.label()
        ));
        seafile_case_rename_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} empty-directory",
            source.label()
        ));
        seafile_empty_directory_sequence(&cluster, source, &prefix).await;
        matrix_progress(format!(
            "seafile permutations source={} single-operation",
            source.label()
        ));
        seafile_single_operation_sequence(&cluster, source, &prefix).await;
    }
}

async fn seafile_add_delete_add_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let path = scoped(prefix, "add-delete-add/1.txt");
    let dir = scoped(prefix, "add-delete-add");
    let bytes = format!("aaaaaaaa from {}", source.label()).into_bytes();
    cluster.write(source, &path, &bytes).await;
    cluster.remove_all(source, &dir).await;
    cluster.write(source, &path, &bytes).await;
    cluster
        .wait_for_convergence_from(source, &format!("{} add-delete-add", source.label()))
        .await;
    assert_file_all(cluster, &path, &bytes);
}

async fn seafile_create_update_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let file = scoped(prefix, "create-update/test/1.txt");
    let first = format!("111 from {}", source.label()).into_bytes();
    let second = format!("222 from {}", source.label()).into_bytes();
    cluster.write(source, &file, &first).await;
    cluster.write(source, &file, &second).await;
    cluster
        .write(
            source,
            &scoped(prefix, "create-update/copy-source/1.txt"),
            b"111",
        )
        .await;
    cluster
        .write(
            source,
            &scoped(prefix, "create-update/copy-source/2/2.txt"),
            b"222",
        )
        .await;
    cluster
        .write(
            source,
            &scoped(prefix, "create-update/test/copied/1.txt"),
            b"111",
        )
        .await;
    cluster
        .write(
            source,
            &scoped(prefix, "create-update/test/copied/2/2.txt"),
            b"222",
        )
        .await;
    cluster
        .mkdir(source, &scoped(prefix, "create-update/empty"))
        .await;
    cluster
        .write(
            source,
            &scoped(prefix, "create-update/empty/test.md"),
            b"dddddddddddddddddddddd",
        )
        .await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} create-update-deep-copy", source.label()),
        )
        .await;
    assert_file_all(cluster, &file, &second);
}

async fn seafile_rename_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let base = scoped(prefix, "rename");
    cluster
        .write(source, &format!("{base}/1.txt"), b"111")
        .await;
    cluster
        .rename(source, &format!("{base}/1.txt"), &format!("{base}/2.txt"))
        .await;
    cluster
        .write(source, &format!("{base}/3.txt"), b"222")
        .await;
    cluster
        .rename(source, &format!("{base}/2.txt"), &format!("{base}/3.txt"))
        .await;
    cluster
        .write(source, &format!("{base}/test.txt"), b"test")
        .await;
    cluster.mkdir(source, &format!("{base}/test")).await;
    cluster
        .write(source, &format!("{base}/4.txt"), b"444")
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/test.txt"),
            &format!("{base}/test/test.txt"),
        )
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/3.txt"),
            &format!("{base}/test/3.txt"),
        )
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/4.txt"),
            &format!("{base}/test/4.txt"),
        )
        .await;
    cluster.mkdir(source, &format!("{base}/test2")).await;
    cluster
        .rename(
            source,
            &format!("{base}/test"),
            &format!("{base}/test2/test"),
        )
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/test2/test"),
            &format!("{base}/test"),
        )
        .await;
    cluster
        .write(source, &format!("{base}/test/4.txt"), b"444555")
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/test"),
            &format!("{base}/test2/test"),
        )
        .await;
    cluster
        .rename(source, &format!("{base}/test2"), &format!("{base}/test3"))
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} rename chain", source.label()))
        .await;
    assert_missing_all(cluster, &format!("{base}/test/4.txt"));
    assert_file_all(cluster, &format!("{base}/test3/test/4.txt"), b"444555");
}

async fn seafile_delete_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let base = scoped(prefix, "delete");
    cluster
        .write(source, &format!("{base}/2.txt"), b"2222")
        .await;
    cluster
        .write(source, &format!("{base}/1/1.txt"), b"111")
        .await;
    cluster
        .write(source, &format!("{base}/1/2/2.txt"), b"222")
        .await;
    cluster
        .write(source, &format!("{base}/test/1/1.txt"), b"111")
        .await;
    cluster
        .write(source, &format!("{base}/test/1/2/2.txt"), b"222")
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} delete baseline", source.label()))
        .await;

    cluster.remove(source, &format!("{base}/2.txt")).await;
    cluster.remove_all(source, &format!("{base}/1")).await;
    cluster.remove_all(source, &format!("{base}/test/1")).await;
    cluster.remove_all(source, &format!("{base}/test")).await;
    cluster
        .wait_for_convergence_from(source, &format!("{} delete file and dirs", source.label()))
        .await;
    assert_missing_all(cluster, &format!("{base}/2.txt"));
    assert_missing_all(cluster, &format!("{base}/1/1.txt"));
    assert_missing_all(cluster, &format!("{base}/test/1/2/2.txt"));
}

async fn seafile_case_rename_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let lower = scoped(prefix, "case/test/a.txt");
    let upper = scoped(prefix, "case/TEST/a.txt");
    cluster.write(source, &lower, b"case bytes").await;
    cluster
        .wait_for_convergence_from(source, &format!("{} case baseline", source.label()))
        .await;
    cluster
        .rename(
            source,
            &scoped(prefix, "case/test"),
            &scoped(prefix, "case/TEST"),
        )
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} case upper rename", source.label()))
        .await;
    assert_visible_missing_all(cluster, &lower);
    assert_file_all(cluster, &upper, b"case bytes");

    cluster
        .rename(
            source,
            &scoped(prefix, "case/TEST"),
            &scoped(prefix, "case/test"),
        )
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} case lower rename", source.label()))
        .await;
    assert_file_all(cluster, &lower, b"case bytes");
    assert_visible_missing_all(cluster, &upper);
}

async fn seafile_empty_directory_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let empty = scoped(prefix, "empty-dir");
    let renamed = scoped(prefix, "empty-dir-renamed");
    cluster.mkdir(source, &empty).await;
    cluster
        .wait_for_provider_entry(
            &empty,
            "directory",
            &format!("{} empty dir create", source.label()),
        )
        .await;
    for client in Client::THREE_VM {
        cluster.assert_provider_entry(client, &empty, "directory");
    }

    cluster.rename(source, &empty, &renamed).await;
    cluster
        .wait_for_provider_missing(&empty, &format!("{} empty dir source gone", source.label()))
        .await;
    for client in Client::THREE_VM {
        cluster.assert_provider_missing(client, &empty);
    }
    cluster
        .wait_for_provider_entry(
            &renamed,
            "directory",
            &format!("{} empty dir rename", source.label()),
        )
        .await;

    cluster.remove_all(source, &renamed).await;
    cluster
        .wait_for_provider_missing(&renamed, &format!("{} empty dir delete", source.label()))
        .await;
    for client in Client::THREE_VM {
        cluster.assert_provider_missing(client, &renamed);
    }
}

async fn seafile_single_operation_sequence(cluster: &SyncCluster, source: Client, prefix: &str) {
    let base = scoped(prefix, "single");
    cluster
        .write(source, &format!("{base}/1.txt"), b"11111")
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} single create file", source.label()))
        .await;
    cluster
        .write(source, &format!("{base}/1.txt"), b"22222")
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} single update file", source.label()))
        .await;
    cluster.mkdir(source, &format!("{base}/dir1")).await;
    cluster
        .wait_for_provider_entry(
            &format!("{base}/dir1"),
            "directory",
            &format!("{} single create empty dir", source.label()),
        )
        .await;
    cluster
        .rename(source, &format!("{base}/1.txt"), &format!("{base}/2.txt"))
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} single rename file", source.label()))
        .await;
    cluster
        .rename(source, &format!("{base}/dir1"), &format!("{base}/dir2"))
        .await;
    cluster
        .wait_for_provider_entry(
            &format!("{base}/dir2"),
            "directory",
            &format!("{} single rename empty dir", source.label()),
        )
        .await;
    cluster
        .write(source, &format!("{base}/dir2/1.txt"), b"1111111")
        .await;
    cluster
        .wait_for_convergence_from(source, &format!("{} single file in dir", source.label()))
        .await;
    cluster
        .rename(source, &format!("{base}/dir2"), &format!("{base}/dir3"))
        .await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} single rename full dir", source.label()),
        )
        .await;
    cluster.remove(source, &format!("{base}/dir3/1.txt")).await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} single remove nested file", source.label()),
        )
        .await;
    cluster
        .write(source, &format!("{base}/dir4/2.txt"), b"2222222")
        .await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} single create move source", source.label()),
        )
        .await;
    cluster
        .rename(
            source,
            &format!("{base}/dir4"),
            &format!("{base}/dir3/dir4"),
        )
        .await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} single move non-empty dir", source.label()),
        )
        .await;
    cluster.remove(source, &format!("{base}/2.txt")).await;
    cluster.remove_all(source, &format!("{base}/dir3")).await;
    cluster
        .wait_for_convergence_from(
            source,
            &format!("{} single delete leftovers", source.label()),
        )
        .await;
    assert_missing_all(cluster, &format!("{base}/2.txt"));
    assert_missing_all(cluster, &format!("{base}/dir3/dir4/2.txt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_three_vm_preserve_concurrent_edit_delete_as_conflict_copy() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start_three(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    let path = "conflicts/edit-delete.txt";
    cluster
        .write(Client::Windows, path, b"baseline before edit-delete")
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "edit-delete baseline")
        .await;

    for client in Client::THREE_VM {
        cluster.stop_daemon(client);
    }
    cluster
        .provider_write(
            Client::Windows,
            path,
            b"windows edited while ubuntu deleted",
        )
        .await;
    let write_second = unix_seconds();
    wait_for_next_unix_second(write_second).await;
    cluster.provider_delete(Client::Ubuntu, path).await;
    for client in Client::THREE_VM {
        cluster.start_daemon(client);
    }
    cluster.wait_until_direct_peers_connected().await;

    let expected_hashes = [to_hex(&sha256(b"windows edited while ubuntu deleted"))];
    wait_for_hashes_with_prefix_all(
        &cluster,
        "conflicts/edit-delete (conflict from ",
        &expected_hashes,
        "edit-delete conflict copy",
    )
    .await;
    assert_visible_missing_all(&cluster, path);
}

fn scoped(prefix: &str, path: &str) -> String {
    format!("{prefix}/{path}")
}

fn assert_file_all(cluster: &SyncCluster, path: &str, expected: &[u8]) {
    for client in Client::THREE_VM {
        cluster.assert_file(client, path, expected);
    }
}

fn assert_missing_all(cluster: &SyncCluster, path: &str) {
    for client in Client::THREE_VM {
        cluster.assert_missing(client, path);
    }
}

fn assert_visible_missing_all(cluster: &SyncCluster, path: &str) {
    for client in Client::THREE_VM {
        assert!(
            !visible_dir_snapshot(cluster.path(client)).contains_key(path),
            "{} visible snapshot should not contain {path}\n{}",
            client.label(),
            cluster.debug_state_with_rerun_hint()
        );
        cluster.assert_provider_missing(client, path);
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn wait_for_next_unix_second(previous: u64) {
    while unix_seconds() <= previous {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_hashes_with_prefix_all(
    cluster: &SyncCluster,
    prefix: &str,
    expected_hashes: &[String],
    label: &str,
) {
    let start = Instant::now();
    while start.elapsed() < WAIT_TIMEOUT {
        for client in Client::THREE_VM {
            cluster.refresh_view(client).await;
        }
        if Client::THREE_VM.into_iter().all(|client| {
            snapshot_has_hashes_with_prefix(
                &dir_snapshot(cluster.path(client)),
                prefix,
                expected_hashes,
            )
        }) {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!(
        "timed out waiting for {label}\n{}",
        cluster.debug_state_with_rerun_hint()
    );
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
        ..SyncClusterOptions::default()
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
