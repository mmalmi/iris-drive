use std::sync::Arc;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore, Store};
use serde::Deserialize;

use super::tree::{TrashTree, entry_cid};
use super::*;
use crate::daemon::Daemon;

#[derive(Deserialize)]
struct Fixture {
    item: TrashItem,
    content_utf8: String,
    marker: TrashRemoval,
}

#[tokio::test]
async fn private_trash_matches_cross_client_fixture_and_consumes_stale_items() {
    let input = include_str!("private-trash-v1.json");
    let Fixture {
        item,
        content_utf8,
        marker,
    } = serde_json::from_str(input).unwrap();
    let wire: serde_json::Value = serde_json::from_str(input).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())));
    let (payload, size) = tree.put(content_utf8.as_bytes()).await.unwrap();
    let name = item.path.last().unwrap();
    let content = DirEntry::from_cid(name, &payload).with_size(size);
    let holding = tree.put_directory(vec![content]).await.unwrap();
    let original = tree
        .list_directory_required(&holding)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut snapshot = TrashTree::default();
    snapshot
        .insert(&tree, item.clone(), original)
        .await
        .unwrap();
    let root = snapshot.write(&tree).await.unwrap();
    let blocks = fixture_blocks(&tree, &root).await;
    println!(
        "{}",
        serde_json::json!({
            "native_trash_fixture": {
                "root_hash": hex::encode(root.hash),
                "root_key": hex::encode(root.key.unwrap()),
                "blocks": blocks,
            }
        })
    );

    let item_path = format!("{}/item.json", item.id);
    let note = tree.resolve(&root, &item_path).await.unwrap().unwrap();
    let item_bytes = tree.get(&note, None).await.unwrap().unwrap();
    let encoded = String::from_utf8(item_bytes).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
        wire["item"]
    );
    assert_eq!(
        encoded,
        format!(
            r#"{{"v":1,"id":"{}","account":"{}","source_scope":"{}","tree_name":"main","path":["Notes","café.txt"],"deleted_at":1800000000}}"#,
            item.id, item.account, item.source_scope
        )
    );

    // Build the incoming root directly from the web metadata and payload bytes.
    // This also exercises the reader with a different JSON key order.
    let (web_note, size) = tree
        .put(serde_json::to_string(&wire["item"]).unwrap().as_bytes())
        .await
        .unwrap();
    let web_slot = tree
        .put_directory(vec![
            DirEntry::from_cid(ITEM_FILE, &web_note).with_size(size),
            DirEntry::from_cid(CONTENT_DIR, &holding).with_link_type(LinkType::Dir),
        ])
        .await
        .unwrap();
    let web_root = tree
        .put_directory(vec![
            DirEntry::from_cid(&item.id, &web_slot).with_link_type(LinkType::Dir),
        ])
        .await
        .unwrap();
    let imported = TrashTree::read(&tree, item.account, &[root.clone(), web_root.clone()])
        .await
        .unwrap();
    let stored = &imported.items[&item.id];
    assert_eq!(stored.item, item);
    assert_eq!(entry_cid(&stored.content), payload);
    assert_eq!(
        tree.get(&payload, None).await.unwrap().unwrap(),
        content_utf8.as_bytes()
    );
    assert!(!item.expired(item.deleted_at + RETENTION_SECS - 1));
    assert!(item.expired(item.deleted_at + RETENTION_SECS));

    snapshot.mark_removed(&tree, &marker).await.unwrap();
    let removed = snapshot.write(&tree).await.unwrap();
    let marker_path = format!("{}/{}", REMOVED_DIR, item.id);
    let note = tree.resolve(&removed, &marker_path).await.unwrap().unwrap();
    let bytes = tree.get(&note, None).await.unwrap().unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!(r#"{{"v":1,"id":"{}","removed_at":1800000123}}"#, item.id)
    );
    let reconciled = TrashTree::read(&tree, item.account, &[web_root, removed, root])
        .await
        .unwrap();
    assert!(reconciled.was_removed(&item.id));
    assert!(reconciled.items.is_empty());
    assert!(
        TrashTree::read(
            &tree,
            item.account,
            &[Cid {
                hash: [241; 32],
                key: holding.key,
            }]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn private_trash_opens_the_encrypted_tree_emitted_by_web() {
    #[derive(Deserialize)]
    struct EncryptedFixture {
        root_hash: String,
        root_key: String,
        blocks: std::collections::BTreeMap<String, String>,
    }
    let expected: Fixture = serde_json::from_str(include_str!("private-trash-v1.json")).unwrap();
    let fixture: EncryptedFixture =
        serde_json::from_str(include_str!("web-trash-tree-v1.json")).unwrap();
    let store = Arc::new(MemoryStore::new());
    for (hash, bytes) in fixture.blocks {
        let hash = hashtree_core::from_hex(&hash).unwrap();
        let bytes = hex::decode(bytes).unwrap();
        assert_eq!(hashtree_core::sha256(&bytes), hash);
        store.put(hash, bytes).await.unwrap();
    }
    let tree = HashTree::new(HashTreeConfig::new(store));
    let web_root = Cid::parse(&format!("{}:{}", fixture.root_hash, fixture.root_key)).unwrap();
    let snapshot = TrashTree::read(&tree, expected.item.account, &[web_root])
        .await
        .unwrap();
    let archived = &snapshot.items[&expected.item.id];
    assert_eq!(archived.item, expected.item);
    let bytes = tree
        .get(&entry_cid(&archived.content), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, expected.content_utf8.as_bytes());
}

#[tokio::test]
async fn private_trash_restore_preserves_payload_and_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let profile = crate::Profile::create(dir.path(), None).unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    config
        .save(crate::paths::config_path_in(dir.path()))
        .unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let (original, size) = daemon.tree().put(b"original\n").await.unwrap();
    let root = daemon
        .tree()
        .put_directory(vec![
            DirEntry::from_cid("notes.txt", &original).with_size(size),
        ])
        .await
        .unwrap();
    daemon.import_visible_root(root).await.unwrap();

    let path = vec!["notes.txt".to_string()];
    let item = trash(dir.path(), path.clone(), 1800000000).await.unwrap();
    assert_eq!(
        list(dir.path(), 1800000001).await.unwrap(),
        vec![item.clone()]
    );
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert!(
        daemon
            .tree()
            .resolve(&root, "notes.txt")
            .await
            .unwrap()
            .is_none()
    );
    let (replacement, size) = daemon.tree().put(b"a new entry\n").await.unwrap();
    let updated = daemon
        .tree()
        .set_entry(&root, &[], "notes.txt", &replacement, size, LinkType::Blob)
        .await
        .unwrap();
    daemon.import_visible_root(updated).await.unwrap();
    assert!(
        restore(dir.path(), &item.id, 1800000001)
            .await
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert_eq!(
        list(dir.path(), 1800000002).await.unwrap(),
        vec![item.clone()]
    );

    trash(dir.path(), path, 1800000002).await.unwrap();
    restore(dir.path(), &item.id, 1800000003).await.unwrap();
    restore(dir.path(), &item.id, 1800000004).await.unwrap();
    let daemon = Daemon::open(dir.path()).unwrap();
    let root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert_eq!(
        daemon.tree().resolve(&root, "notes.txt").await.unwrap(),
        Some(original)
    );
    assert!(
        list(dir.path(), 1800000004)
            .await
            .unwrap()
            .iter()
            .all(|live| live.id != item.id)
    );
}

async fn fixture_blocks(
    tree: &HashTree<MemoryStore>,
    root: &Cid,
) -> std::collections::BTreeMap<String, String> {
    let hashes = hashtree_core::diff::collect_hashes(tree, root, 4)
        .await
        .unwrap();
    let store = tree.get_store();
    let mut blocks = std::collections::BTreeMap::new();
    for hash in hashes {
        blocks.insert(
            hex::encode(hash),
            hex::encode(store.get(&hash).await.unwrap().unwrap()),
        );
    }
    blocks
}
