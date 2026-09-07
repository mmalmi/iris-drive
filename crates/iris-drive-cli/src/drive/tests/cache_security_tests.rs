use super::*;

#[cfg(unix)]
#[tokio::test]
async fn cache_hydration_rejects_symlinks_without_overwriting_external_files() {
    use std::os::unix::fs::symlink;
    use std::sync::Arc;

    for symlink_is_parent in [false, true] {
        let storage = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external_file = outside.path().join("private.txt");
        std::fs::write(&external_file, b"keep private data").unwrap();
        let tree = Arc::new(HashTree::new(hashtree_core::HashTreeConfig::new(Arc::new(
            FsBlobStore::new(storage.path()).unwrap(),
        ))));
        let provider = HashTreeProviderFs::fresh(tree.clone()).await.unwrap();
        let path = if symlink_is_parent {
            symlink(outside.path(), target.path().join("linked")).unwrap();
            provider.create_dir(&String::new(), "linked").await.unwrap();
            "linked/private.txt"
        } else {
            symlink(&external_file, target.path().join("private.txt")).unwrap();
            "private.txt"
        };
        write_provider_file(&provider, path, b"remote replacement")
            .await
            .unwrap();
        let entries = provider_entries(&tree, &provider.current_root().await, &BTreeMap::new())
            .await
            .unwrap();

        let result = hydrate_provider_cache(&provider, &entries, target.path()).await;

        assert!(result.is_err(), "symlink must be rejected");
        assert_eq!(std::fs::read(&external_file).unwrap(), b"keep private data");
    }
}

#[tokio::test]
async fn cache_hydration_replaces_hardlinks_without_overwriting_external_files() {
    use std::sync::Arc;

    let storage = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let external_file = outside.path().join("private.txt");
    std::fs::write(&external_file, b"keep private data").unwrap();
    std::fs::hard_link(&external_file, target.path().join("private.txt")).unwrap();
    let tree = Arc::new(HashTree::new(hashtree_core::HashTreeConfig::new(Arc::new(
        FsBlobStore::new(storage.path()).unwrap(),
    ))));
    let provider = HashTreeProviderFs::fresh(tree.clone()).await.unwrap();
    write_provider_file(&provider, "private.txt", b"remote replacement")
        .await
        .unwrap();
    let entries = provider_entries(&tree, &provider.current_root().await, &BTreeMap::new())
        .await
        .unwrap();

    hydrate_provider_cache(&provider, &entries, target.path())
        .await
        .unwrap();

    assert_eq!(std::fs::read(&external_file).unwrap(), b"keep private data");
    assert_eq!(
        std::fs::read(target.path().join("private.txt")).unwrap(),
        b"remote replacement"
    );
}
