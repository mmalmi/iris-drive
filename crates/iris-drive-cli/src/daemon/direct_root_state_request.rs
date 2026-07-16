const DIRECT_ROOT_STATE_REQUEST_MIN_INTERVAL_SECS: u64 = 10;
const DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS: u64 = 1;

static DIRECT_ROOT_STATE_REQUEST_THROTTLE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<String, std::time::Instant>>,
> = std::sync::OnceLock::new();

async fn request_latest_direct_root_state(
    config: &AppConfig,
    fips_blocks: Option<&FsFipsBlockSync>,
    projection_event: &'static str,
    bypass_throttle: bool,
) {
    let Some(sync) = fips_blocks else {
        return;
    };
    let Some(root_scope_id) = config.profile.as_ref().map(ProfileState::root_scope_id) else {
        return;
    };
    let visible_peers = sync
        .connected_peer_ids()
        .await
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if !should_publish_direct_root_state_request(
        &root_scope_id,
        visible_peers.iter().map(String::as_str),
        bypass_throttle,
    ) {
        println!(
            "{}",
            json!({
                "event": "direct_root_state_request_throttled",
                "trigger": projection_event,
                "root_scope_id": root_scope_id,
                "visible_peers": visible_peers.len(),
            })
        );
        return;
    }
    let bytes = match iris_drive_core::encode_direct_root_state_request_frame(&root_scope_id) {
        Ok(bytes) => bytes,
        Err(error) => {
            println!(
                "{}",
                json!({
                    "event": "direct_root_state_request_error",
                    "trigger": projection_event,
                    "error": format!("{error:#}"),
                })
            );
            return;
        }
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS),
        crate::publish::send_direct_root_app_message_to_authorized_peers(sync, bytes),
    )
    .await
    {
        Ok(send_stats) => println!(
            "{}",
            json!({
                "event": "direct_root_state_request_publish",
                "trigger": projection_event,
                "root_scope_id": root_scope_id.clone(),
                "selected_peers": send_stats.selected_peers,
                "visible_peers": visible_peers.len(),
                "sent_peers": send_stats.sent_peers,
                "failed_peers": send_stats.failed_peers,
            })
        ),
        Err(_) => println!(
            "{}",
            json!({
                "event": "direct_root_state_request_publish_timeout",
                "trigger": projection_event,
                "root_scope_id": root_scope_id.clone(),
                "visible_peers": visible_peers.len(),
                "timeout_secs": DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS,
            })
        ),
    }
}

fn should_publish_direct_root_state_request<'a>(
    root_scope_id: &str,
    visible_peers: impl IntoIterator<Item = &'a str>,
    bypass_throttle: bool,
) -> bool {
    if bypass_throttle {
        return true;
    }
    let throttle = DIRECT_ROOT_STATE_REQUEST_THROTTLE
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let Ok(mut throttle) = throttle.lock() else {
        return true;
    };
    let now = std::time::Instant::now();
    let mut throttle_keys = visible_peers
        .into_iter()
        .filter(|peer| !peer.is_empty())
        .map(|peer| format!("request:{peer}:{root_scope_id}"))
        .collect::<Vec<_>>();
    if throttle_keys.is_empty() {
        throttle_keys.push(format!("request:*:{root_scope_id}"));
    }
    let interval = std::time::Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_MIN_INTERVAL_SECS);
    if throttle_keys.iter().all(|key| {
        throttle
            .get(key)
            .is_some_and(|last| now.duration_since(*last) < interval)
    }) {
        return false;
    }
    for key in throttle_keys {
        throttle.insert(key, now);
    }
    true
}
