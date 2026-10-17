use super::*;
use crate::{
    profile::Profile,
    sharing::{ShareRecipient, ShareRole, ShareShortcut, create_shared_folder},
};

#[tokio::test]
async fn reader_projects_shared_root_instead_of_its_own_same_named_folder() {
    let owner_dir = tempfile::tempdir().unwrap();
    let reader_dir = tempfile::tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let reader = Profile::create(reader_dir.path(), Some("Reader".into())).unwrap();
    let mut folder = create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Picnic",
        "Picnic",
        None,
        vec![ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.app_key_pubkey.clone(),
            role: ShareRole::Reader,
            label: None,
            representative_npub_hint: None,
            display_name: None,
        }],
        10,
    )
    .unwrap();
    let store = std::sync::Arc::new(hashtree_core::MemoryStore::new());
    let tree = HashTree::new(hashtree_core::HashTreeConfig::new(store));
    let (file, _) = tree.put(b"Meet at noon").await.unwrap();
    let empty = tree.put_directory(vec![]).await.unwrap();
    let shared = tree
        .set_entry(&empty, &[], "Plan.txt", &file, 12, LinkType::Blob)
        .await
        .unwrap();
    folder.app_key_roots.insert(
        owner.state.app_key_pubkey.clone(),
        AppKeyRootRef::legacy(shared.to_string(), 10, 1),
    );
    let mut drive = Drive::primary(reader.state.root_scope_id());
    let local = tree
        .set_entry(&empty, &[], "Private.txt", &file, 12, LinkType::Blob)
        .await
        .unwrap();
    let local = tree
        .set_entry(&empty, &[], "Picnic", &local, 0, LinkType::Dir)
        .await
        .unwrap();
    drive.app_key_roots.insert(
        reader.state.app_key_pubkey.clone(),
        AppKeyRootRef::legacy(local.to_string(), 10, 1),
    );
    let config = AppConfig {
        profile: Some(reader.state),
        drives: vec![drive],
        share_shortcuts: vec![ShareShortcut::new(folder.share_id, "Team", "").unwrap()],
        shared_folders: vec![folder],
        ..AppConfig::default()
    };
    let merged = primary_merged_view(&tree, &config).await.unwrap();
    assert!(
        merged
            .view
            .files
            .iter()
            .any(|entry| entry.path == "Team/Plan.txt")
    );
    assert!(
        !merged
            .view
            .files
            .iter()
            .any(|entry| entry.path == "Team/Private.txt")
    );
    let root = primary_merged_root_from_view(&tree, &config, &merged)
        .await
        .unwrap();
    let cid = tree
        .resolve(&root.root_cid, "Team/Plan.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tree.get(&cid, None).await.unwrap().unwrap(),
        b"Meet at noon"
    );
}

#[test]
fn same_named_shares_have_distinct_canonical_mounts() {
    let owner = nostr_sdk::Keys::generate();
    let profile = crate::NostrIdentityId::new_v4();
    let first = create_shared_folder(&owner, profile, "First", "Photos", None, vec![], 1).unwrap();
    let second =
        create_shared_folder(&owner, profile, "Second", "Photos", None, vec![], 1).unwrap();
    let folders = vec![first.clone(), second.clone()];
    assert_ne!(
        crate::sharing::shared_with_me_mount_path(&first, &folders),
        crate::sharing::shared_with_me_mount_path(&second, &folders)
    );
    let views = crate::shared_folder_views(&folders, &[], &owner.public_key().to_hex());
    assert_ne!(views[0].shared_with_me_path, views[1].shared_with_me_path);
}
