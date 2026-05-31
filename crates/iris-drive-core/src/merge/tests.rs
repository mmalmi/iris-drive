#[allow(clippy::wildcard_imports)]
use super::*;

fn dev_root(published_at: i64) -> DeviceRootRef {
    DeviceRootRef::legacy(format!("cid-{published_at}"), published_at, 1)
}

fn causal_root(
    root_cid: &str,
    published_at: i64,
    device_seq: u64,
    observed: &[(&str, u64, &str)],
) -> DeviceRootRef {
    DeviceRootRef {
        root_cid: root_cid.into(),
        published_at,
        dck_generation: 1,
        device_seq,
        parents: Vec::new(),
        observed: observed
            .iter()
            .map(|(device, seq, cid)| {
                (
                    (*device).to_string(),
                    crate::RootObservation {
                        device_seq: *seq,
                        root_cid: (*cid).to_string(),
                    },
                )
            })
            .collect(),
        local_only: false,
    }
}

fn file(path: &str, hash_byte: u8, size: u64) -> DeviceFileEntry {
    DeviceFileEntry {
        path: path.into(),
        hash: [hash_byte; 32],
        size,
        whole_file_hash: None,
    }
}

fn file_with_whole_hash(
    path: &str,
    cid_hash_byte: u8,
    whole_hash_byte: u8,
    size: u64,
) -> DeviceFileEntry {
    DeviceFileEntry {
        path: path.into(),
        hash: [cid_hash_byte; 32],
        size,
        whole_file_hash: Some([whole_hash_byte; 32]),
    }
}

fn tomb(path: &str, at: i64) -> DeviceTombstone {
    DeviceTombstone {
        path: path.into(),
        tombstoned_at: at,
    }
}

fn snap<'a>(
    device: &'a str,
    root: &'a DeviceRootRef,
    files: Vec<DeviceFileEntry>,
    tombstones: Vec<DeviceTombstone>,
) -> DeviceSnapshot<'a> {
    DeviceSnapshot {
        device_pubkey: device,
        root,
        files,
        tombstones,
    }
}

#[test]
fn empty_merge_is_empty() {
    let view = merge_drives(&[], &[]);
    assert!(view.files.is_empty());
    assert!(view.suppressed_by_tombstone.is_empty());
}

#[test]
fn single_device_files_pass_through() {
    let r = dev_root(100);
    let view = merge_drives(
        &["dev-a"],
        &[snap(
            "dev-a",
            &r,
            vec![file("hello.txt", 1, 5), file("dir/x", 2, 3)],
            vec![],
        )],
    );
    assert_eq!(view.files.len(), 2);
    assert_eq!(view.files[0].path, "dir/x");
    assert_eq!(view.files[0].source_device, "dev-a");
    assert_eq!(view.files[1].path, "hello.txt");
}

#[test]
fn ignored_paths_are_not_merged_from_legacy_roots() {
    let r = dev_root(100);
    let view = merge_drives(
        &["dev-a"],
        &[snap(
            "dev-a",
            &r,
            vec![
                file("keep.txt", 1, 5),
                file(".Trash-1000/files/removed.txt", 2, 7),
                file("$RECYCLE.BIN/S-1-5-21/removed.txt", 3, 7),
                file("notes/.DS_Store", 4, 9),
            ],
            vec![tomb(".Trash-1000/files/removed.txt", 99)],
        )],
    );

    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].path, "keep.txt");
    assert!(view.suppressed_by_tombstone.is_empty());
}

#[test]
fn unauthorized_device_is_ignored() {
    let r_ok = dev_root(100);
    let r_evil = dev_root(999);
    let view = merge_drives(
        &["dev-a"], // only dev-a authorized
        &[
            snap("dev-a", &r_ok, vec![file("ok.txt", 1, 1)], vec![]),
            snap(
                "dev-evil",
                &r_evil,
                vec![file("ok.txt", 9, 1)], // tries to overwrite
                vec![],
            ),
        ],
    );
    // dev-evil's write doesn't win because it isn't in the allow list.
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].hash, [1u8; 32]);
    assert_eq!(view.files[0].source_device, "dev-a");
}

#[test]
fn lww_picks_newer_publisher_for_same_path() {
    let r_old = dev_root(100);
    let r_new = dev_root(200);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_old, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_new, vec![file("x", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].source_device, "dev-b");
    assert_eq!(view.files[0].hash, [2u8; 32]);
}

#[test]
fn causal_descendant_wins_even_with_older_wall_clock() {
    let r_a = causal_root("cid-a", 300, 1, &[]);
    let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a")]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].source_device, "dev-b");
    assert_eq!(view.files[0].hash, [2u8; 32]);
    assert!(view.conflicts.is_empty());
}

#[test]
fn observed_same_sequence_with_different_root_is_not_descendant() {
    let r_a = causal_root("cid-a", 300, 1, &[]);
    let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a-fork")]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(
        view.files[0].source_device, "dev-a",
        "legacy timestamp ordering should win when ancestry is unknown"
    );
    assert_eq!(view.conflicts, vec!["x".to_string()]);
}

#[test]
fn concurrent_different_writes_are_marked_as_conflicts() {
    let r_a = causal_root("cid-a", 100, 1, &[]);
    let r_b = causal_root("cid-b", 200, 1, &[]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.conflicts, vec!["x".to_string()]);
    assert_eq!(view.conflict_details.len(), 1);
    let detail = &view.conflict_details[0];
    assert_eq!(detail.path, "x");
    assert_eq!(detail.kind, MergedConflictKind::WriteWrite);
    assert!(detail.tombstone.is_none());
    assert_eq!(detail.files.len(), 2);
    assert_eq!(detail.files[0].device_id, "dev-a");
    assert_eq!(detail.files[0].device_seq, 1);
    assert_eq!(detail.files[0].root_cid, "cid-a");
    assert_eq!(detail.files[0].content_hash, "01".repeat(32));
    assert_eq!(detail.files[1].device_id, "dev-b");
    assert_eq!(detail.files[1].device_seq, 1);
    assert_eq!(detail.files[1].root_cid, "cid-b");
    assert_eq!(detail.files[1].content_hash, "02".repeat(32));
}

#[test]
fn concurrent_same_content_converges_without_conflict() {
    let r_a = causal_root("cid-a", 100, 1, &[]);
    let r_b = causal_root("cid-b", 200, 1, &[]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("x", 1, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert!(view.conflicts.is_empty());
}

#[test]
fn concurrent_same_whole_file_hash_converges_despite_different_content_cids() {
    let r_a = causal_root("cid-a", 100, 1, &[]);
    let r_b = causal_root("cid-b", 200, 1, &[]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap(
                "dev-a",
                &r_a,
                vec![file_with_whole_hash("x", 1, 9, 1024)],
                vec![],
            ),
            snap(
                "dev-b",
                &r_b,
                vec![file_with_whole_hash("x", 2, 9, 1024)],
                vec![],
            ),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].whole_file_hash, Some([9u8; 32]));
    assert!(view.conflicts.is_empty());
}

#[test]
fn concurrent_different_whole_file_hashes_conflict_even_with_same_content_cid() {
    let r_a = causal_root("cid-a", 100, 1, &[]);
    let r_b = causal_root("cid-b", 200, 1, &[]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap(
                "dev-a",
                &r_a,
                vec![file_with_whole_hash("x", 1, 8, 1024)],
                vec![],
            ),
            snap(
                "dev-b",
                &r_b,
                vec![file_with_whole_hash("x", 1, 9, 1024)],
                vec![],
            ),
        ],
    );
    assert_eq!(view.conflicts, vec!["x".to_string()]);
}

#[test]
fn causal_tombstone_suppresses_observed_write_even_with_older_clock() {
    let r_a = causal_root("cid-a", 300, 1, &[]);
    let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a")]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![], vec![tomb("x", 100)]),
        ],
    );
    assert!(view.files.is_empty());
    assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
    assert!(view.conflicts.is_empty());
}

#[test]
fn concurrent_write_delete_is_marked_as_conflict() {
    let r_a = causal_root("cid-a", 100, 1, &[]);
    let r_b = causal_root("cid-b", 200, 1, &[]);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![], vec![tomb("x", 200)]),
        ],
    );
    assert_eq!(view.conflicts, vec!["x".to_string()]);
    assert_eq!(view.conflict_details.len(), 1);
    let detail = &view.conflict_details[0];
    assert_eq!(detail.path, "x");
    assert_eq!(detail.kind, MergedConflictKind::WriteDelete);
    assert_eq!(detail.files.len(), 1);
    assert_eq!(detail.files[0].device_id, "dev-a");
    let tombstone = detail.tombstone.as_ref().unwrap();
    assert_eq!(tombstone.device_id, "dev-b");
    assert_eq!(tombstone.device_seq, 1);
    assert_eq!(tombstone.root_cid, "cid-b");
    assert_eq!(tombstone.tombstoned_at, 200);
}

#[test]
fn disjoint_paths_all_appear() {
    let r_a = dev_root(100);
    let r_b = dev_root(200);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("a.txt", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("b.txt", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 2);
    assert_eq!(view.files[0].path, "a.txt");
    assert_eq!(view.files[1].path, "b.txt");
}

#[test]
fn newer_tombstone_suppresses_older_write() {
    let r_a = dev_root(100);
    let r_b = dev_root(200);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![], vec![tomb("x", 200)]),
        ],
    );
    assert!(view.files.is_empty());
    assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
}

#[test]
fn newer_write_resurrects_after_older_tombstone() {
    let r_old = dev_root(100);
    let r_new = dev_root(200);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r_old, vec![], vec![tomb("x", 100)]),
            snap("dev-b", &r_new, vec![file("x", 2, 1)], vec![]),
        ],
    );
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].source_device, "dev-b");
}

#[test]
fn same_timestamp_tombstone_wins_over_write() {
    // Deletion is conservative on ties.
    let r = dev_root(100);
    let view = merge_drives(
        &["dev-a", "dev-b"],
        &[
            snap("dev-a", &r, vec![file("x", 1, 1)], vec![]),
            snap("dev-b", &r, vec![], vec![tomb("x", 100)]),
        ],
    );
    assert!(view.files.is_empty());
    assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
}

#[test]
fn three_devices_converge() {
    let r_a = dev_root(100);
    let r_b = dev_root(200);
    let r_c = dev_root(300);
    let view = merge_drives(
        &["dev-a", "dev-b", "dev-c"],
        &[
            snap(
                "dev-a",
                &r_a,
                vec![file("alpha", 1, 1), file("contested", 1, 1)],
                vec![],
            ),
            snap(
                "dev-b",
                &r_b,
                vec![file("beta", 2, 1), file("contested", 2, 1)],
                vec![],
            ),
            snap(
                "dev-c",
                &r_c,
                vec![file("gamma", 3, 1), file("contested", 3, 1)],
                vec![],
            ),
        ],
    );
    assert_eq!(view.files.len(), 4);
    let paths: Vec<&str> = view.files.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["alpha", "beta", "contested", "gamma"]);
    // Contested file resolved to dev-c (latest).
    let contested = view.files.iter().find(|e| e.path == "contested").unwrap();
    assert_eq!(contested.source_device, "dev-c");
}

#[test]
fn tombstone_in_one_device_wipes_across_others_when_newer() {
    let r_a = dev_root(100);
    let r_b = dev_root(150);
    let r_c = dev_root(200);
    let view = merge_drives(
        &["dev-a", "dev-b", "dev-c"],
        &[
            snap("dev-a", &r_a, vec![file("shared", 1, 1)], vec![]),
            snap("dev-b", &r_b, vec![file("shared", 2, 1)], vec![]),
            snap("dev-c", &r_c, vec![], vec![tomb("shared", 200)]),
        ],
    );
    assert!(view.files.is_empty());
    assert_eq!(view.suppressed_by_tombstone, vec!["shared".to_string()]);
}

#[test]
fn local_only_roots_do_not_compete_with_source_roots() {
    let remote_v2 = causal_root("remote-v2", 100, 2, &[]);
    let mut local_mirror_v1 = causal_root("local-mirror-v1", 200, 1, &[("remote", 1, "remote-v1")]);
    local_mirror_v1.local_only = true;

    let view = merge_drives(
        &["local", "remote"],
        &[
            snap(
                "local",
                &local_mirror_v1,
                vec![file("note.txt", 1, 5)],
                vec![],
            ),
            snap("remote", &remote_v2, vec![file("note.txt", 2, 7)], vec![]),
        ],
    );

    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].source_device, "remote");
    assert_eq!(view.files[0].hash, [2; 32]);
    assert!(view.conflicts.is_empty());
}

#[test]
fn local_only_roots_do_not_block_source_tombstones() {
    let remote_delete = causal_root("remote-delete", 100, 2, &[]);
    let mut local_mirror_v1 = causal_root("local-mirror-v1", 200, 1, &[("remote", 1, "remote-v1")]);
    local_mirror_v1.local_only = true;

    let view = merge_drives(
        &["local", "remote"],
        &[
            snap(
                "local",
                &local_mirror_v1,
                vec![file("note.txt", 1, 5)],
                vec![],
            ),
            snap(
                "remote",
                &remote_delete,
                vec![],
                vec![tomb("note.txt", 100)],
            ),
        ],
    );

    assert!(view.files.is_empty());
    assert_eq!(view.suppressed_by_tombstone, vec!["note.txt"]);
    assert!(view.conflicts.is_empty());
}

#[test]
fn output_is_sorted_lexicographic() {
    let r = dev_root(100);
    let view = merge_drives(
        &["dev-a"],
        &[snap(
            "dev-a",
            &r,
            vec![file("zeta", 1, 1), file("alpha", 2, 1), file("mid", 3, 1)],
            vec![],
        )],
    );
    let paths: Vec<&str> = view.files.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["alpha", "mid", "zeta"]);
}

#[test]
fn tombstone_path_round_trip() {
    let original = "Photos/IMG_001.heic";
    let encoded = tombstone_path(original);
    assert_eq!(encoded, ".hashtree/tombstones/Photos/IMG_001.heic");
    assert_eq!(original_path_from_tombstone(&encoded), Some(original));
}

#[test]
fn original_path_from_non_tombstone_is_none() {
    assert!(original_path_from_tombstone("notes.txt").is_none());
    assert!(original_path_from_tombstone(".hashtree/tombstones").is_none());
}
