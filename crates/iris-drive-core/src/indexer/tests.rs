#[allow(clippy::wildcard_imports)]
use super::*;
use crate::conflict::{ConflictRecord, ConflictSide, ConflictState};
use crate::root_meta::{DriveRootMeta, RootObservation, RootParent};
use hashtree_core::{DEFAULT_CHUNK_SIZE, HashTreeConfig, MemoryStore, sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tempfile::tempdir;

fn new_tree() -> HashTree<MemoryStore> {
    HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public())
}

#[tokio::test]
async fn empty_dir_indexes_to_empty_htree_dir() {
    let dir = tempdir().unwrap();
    let tree = new_tree();
    let cid = index_dir(&tree, dir.path()).await.unwrap();
    let listing = tree.list_directory(&cid).await.unwrap();
    assert!(listing.is_empty());
}

#[tokio::test]
async fn single_file_appears_with_correct_name_and_size() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), b"hi there").unwrap();
    let tree = new_tree();
    let cid = index_dir(&tree, dir.path()).await.unwrap();
    let listing = tree.list_directory(&cid).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "hello.txt");
    assert_eq!(listing[0].size, 8);
}

#[tokio::test]
async fn nested_dir_indexed_recursively() {
    let dir = tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("a.txt"), b"a").unwrap();
    let tree = new_tree();
    let cid = index_dir(&tree, dir.path()).await.unwrap();
    let top = tree.list_directory(&cid).await.unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].name, "sub");
    let sub_cid = Cid {
        hash: top[0].hash,
        key: top[0].key,
    };
    let sub = tree.list_directory(&sub_cid).await.unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].name, "a.txt");
}

#[tokio::test]
async fn indexing_is_deterministic() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a"), b"alpha").unwrap();
    std::fs::write(dir.path().join("b"), b"beta").unwrap();
    std::fs::create_dir(dir.path().join("inner")).unwrap();
    std::fs::write(dir.path().join("inner").join("c"), b"gamma").unwrap();
    let cid_1 = index_dir(&new_tree(), dir.path()).await.unwrap();
    let cid_2 = index_dir(&new_tree(), dir.path()).await.unwrap();
    assert_eq!(cid_1.hash, cid_2.hash);
}

#[tokio::test]
async fn different_contents_produce_different_cids() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    std::fs::write(dir_a.path().join("a.txt"), b"alpha").unwrap();
    std::fs::write(dir_b.path().join("a.txt"), b"different").unwrap();
    let cid_a = index_dir(&new_tree(), dir_a.path()).await.unwrap();
    let cid_b = index_dir(&new_tree(), dir_b.path()).await.unwrap();
    assert_ne!(cid_a.hash, cid_b.hash);
}

#[tokio::test]
async fn symlinks_are_ignored() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), b"real").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();
    let tree = new_tree();
    let cid = index_dir(&tree, dir.path()).await.unwrap();
    let listing = tree.list_directory(&cid).await.unwrap();
    // On Unix we expect only the real file; on non-Unix the symlink
    // isn't created so we also expect just one entry. Either way:
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "real.txt");
}

#[tokio::test]
async fn built_in_noise_files_are_ignored() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), b"real").unwrap();
    std::fs::write(dir.path().join(".DS_Store"), b"finder state").unwrap();
    std::fs::write(dir.path().join("._real.txt"), b"resource fork").unwrap();
    std::fs::write(dir.path().join("Thumbs.db"), b"windows state").unwrap();
    std::fs::write(dir.path().join("desktop.ini"), b"windows metadata").unwrap();
    std::fs::write(dir.path().join("draft~"), b"editor backup").unwrap();
    std::fs::write(dir.path().join("#draft#"), b"emacs temp").unwrap();
    std::fs::write(dir.path().join("backup.sbak"), b"seafile backup").unwrap();
    std::fs::create_dir(dir.path().join(".hashtree")).unwrap();
    std::fs::write(dir.path().join(".hashtree").join("prev"), b"internal").unwrap();
    std::fs::create_dir_all(dir.path().join(".Trash-1000").join("files")).unwrap();
    std::fs::write(
        dir.path().join(".Trash-1000").join("files").join("old.txt"),
        b"trash",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("$RECYCLE.BIN").join("S-1-5-21")).unwrap();
    std::fs::write(
        dir.path()
            .join("$RECYCLE.BIN")
            .join("S-1-5-21")
            .join("old.txt"),
        b"recycle",
    )
    .unwrap();

    let tree = new_tree();
    let cid = index_dir(&tree, dir.path()).await.unwrap();
    let listing = tree.list_directory(&cid).await.unwrap();

    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "real.txt");
}

#[tokio::test]
async fn ignored_files_do_not_keep_removed_files_alive() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("removed.txt"), b"bye").unwrap();
    std::fs::write(dir.path().join(".DS_Store"), b"finder state").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();

    std::fs::remove_file(dir.path().join("removed.txt")).unwrap();
    std::fs::create_dir_all(dir.path().join(".Trash-1000").join("files")).unwrap();
    std::fs::write(
        dir.path()
            .join(".Trash-1000")
            .join("files")
            .join("removed.txt"),
        b"bye",
    )
    .unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1234)
        .await
        .unwrap();

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    assert!(files.is_empty());
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "removed.txt");
}

#[tokio::test]
async fn non_existent_dir_errors() {
    let tree = new_tree();
    let err = index_dir(&tree, Path::new("/this/should/not/exist/abcxyz"))
        .await
        .unwrap_err();
    assert!(matches!(err, IndexError::NotADirectory(_)));
}

// ----- index_dir_with_history / tombstone lifecycle -----

#[tokio::test]
async fn history_with_no_previous_root_matches_index_dir() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let tree = new_tree();
    let cid_plain = index_dir(&tree, dir.path()).await.unwrap();
    let cid_history = index_dir_with_history(&tree, dir.path(), None, 1000)
        .await
        .unwrap();
    assert_eq!(cid_plain.hash, cid_history.hash);
}

#[tokio::test]
async fn root_metadata_is_embedded_under_hashtree_and_not_user_visible() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let tree = new_tree();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: "main".into(),
        device_id: "device-a".into(),
        device_seq: 2,
        dck_generation: 1,
        local_only: false,
        parents: vec![RootParent {
            device_id: "device-a".into(),
            device_seq: 1,
            root_cid: "cid-parent".into(),
        }],
        observed: BTreeMap::from([(
            "device-b".into(),
            RootObservation {
                device_seq: 7,
                root_cid: "cid-b".into(),
            },
        )]),
        created_at: 1234,
    };

    let root = index_dir_with_history_and_meta(&tree, dir.path(), None, 1234, Some(&meta))
        .await
        .unwrap();

    let loaded = read_root_meta(&tree, &root).await.unwrap().unwrap();
    assert_eq!(loaded, meta);

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &root).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.txt");
    assert!(tombstones.is_empty());
}

#[tokio::test]
async fn indexed_large_files_preserve_whole_file_hash_metadata() {
    let dir = tempdir().unwrap();
    let bytes = vec![42u8; DEFAULT_CHUNK_SIZE + 1];
    let whole_file_hash = sha256(&bytes);
    std::fs::write(dir.path().join("large.bin"), &bytes).unwrap();
    let tree = new_tree();

    let root = index_dir(&tree, dir.path()).await.unwrap();

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &root).await.unwrap();
    assert!(tombstones.is_empty());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "large.bin");
    assert_eq!(files[0].whole_file_hash, Some(whole_file_hash));
    assert_ne!(
        files[0].hash, whole_file_hash,
        "large-file CID is a chunk-tree hash, not the whole-file hash"
    );
}

#[tokio::test]
async fn indexed_files_preserve_modified_at_metadata() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("note.txt");
    std::fs::write(&file_path, b"note").unwrap();
    let modified_at: i64 = 1_700_000_123;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file_path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(modified_at.cast_unsigned()))
        .unwrap();
    let tree = new_tree();

    let root = index_dir(&tree, dir.path()).await.unwrap();

    let listing = tree.list_directory(&root).await.unwrap();
    let stored = listing[0]
        .meta
        .as_ref()
        .and_then(|meta| meta.get(MODIFIED_AT_META_KEY))
        .and_then(serde_json::Value::as_i64);
    assert_eq!(stored, Some(modified_at));
    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &root).await.unwrap();
    assert!(tombstones.is_empty());
    assert_eq!(files[0].modified_at, Some(modified_at));
}

#[tokio::test]
async fn conflict_records_round_trip_and_are_not_user_visible() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let tree = new_tree();
    let root = index_dir(&tree, dir.path()).await.unwrap();
    let record = ConflictRecord {
        schema: ConflictRecord::SCHEMA,
        conflict_id: "dev-a-2-dev-b-7".into(),
        path: "report.pdf".into(),
        visible_conflict_path: "report (conflict from phone).pdf".into(),
        local: ConflictSide {
            device_id: "dev-a".into(),
            device_seq: 2,
            root_cid: "cid-a".into(),
            whole_file_hash: "hash-a".into(),
        },
        remote: Some(ConflictSide {
            device_id: "dev-b".into(),
            device_seq: 7,
            root_cid: "cid-b".into(),
            whole_file_hash: "hash-b".into(),
        }),
        deleted: None,
        state: ConflictState::Unresolved,
        created_at: 1234,
    };

    let with_conflict = layer_conflict_records(&tree, root, std::slice::from_ref(&record))
        .await
        .unwrap();

    let records = read_conflict_records(&tree, &with_conflict).await.unwrap();
    assert_eq!(records, vec![record]);

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &with_conflict)
        .await
        .unwrap();
    let file_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(file_paths, vec!["a.txt"]);
    assert!(tombstones.is_empty());
}

#[tokio::test]
async fn missing_conflict_records_dir_reads_as_empty() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let tree = new_tree();
    let root = index_dir(&tree, dir.path()).await.unwrap();

    let records = read_conflict_records(&tree, &root).await.unwrap();
    assert!(records.is_empty());
}

#[tokio::test]
async fn conflict_record_id_must_be_single_path_segment() {
    let tree = new_tree();
    let root = tree.put_directory(Vec::new()).await.unwrap();
    let record = ConflictRecord {
        schema: ConflictRecord::SCHEMA,
        conflict_id: "bad/id".into(),
        path: "report.pdf".into(),
        visible_conflict_path: "report (conflict from phone).pdf".into(),
        local: ConflictSide {
            device_id: "dev-a".into(),
            device_seq: 2,
            root_cid: "cid-a".into(),
            whole_file_hash: "hash-a".into(),
        },
        remote: Some(ConflictSide {
            device_id: "dev-b".into(),
            device_seq: 7,
            root_cid: "cid-b".into(),
            whole_file_hash: "hash-b".into(),
        }),
        deleted: None,
        state: ConflictState::Unresolved,
        created_at: 1234,
    };

    let err = layer_conflict_records(&tree, root, &[record])
        .await
        .unwrap_err();
    assert!(matches!(err, IndexError::ConflictRecord(msg) if msg.contains("conflict_id")));
}

#[tokio::test]
async fn removed_file_emits_tombstone_in_next_import() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("removed.txt"), b"bye").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();

    // Remove the file, re-import with history.
    std::fs::remove_file(dir.path().join("removed.txt")).unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1234)
        .await
        .unwrap();

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    assert!(files.is_empty(), "no live files expected, got {files:?}");
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "removed.txt");
    assert_eq!(tombstones[0].tombstoned_at, 1234);
}

#[tokio::test]
async fn missing_tombstone_blob_errors_instead_of_ignoring_delete() {
    let tree = new_tree();
    let missing_hash = [42u8; 32];
    let missing_cid = Cid::public(missing_hash);
    let tombstone = DirEntry::from_cid("gone.txt".to_string(), &missing_cid).with_size(10);
    let tombstones_cid = tree.put_directory(vec![tombstone]).await.unwrap();
    let tombstones_dir =
        DirEntry::from_cid("tombstones".to_string(), &tombstones_cid).with_link_type(LinkType::Dir);
    let meta_cid = tree.put_directory(vec![tombstones_dir]).await.unwrap();
    let meta_dir =
        DirEntry::from_cid(META_DIR.to_string(), &meta_cid).with_link_type(LinkType::Dir);
    let root = tree.put_directory(vec![meta_dir]).await.unwrap();

    let err = crate::merge::walk_device_tree(&tree, &root)
        .await
        .unwrap_err();

    assert!(
        matches!(err, HashTreeError::MissingChunk(missing) if missing == hashtree_core::to_hex(&missing_hash))
    );
}

#[tokio::test]
async fn visible_root_history_emits_tombstone_without_plain_directory() {
    let first_dir = tempdir().unwrap();
    std::fs::write(first_dir.path().join("removed.txt"), b"bye").unwrap();
    std::fs::write(first_dir.path().join("kept.txt"), b"still here").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, first_dir.path()).await.unwrap();

    let visible_dir = tempdir().unwrap();
    std::fs::write(visible_dir.path().join("kept.txt"), b"still here").unwrap();
    let visible_root = index_dir(&tree, visible_dir.path()).await.unwrap();
    let second = layer_history_and_meta_on_root(&tree, visible_root, Some(&first), 5678, None)
        .await
        .unwrap();

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "kept.txt");
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "removed.txt");
    assert_eq!(tombstones[0].tombstoned_at, 5678);
}

#[tokio::test]
async fn visible_root_history_filters_ignored_entries_before_diffing() {
    let first_dir = tempdir().unwrap();
    std::fs::write(first_dir.path().join("removed.txt"), b"bye").unwrap();
    std::fs::write(first_dir.path().join("kept.txt"), b"still here").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, first_dir.path()).await.unwrap();

    let kept_cid = tree.put(b"still here").await.unwrap().0;
    let trash_cid = tree.put(b"bye").await.unwrap().0;
    let trash_file = DirEntry::from_cid("removed.txt".to_string(), &trash_cid)
        .with_size(3)
        .with_meta(file_entry_meta(&hashtree_core::sha256(b"bye"), None));
    let trash_files_cid = tree.put_directory(vec![trash_file]).await.unwrap();
    let mut trash_files_entry = DirEntry::from_cid("files".to_string(), &trash_files_cid);
    trash_files_entry.link_type = LinkType::Dir;
    let trash_dir_cid = tree.put_directory(vec![trash_files_entry]).await.unwrap();
    let mut trash_entry = DirEntry::from_cid(".Trash-1000".to_string(), &trash_dir_cid);
    trash_entry.link_type = LinkType::Dir;
    let kept_entry = DirEntry::from_cid("kept.txt".to_string(), &kept_cid)
        .with_size(10)
        .with_meta(file_entry_meta(&hashtree_core::sha256(b"still here"), None));
    let visible_root = tree
        .put_directory(vec![trash_entry, kept_entry])
        .await
        .unwrap();

    let second = layer_history_and_meta_on_root(&tree, visible_root, Some(&first), 5678, None)
        .await
        .unwrap();

    let top = tree.list_directory(&second).await.unwrap();
    assert!(top.iter().all(|entry| entry.name != ".Trash-1000"));
    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["kept.txt"]
    );
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "removed.txt");
}

#[tokio::test]
async fn visible_root_import_preserves_created_empty_directory() {
    let tree = new_tree();
    let base_root = tree.put_directory(Vec::new()).await.unwrap();
    let empty_dir = tree.put_directory(Vec::new()).await.unwrap();
    let edited_root = tree
        .set_entry(&base_root, &[], "folder", &empty_dir, 0, LinkType::Dir)
        .await
        .unwrap();

    let delta =
        local_visible_root_for_mount_import(&tree, &edited_root, None, &base_root, None, None)
            .await
            .unwrap();

    let folder = tree
        .resolve(&delta.root, "folder")
        .await
        .unwrap()
        .expect("folder should exist");
    assert!(tree.is_dir(&folder).await.unwrap());
}

#[tokio::test]
async fn visible_root_import_tombstones_deleted_scoped_directory_tree() {
    let tree = new_tree();
    let base_dir = tempdir().unwrap();
    std::fs::create_dir_all(base_dir.path().join("folder")).unwrap();
    std::fs::write(base_dir.path().join("folder").join("note.txt"), b"note").unwrap();
    let base_root = index_dir(&tree, base_dir.path()).await.unwrap();
    let edited_root = tree.put_directory(Vec::new()).await.unwrap();
    let scoped_paths = BTreeSet::from(["folder".to_string()]);

    let delta = local_visible_root_for_mount_import(
        &tree,
        &edited_root,
        None,
        &base_root,
        None,
        Some(&scoped_paths),
    )
    .await
    .unwrap();

    assert_eq!(
        delta.tombstone_paths,
        BTreeSet::from(["folder".to_string(), "folder/note.txt".to_string()])
    );
    let layered = layer_history_and_meta_on_root_with_tombstone_base_and_paths(
        &tree,
        delta.root,
        None,
        Some(&base_root),
        1234,
        None,
        Some(&delta.tombstone_paths),
    )
    .await
    .unwrap();
    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &layered)
        .await
        .unwrap();
    assert!(files.is_empty());
    assert_eq!(
        tombstones
            .iter()
            .map(|tombstone| tombstone.path.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["folder", "folder/note.txt"])
    );
}

#[tokio::test]
async fn tombstone_carries_forward_when_file_stays_absent() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("gone.txt"), b"x").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
        .await
        .unwrap();

    // Third import: file still absent, tombstone should keep its
    // original timestamp (1000), not be refreshed to 2000.
    let third = index_dir_with_history(&tree, dir.path(), Some(&second), 2000)
        .await
        .unwrap();
    let (_, tombstones) = crate::merge::walk_device_tree(&tree, &third).await.unwrap();
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].tombstoned_at, 1000, "original ts preserved");
}

#[tokio::test]
async fn tombstone_drops_when_file_returns() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("back.txt"), b"v1").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();
    std::fs::remove_file(dir.path().join("back.txt")).unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
        .await
        .unwrap();

    // File comes back.
    std::fs::write(dir.path().join("back.txt"), b"v2").unwrap();
    let third = index_dir_with_history(&tree, dir.path(), Some(&second), 2000)
        .await
        .unwrap();
    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &third).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "back.txt");
    assert!(tombstones.is_empty(), "tombstone should be gone");
}

#[tokio::test]
async fn nested_file_removal_writes_nested_tombstone() {
    let dir = tempdir().unwrap();
    std::fs::create_dir(dir.path().join("photos")).unwrap();
    std::fs::write(dir.path().join("photos").join("img.heic"), b"photo").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();

    std::fs::remove_file(dir.path().join("photos").join("img.heic")).unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 5000)
        .await
        .unwrap();

    let (_, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "photos/img.heic");
    assert_eq!(tombstones[0].tombstoned_at, 5000);
}

#[tokio::test]
async fn surviving_files_unaffected_by_unrelated_removal() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("keep.txt"), b"k").unwrap();
    std::fs::write(dir.path().join("drop.txt"), b"d").unwrap();
    let tree = new_tree();
    let first = index_dir(&tree, dir.path()).await.unwrap();

    std::fs::remove_file(dir.path().join("drop.txt")).unwrap();
    let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
        .await
        .unwrap();

    let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
        .await
        .unwrap();
    let live_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let tomb_paths: Vec<&str> = tombstones.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(live_paths, vec!["keep.txt"]);
    assert_eq!(tomb_paths, vec!["drop.txt"]);
}
