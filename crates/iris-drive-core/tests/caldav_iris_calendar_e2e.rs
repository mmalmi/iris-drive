use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use iris_drive_core::{
    AppConfig, Daemon, Drive, GatewayBind, GatewayServer, Profile,
    app_key_summary::pubkey_npub,
    calendar::{CalendarData, load_calendar_data, save_calendar_data},
    gateway::local_caldav_url_for_identity,
    paths::config_path_in,
};
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_caldav_client_round_trips_with_iris_calendar_source_json() {
    let Some(seed_json) = env_path("IRIS_DRIVE_CALDAV_E2E_SEED_JSON") else {
        eprintln!("skipping CalDAV real-client e2e; run scripts/check-caldav-iris-calendar-e2e.sh");
        return;
    };
    let output_json = env_path("IRIS_DRIVE_CALDAV_E2E_OUTPUT_JSON")
        .expect("IRIS_DRIVE_CALDAV_E2E_OUTPUT_JSON must be set");
    let client_script = env_path("IRIS_DRIVE_CALDAV_E2E_CLIENT_SCRIPT")
        .expect("IRIS_DRIVE_CALDAV_E2E_CLIENT_SCRIPT must be set");
    let python = env::var_os("IRIS_DRIVE_CALDAV_E2E_PYTHON")
        .map_or_else(|| PathBuf::from("python3"), PathBuf::from);
    let app_title = env::var("IRIS_DRIVE_CALDAV_E2E_APP_TITLE")
        .unwrap_or_else(|_| "Iris Calendar source event".to_string());
    let caldav_title = env::var("IRIS_DRIVE_CALDAV_E2E_CLIENT_TITLE")
        .unwrap_or_else(|_| "CalDAV client event".to_string());

    let config_dir = tempdir().unwrap();
    let identity = init_account_config(config_dir.path());
    let seed_data = seed_calendar_data(&seed_json, &app_title);
    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    save_calendar_data(&mut daemon, &seed_data).await.unwrap();

    let server = GatewayServer::bind_with_tree(
        config_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let caldav_url = local_caldav_url_for_identity(server.local_addr().port(), &identity)
        .replace("http://localhost:", "http://127.0.0.1:");

    let client_result = Command::new(&python)
        .arg(&client_script)
        .env("IRIS_CALDAV_E2E_URL", &caldav_url)
        .env("IRIS_CALDAV_E2E_EXPECTED_TITLE", &app_title)
        .env("IRIS_CALDAV_E2E_CLIENT_TITLE", &caldav_title)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", python.display()));

    if !client_result.status.success() {
        let _ = server.shutdown().await;
        panic!(
            "CalDAV client failed\nstatus: {}\nstdout: {}\nstderr: {}",
            client_result.status,
            String::from_utf8_lossy(&client_result.stdout),
            String::from_utf8_lossy(&client_result.stderr)
        );
    }

    let daemon = Daemon::open(config_dir.path()).unwrap();
    let data = load_calendar_data(daemon.tree(), daemon.config(), &identity)
        .await
        .unwrap();
    assert_event_title(&data, &app_title);
    assert_event_title(&data, &caldav_title);
    write_calendar_json(&output_json, &data);

    server.shutdown().await.unwrap();
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).map(PathBuf::from)
}

fn init_account_config(dir: &Path) -> String {
    let account = Profile::create(dir, Some("caldav-e2e".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(dir)).unwrap();
    pubkey_npub(&account.state.app_key_pubkey)
}

fn seed_calendar_data(path: &Path, expected_title: &str) -> CalendarData {
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!(
            "failed to read seed calendar JSON {}: {error}",
            path.display()
        )
    });
    let data: CalendarData = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "seed calendar JSON must match Iris Calendar schema {}: {error}",
            path.display()
        )
    });
    assert_event_title(&data, expected_title);
    data
}

fn assert_event_title(data: &CalendarData, title: &str) {
    assert!(
        data.events.iter().any(|event| event.title == title),
        "calendar data did not contain event title {title:?}; titles: {:?}",
        data.events
            .iter()
            .map(|event| event.title.as_str())
            .collect::<Vec<_>>()
    );
}

fn write_calendar_json(path: &Path, data: &CalendarData) {
    let mut bytes = serde_json::to_vec_pretty(data).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}
