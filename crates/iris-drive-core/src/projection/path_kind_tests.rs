use super::*;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::profile::Profile;
use crate::root_meta::{DriveRootMeta, RootObservation};
use hashtree_core::{HashTreeConfig, MemoryStore, sha256};
use std::sync::Arc;
use tempfile::tempdir;

const PEER_APP_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SECOND_PEER_APP_KEY: &str =
    "3333333333333333333333333333333333333333333333333333333333333333";
const FILE_MODIFIED_AT: i64 = 1_700_000_001;
const DIRECTORY_MODIFIED_AT: i64 = 1_700_000_002;
const CHILD_MODIFIED_AT: i64 = 1_700_000_003;

#[tokio::test]
async fn causally_newer_directory_keeps_path_and_preserves_replaced_file() {
    let fixture = PathKindFixture::new();
    let file_root = fixture
        .file_root("docs", b"the former file", FILE_MODIFIED_AT)
        .await;
    let directory_root = fixture
        .directory_root("docs", "inside.txt", b"directory child")
        .await;
    let file_ref = root_ref(&file_root, &fixture.owner_app_key, 3, 999, None);
    let directory_ref = root_ref(
        &directory_root,
        PEER_APP_KEY,
        2,
        100,
        Some((&fixture.owner_app_key, 3, &file_root)),
    );

    let merged = primary_merged_root(&fixture.tree, &fixture.config(file_ref, directory_ref))
        .await
        .unwrap();
    assert_eq!(merged.top_level_entries, 2);

    assert_directory_meta(
        &fixture.tree,
        &merged.root_cid,
        "docs",
        DIRECTORY_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "docs/inside.txt",
        b"directory child",
        CHILD_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("docs (conflict from {})", fixture.owner_app_key),
        b"the former file",
        FILE_MODIFIED_AT,
    )
    .await;
}

#[tokio::test]
async fn concurrent_file_directory_conflict_keeps_directory_canonical() {
    let fixture = PathKindFixture::new();
    let file_root = fixture
        .file_root("workspace", b"concurrent file", FILE_MODIFIED_AT)
        .await;
    let directory_root = fixture
        .directory_root("workspace", "kept.txt", b"concurrent directory child")
        .await;
    let file_ref = root_ref(&file_root, &fixture.owner_app_key, 1, 999, None);
    let directory_ref = root_ref(&directory_root, PEER_APP_KEY, 1, 100, None);

    let merged = primary_merged_root(&fixture.tree, &fixture.config(file_ref, directory_ref))
        .await
        .unwrap();
    assert_eq!(merged.top_level_entries, 2);

    assert_directory_meta(
        &fixture.tree,
        &merged.root_cid,
        "workspace",
        DIRECTORY_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "workspace/kept.txt",
        b"concurrent directory child",
        CHILD_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("workspace (conflict from {})", fixture.owner_app_key),
        b"concurrent file",
        FILE_MODIFIED_AT,
    )
    .await;
}

#[tokio::test]
async fn causally_newer_file_keeps_path_and_remaps_entire_directory_subtree() {
    let fixture = PathKindFixture::new();
    let directory_root = fixture
        .directory_root("draft", "old.txt", b"old child")
        .await;
    let file_root = fixture
        .file_root("draft", b"replacement file", FILE_MODIFIED_AT)
        .await;
    let directory_ref = root_ref(&directory_root, &fixture.owner_app_key, 4, 999, None);
    let file_ref = root_ref(
        &file_root,
        PEER_APP_KEY,
        2,
        100,
        Some((&fixture.owner_app_key, 4, &directory_root)),
    );

    let merged = primary_merged_root(&fixture.tree, &fixture.config(directory_ref, file_ref))
        .await
        .unwrap();
    assert_eq!(merged.top_level_entries, 2);
    let conflict_dir = format!("draft (conflict from {})", fixture.owner_app_key);

    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "draft",
        b"replacement file",
        FILE_MODIFIED_AT,
    )
    .await;
    assert_directory_meta(
        &fixture.tree,
        &merged.root_cid,
        &conflict_dir,
        DIRECTORY_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("{conflict_dir}/old.txt"),
        b"old child",
        CHILD_MODIFIED_AT,
    )
    .await;
    let empty = fixture
        .tree
        .resolve(&merged.root_cid, &format!("{conflict_dir}/nested/empty"))
        .await
        .unwrap()
        .expect("nested empty directory survives subtree remap");
    assert!(fixture.tree.is_dir(&empty).await.unwrap());
}

#[tokio::test]
async fn causal_publication_time_drift_preserves_replaced_directory_children() {
    let fixture = PathKindFixture::new();
    let directory_root = fixture
        .directory_root("draft", "keep.txt", b"web directory child")
        .await;
    let (deleted, deleted_size) = fixture
        .tree
        .put_file(b"historically deleted")
        .await
        .unwrap();
    let directory_root = fixture
        .tree
        .set_entry(
            &directory_root,
            &["draft"],
            "deleted.txt",
            &deleted,
            deleted_size,
            LinkType::File,
        )
        .await
        .unwrap();
    let after_first_delete = fixture
        .tree
        .remove_entry(&directory_root, &["draft"], "deleted.txt")
        .await
        .unwrap();
    let first_tombstone_paths = BTreeSet::from(["draft/deleted.txt".to_string()]);
    let first_root = crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
        &fixture.tree,
        after_first_delete,
        None,
        Some(&directory_root),
        100,
        None,
        Some(&first_tombstone_paths),
    )
    .await
    .unwrap();
    let replacement_root = fixture
        .file_root("draft", b"web replacement file", FILE_MODIFIED_AT)
        .await;
    let tombstone_paths = BTreeSet::from(["draft/keep.txt".to_string()]);
    let replacement_root =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
            &fixture.tree,
            replacement_root,
            Some(&first_root),
            Some(&first_root),
            200,
            None,
            Some(&tombstone_paths),
        )
        .await
        .unwrap();
    let replacement_root = without_path_kind_roles(&fixture.tree, &replacement_root).await;
    let directory_ref = root_ref(&directory_root, &fixture.owner_app_key, 1, 100, None);
    // Web roots are causal but do not embed native root metadata. Multiple
    // same-second writes can advance their relay publication time beyond the
    // wall-clock tombstone timestamp.
    let replacement_ref = root_ref(
        &replacement_root,
        PEER_APP_KEY,
        2,
        999,
        Some((&fixture.owner_app_key, 1, &directory_root)),
    );

    let merged = primary_merged_root(
        &fixture.tree,
        &fixture.config(directory_ref, replacement_ref),
    )
    .await
    .unwrap();
    let conflict_dir = format!("draft (conflict from {})", fixture.owner_app_key);

    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "draft",
        b"web replacement file",
        FILE_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("{conflict_dir}/keep.txt"),
        b"web directory child",
        CHILD_MODIFIED_AT,
    )
    .await;
    assert!(
        fixture
            .tree
            .resolve(&merged.root_cid, &format!("{conflict_dir}/deleted.txt"))
            .await
            .unwrap()
            .is_none(),
        "the max-marker fallback must exclude an older carried tombstone"
    );
}

#[tokio::test]
async fn legacy_file_directory_conflict_preserves_suppressed_directory_children() {
    let fixture = PathKindFixture::new();
    let directory_root = fixture
        .directory_root("draft", "child.txt", b"legacy directory child")
        .await;
    let replacement_root = fixture
        .file_root("draft", b"legacy replacement file", FILE_MODIFIED_AT)
        .await;
    let tombstone_paths = BTreeSet::from(["draft/child.txt".to_string()]);
    let replacement_root =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
            &fixture.tree,
            replacement_root,
            None,
            Some(&directory_root),
            200,
            None,
            Some(&tombstone_paths),
        )
        .await
        .unwrap();
    let replacement_root = without_path_kind_roles(&fixture.tree, &replacement_root).await;
    let directory_ref = AppKeyRootRef::legacy(directory_root.to_string(), 100, 1);
    // A legacy relay publication time need not equal the tombstone's local
    // creation time, so lossless conflict projection cannot depend on equality.
    let replacement_ref = AppKeyRootRef::legacy(replacement_root.to_string(), 999, 1);

    let merged = primary_merged_root(
        &fixture.tree,
        &fixture.config(directory_ref, replacement_ref),
    )
    .await
    .unwrap();
    let conflict_path = format!("draft (conflict from {PEER_APP_KEY})");

    assert_directory_meta(
        &fixture.tree,
        &merged.root_cid,
        "draft",
        DIRECTORY_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "draft/child.txt",
        b"legacy directory child",
        CHILD_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &conflict_path,
        b"legacy replacement file",
        FILE_MODIFIED_AT,
    )
    .await;
}

#[tokio::test]
async fn concurrent_replacement_sources_use_the_matching_barrier_root() {
    let fixture = PathKindFixture::new();
    let directory_root = fixture
        .directory_root("draft", "old.txt", b"old child")
        .await;
    let empty_base = fixture.tree.put_directory(Vec::new()).await.unwrap();
    let barrier_paths = BTreeSet::from(["draft".to_string()]);
    let replacement_b = fixture
        .file_root("draft", b"replacement B", FILE_MODIFIED_AT)
        .await;
    let replacement_b =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
            &fixture.tree,
            replacement_b,
            None,
            Some(&empty_base),
            200,
            None,
            Some(&barrier_paths),
        )
        .await
        .unwrap();
    let replacement_b = crate::indexer::layer_path_kind_replacements(
        &fixture.tree,
        replacement_b,
        &BTreeMap::from([("draft".to_string(), 200)]),
    )
    .await
    .unwrap();
    let replacement_c = fixture
        .file_root("draft", b"replacement C", FILE_MODIFIED_AT)
        .await;
    let replacement_c =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
            &fixture.tree,
            replacement_c,
            None,
            Some(&empty_base),
            300,
            None,
            Some(&barrier_paths),
        )
        .await
        .unwrap();
    let replacement_c = crate::indexer::layer_path_kind_replacements(
        &fixture.tree,
        replacement_c,
        &BTreeMap::from([("draft".to_string(), 300)]),
    )
    .await
    .unwrap();
    let directory_ref = root_ref(&directory_root, &fixture.owner_app_key, 1, 100, None);
    let newer_file_ref = root_ref(
        &replacement_b,
        PEER_APP_KEY,
        1,
        999,
        Some((&fixture.owner_app_key, 1, &directory_root)),
    );
    let newer_barrier_ref = root_ref(
        &replacement_c,
        SECOND_PEER_APP_KEY,
        1,
        998,
        Some((&fixture.owner_app_key, 1, &directory_root)),
    );
    let mut config = fixture.config(directory_ref, newer_file_ref);
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive
        .app_key_roots
        .insert(SECOND_PEER_APP_KEY.to_string(), newer_barrier_ref);
    config.upsert_drive(drive);

    let merged = primary_merged_root(&fixture.tree, &config).await.unwrap();
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        "draft",
        b"replacement B",
        FILE_MODIFIED_AT,
    )
    .await;
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("draft (conflict from {SECOND_PEER_APP_KEY})"),
        b"replacement C",
        FILE_MODIFIED_AT,
    )
    .await;
    let conflict_dir = format!("draft (conflict from {})", fixture.owner_app_key);
    assert_file(
        &fixture.tree,
        &merged.root_cid,
        &format!("{conflict_dir}/old.txt"),
        b"old child",
        CHILD_MODIFIED_AT,
    )
    .await;
    let empty = fixture
        .tree
        .resolve(&merged.root_cid, &format!("{conflict_dir}/nested/empty"))
        .await
        .unwrap()
        .expect("the matching losing replacement barrier preserves empty directories too");
    assert!(fixture.tree.is_dir(&empty).await.unwrap());
}

struct PathKindFixture {
    tree: HashTree<MemoryStore>,
    profile: Profile,
    owner_app_key: String,
}

async fn without_path_kind_roles(tree: &HashTree<MemoryStore>, root: &Cid) -> Cid {
    tree.remove_entry(root, &[crate::merge::META_DIR], "path-kind-replacements")
        .await
        .unwrap()
}

impl PathKindFixture {
    fn new() -> Self {
        let config_dir = tempdir().unwrap();
        let mut profile = Profile::create(config_dir.path(), Some("owner".into())).unwrap();
        profile
            .approve_app_key(PEER_APP_KEY, Some("peer".into()))
            .unwrap();
        profile
            .approve_app_key(SECOND_PEER_APP_KEY, Some("second peer".into()))
            .unwrap();
        let owner_app_key = profile.state.app_key_pubkey.clone();
        Self {
            tree: HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public()),
            profile,
            owner_app_key,
        }
    }

    async fn file_root(&self, name: &str, bytes: &[u8], modified_at: i64) -> Cid {
        let (file, size) = self.tree.put_file(bytes).await.unwrap();
        self.tree
            .put_directory(vec![file_entry(name, &file, size, bytes, modified_at)])
            .await
            .unwrap()
    }

    async fn directory_root(&self, name: &str, child_name: &str, bytes: &[u8]) -> Cid {
        let (child, child_size) = self.tree.put_file(bytes).await.unwrap();
        let empty = self.tree.put_directory(Vec::new()).await.unwrap();
        let nested = self
            .tree
            .put_directory(vec![
                DirEntry::from_cid("empty", &empty).with_link_type(LinkType::Dir),
            ])
            .await
            .unwrap();
        let directory = self
            .tree
            .put_directory(vec![
                file_entry(child_name, &child, child_size, bytes, CHILD_MODIFIED_AT),
                DirEntry::from_cid("nested", &nested).with_link_type(LinkType::Dir),
            ])
            .await
            .unwrap();
        self.tree
            .put_directory(vec![
                DirEntry::from_cid(name, &directory)
                    .with_link_type(LinkType::Dir)
                    .with_meta(modified_at_meta(DIRECTORY_MODIFIED_AT)),
            ])
            .await
            .unwrap()
    }

    fn config(&self, owner_root: AppKeyRootRef, peer_root: AppKeyRootRef) -> AppConfig {
        let mut config = AppConfig {
            profile: Some(self.profile.state.clone()),
            ..AppConfig::default()
        };
        let mut drive = Drive::primary(self.profile.state.root_scope_id());
        drive
            .app_key_roots
            .insert(self.owner_app_key.clone(), owner_root);
        drive
            .app_key_roots
            .insert(PEER_APP_KEY.to_string(), peer_root);
        config.upsert_drive(drive);
        config
    }
}

fn root_ref(
    root: &Cid,
    app_key_pubkey: &str,
    app_key_seq: u64,
    created_at: i64,
    observed: Option<(&str, u64, &Cid)>,
) -> AppKeyRootRef {
    let mut observations = BTreeMap::new();
    if let Some((observed_app_key, observed_seq, observed_root)) = observed {
        observations.insert(
            observed_app_key.to_string(),
            RootObservation {
                app_key_seq: observed_seq,
                root_cid: observed_root.to_string(),
            },
        );
    }
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_string(),
        app_key_seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: observations,
        created_at,
    };
    AppKeyRootRef::from_meta(root.to_string(), created_at, &meta)
}

fn file_entry(name: &str, cid: &Cid, size: u64, bytes: &[u8], modified_at: i64) -> DirEntry {
    let mut meta = modified_at_meta(modified_at);
    meta.insert(
        crate::merge::WHOLE_FILE_HASH_META_KEY.to_string(),
        serde_json::json!(hashtree_core::to_hex(&sha256(bytes))),
    );
    DirEntry::from_cid(name, cid)
        .with_size(size)
        .with_link_type(LinkType::File)
        .with_meta(meta)
}

fn modified_at_meta(modified_at: i64) -> DirectoryMeta {
    std::collections::HashMap::from([(
        MODIFIED_AT_META_KEY.to_string(),
        serde_json::json!(modified_at),
    )])
}

async fn assert_file(
    tree: &HashTree<MemoryStore>,
    root: &Cid,
    path: &str,
    expected: &[u8],
    modified_at: i64,
) {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    let parent = if parent.is_empty() {
        root.clone()
    } else {
        tree.resolve(root, parent)
            .await
            .unwrap()
            .expect("file parent exists")
    };
    let entry = tree
        .list_directory(&parent)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.name == name)
        .expect("file exists");
    assert_ne!(entry.link_type, LinkType::Dir);
    let cid = Cid {
        hash: entry.hash,
        key: entry.key,
    };
    assert_eq!(tree.get(&cid, None).await.unwrap().unwrap(), expected);
    assert_eq!(
        entry
            .meta
            .as_ref()
            .and_then(|meta| meta.get(MODIFIED_AT_META_KEY))
            .and_then(serde_json::Value::as_i64),
        Some(modified_at)
    );
    assert_eq!(
        entry
            .meta
            .as_ref()
            .and_then(|meta| meta.get(crate::merge::WHOLE_FILE_HASH_META_KEY))
            .and_then(serde_json::Value::as_str),
        Some(hashtree_core::to_hex(&sha256(expected)).as_str())
    );
}

async fn assert_directory_meta(
    tree: &HashTree<MemoryStore>,
    root: &Cid,
    path: &str,
    modified_at: i64,
) {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    let parent = if parent.is_empty() {
        root.clone()
    } else {
        tree.resolve(root, parent)
            .await
            .unwrap()
            .expect("directory parent exists")
    };
    let entry = tree
        .list_directory(&parent)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.name == name)
        .expect("directory exists");
    assert_eq!(entry.link_type, LinkType::Dir);
    assert_eq!(
        entry
            .meta
            .as_ref()
            .and_then(|meta| meta.get(MODIFIED_AT_META_KEY))
            .and_then(serde_json::Value::as_i64),
        Some(modified_at)
    );
}
