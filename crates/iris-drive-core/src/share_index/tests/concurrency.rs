//! One device takes in lists from several places at once: the daemon, the
//! app and a one-shot sync all write the same file.
use super::*;
use crate::share_index::{ShareIndexApply, apply_share_index_event};

/// A laptop's config directory and profile, with a phone linked to it.
fn laptop_with_phone() -> (tempfile::TempDir, Profile, Profile) {
    let laptop_dir = tempdir().unwrap();
    let phone_dir = tempdir().unwrap();
    let mut laptop = Profile::create(laptop_dir.path(), Some("Laptop".into())).unwrap();
    let phone = Profile::link_to_profile(
        phone_dir.path(),
        laptop.state.profile_id,
        laptop.state.app_key_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    laptop
        .approve_app_key(&phone.state.app_key_pubkey, Some("Phone".into()))
        .unwrap();
    AppConfig {
        profile: Some(laptop.state.clone()),
        ..AppConfig::default()
    }
    .save(config_path_in(laptop_dir.path()))
    .unwrap();
    (laptop_dir, laptop, phone)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lists_arriving_at_once_on_one_device_all_land() {
    let (laptop_dir, laptop, phone) = laptop_with_phone();
    let scope = laptop.state.root_scope_id();
    let recipients = [laptop.state.app_key_pubkey.clone()];
    let events = (0..24_u32)
        .map(|part| {
            build_share_index_event(
                phone.app_key.keys(),
                &scope,
                part,
                [&listed(
                    phone.app_key.keys(),
                    &format!("Folder {part}"),
                    100,
                )],
                &recipients,
                1_000,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let mut applying = tokio::task::JoinSet::new();
    for event in events.clone() {
        let dir = laptop_dir.path().to_path_buf();
        applying.spawn(async move { apply_share_index_event(&dir, &event).await.unwrap() });
    }
    while let Some(outcome) = applying.join_next().await {
        assert_eq!(outcome.unwrap(), ShareIndexApply::Applied);
    }

    // None wrote over another: every folder is in the list, waiting to be added.
    let index = ShareIndex::load(laptop_dir.path()).unwrap();
    assert_eq!(index.entries.len(), events.len());
    assert_eq!(index.wanted().count(), events.len());
    for event in &events {
        assert_eq!(
            apply_share_index_event(laptop_dir.path(), event)
                .await
                .unwrap(),
            ShareIndexApply::Current
        );
    }
}

#[tokio::test]
async fn a_list_is_taken_only_from_the_profiles_own_devices() {
    let (laptop_dir, laptop, phone) = laptop_with_phone();
    let scope = laptop.state.root_scope_id();
    let recipients = [laptop.state.app_key_pubkey.clone()];
    let stranger = Keys::generate();
    let folder = listed(&stranger, "Planted", 100);

    // Signed by a key the profile never linked, for this profile.
    let planted =
        build_share_index_event(&stranger, &scope, 0, [&folder], &recipients, 1_000).unwrap();
    assert_eq!(
        apply_share_index_event(laptop_dir.path(), &planted)
            .await
            .unwrap(),
        ShareIndexApply::Unauthorized
    );
    // Signed by the phone, but for another profile's list.
    let elsewhere = build_share_index_event(
        phone.app_key.keys(),
        &NostrIdentityId::new_v4().to_string(),
        0,
        [&folder],
        &recipients,
        1_000,
    )
    .unwrap();
    assert_eq!(
        apply_share_index_event(laptop_dir.path(), &elsewhere)
            .await
            .unwrap(),
        ShareIndexApply::NotOurs
    );
    // Signed by the phone before the laptop was among its devices.
    let early =
        build_share_index_event(phone.app_key.keys(), &scope, 0, [&folder], &[], 1_000).unwrap();
    assert_eq!(
        apply_share_index_event(laptop_dir.path(), &early)
            .await
            .unwrap(),
        ShareIndexApply::KeyUnavailable
    );

    assert_eq!(
        ShareIndex::load(laptop_dir.path()).unwrap().entries.len(),
        0
    );
}
