//! A folder shared on one device of a profile is shared on its other
//! devices, with real daemons that stay subscribed: nothing here syncs by
//! hand.
#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::{AppConfig, paths::config_path_in};

fn shares(config_dir: &Path) -> Vec<Value> {
    run_json(config_dir, &["shares", "list"])["shares"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The note that tells `device_dir`'s device it was let into a folder, once
/// the relay holds one: what Iris Drive web listens for.
async fn share_added_notice(
    relay: &LocalNostrRelay,
    device_dir: &Path,
) -> (iris_drive_core::share_added::ShareAddedNotice, String) {
    use nostr_sdk::JsonUtil;
    let device = iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(device_dir))
        .unwrap()
        .keys()
        .clone();
    tokio::time::timeout(WAIT_TIMEOUT, async {
        loop {
            for stored in relay.events().await {
                if let Ok(event) = nostr_sdk::Event::from_json(stored.to_string())
                    && let Some(notice) =
                        iris_drive_core::share_added::read_share_added_notice(&event, &device)
                {
                    return (notice, event.pubkey.to_hex());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the device let into the folder is told of it")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_share_a_folder_on_every_device_of_the_profile() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;
    let first = cluster.config_path(Client::Windows);
    let second = cluster.config_path(Client::Ubuntu);

    cluster
        .provider_write(Client::Windows, "Projects/Alpha/Plan.txt", b"Shared plan")
        .await;
    cluster
        .wait_for_provider_entry("Projects/Alpha/Plan.txt", "file", "folder on both devices")
        .await;

    // Shared on the first device: the second shares it too, in the same place.
    let created = run_json(first, &["shares", "create", "Projects/Alpha"]);
    let share_id = created["share"]["share_id"].as_str().unwrap().to_owned();
    cluster
        .wait_until("the second device shares the folder", || {
            let shares = shares(second);
            shares.len() == 1
                && shares[0]["share_id"] == share_id.as_str()
                && shares[0]["source_path"] == "Projects/Alpha"
                && shares[0]["can_admin"] == true
                && shares[0]["has_current_key_wrap"] == true
        })
        .await;
    // The first device's Drive let the second in, and told it so in the note
    // Iris Drive web listens for.
    let (told, teller) = share_added_notice(&cluster.relay, second).await;
    assert_eq!(told.share_id.to_string(), share_id);
    assert_eq!(told.name, "Alpha");
    let first_device = AppConfig::load_or_default(config_path_in(first))
        .unwrap()
        .profile
        .unwrap()
        .app_key_pubkey;
    assert_eq!(teller, first_device);
    cluster
        .wait_until("the second device has the shared folder's files", || {
            AppConfig::load_or_default(config_path_in(second)).is_ok_and(|config| {
                config
                    .shared_folders
                    .first()
                    .is_some_and(|folder| !folder.app_key_roots.is_empty())
            }) && cluster
                .provider_entry_kind(Client::Ubuntu, "Projects/Alpha/Plan.txt")
                .as_deref()
                == Some("file")
        })
        .await;

    // What the second device saves inside goes into the shared folder, not
    // only into its own Drive, and reaches the first.
    cluster
        .provider_write(
            Client::Ubuntu,
            "Projects/Alpha/Notes.txt",
            b"From the second",
        )
        .await;
    let second_device = AppConfig::load_or_default(config_path_in(second))
        .unwrap()
        .profile
        .unwrap()
        .app_key_pubkey;
    cluster
        .wait_until("the first device has the second's shared edit", || {
            AppConfig::load_or_default(config_path_in(first)).is_ok_and(|config| {
                config
                    .shared_folders
                    .first()
                    .is_some_and(|folder| folder.app_key_roots.contains_key(&second_device))
            }) && cluster
                .provider_entry_kind(Client::Windows, "Projects/Alpha/Notes.txt")
                .as_deref()
                == Some("file")
        })
        .await;

    // Stopped on the second device: the first stops sharing too, and both
    // keep the folder.
    run_json(second, &["shares", "delete", &share_id]);
    cluster
        .wait_until("the first device stopped sharing", || {
            shares(first).is_empty()
        })
        .await;
    cluster
        .wait_for_provider_entry(
            "Projects/Alpha/Plan.txt",
            "file",
            "folder kept on both devices",
        )
        .await;
}
