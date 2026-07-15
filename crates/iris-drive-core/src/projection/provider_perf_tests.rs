#[allow(clippy::wildcard_imports)]
use super::*;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::indexer::index_dir_with_history_and_meta;
use crate::merge::walk_app_key_tree;
use crate::profile::Profile;
use crate::root_meta::DriveRootMeta;
use hashtree_core::{HashTreeConfig, MemoryStore};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn primary_merged_root_from_view_uses_supplied_projection() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("provider-list-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let first_source = tempdir().unwrap();
    std::fs::write(first_source.path().join("one.txt"), b"one").unwrap();
    let first_meta = test_meta(&account.state.app_key_pubkey, 1);
    let first_root =
        index_dir_with_history_and_meta(&tree, first_source.path(), None, 1, Some(&first_meta))
            .await
            .unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    upsert_root(
        &mut config,
        &account.state,
        first_root.to_string(),
        &first_meta,
    );
    let first_view = primary_merged_view(&tree, &config).await.unwrap();

    let second_source = tempdir().unwrap();
    std::fs::write(second_source.path().join("one.txt"), b"one").unwrap();
    std::fs::write(second_source.path().join("two.txt"), b"two").unwrap();
    let second_meta = test_meta(&account.state.app_key_pubkey, 2);
    let second_root =
        index_dir_with_history_and_meta(&tree, second_source.path(), None, 2, Some(&second_meta))
            .await
            .unwrap();
    upsert_root(
        &mut config,
        &account.state,
        second_root.to_string(),
        &second_meta,
    );

    let supplied = primary_merged_root_from_view(&tree, &config, &first_view)
        .await
        .unwrap();
    assert_eq!(
        visible_paths(&tree, &supplied.root_cid).await,
        vec!["one.txt"]
    );
    assert_eq!(supplied.file_count, 1);

    let recomputed = primary_merged_root(&tree, &config).await.unwrap();
    assert_eq!(
        visible_paths(&tree, &recomputed.root_cid).await,
        vec!["one.txt", "two.txt"]
    );
}

fn test_meta(app_key_pubkey: &str, seq: u64) -> DriveRootMeta {
    DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_owned(),
        app_key_seq: seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: i64::try_from(seq).unwrap(),
    }
}

fn upsert_root(
    config: &mut AppConfig,
    account: &crate::profile::ProfileState,
    root_cid: String,
    meta: &DriveRootMeta,
) {
    let mut drive = config
        .drive(PRIMARY_DRIVE_ID)
        .cloned()
        .unwrap_or_else(|| Drive::primary(account.root_scope_id()));
    drive.app_key_roots.insert(
        account.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(root_cid, i64::try_from(meta.app_key_seq).unwrap(), meta),
    );
    config.upsert_drive(drive);
}

async fn visible_paths<S: hashtree_core::Store>(tree: &HashTree<S>, root: &Cid) -> Vec<String> {
    let (files, _) = walk_app_key_tree(tree, root).await.unwrap();
    files.into_iter().map(|entry| entry.path).collect()
}
