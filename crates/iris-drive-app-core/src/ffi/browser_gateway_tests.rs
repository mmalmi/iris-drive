use super::native_browser_gateway_status_value_port;

#[test]
fn native_browser_gateway_status_ignores_non_running_or_malformed_status() {
    assert_eq!(
        native_browser_gateway_status_value_port(&serde_json::json!({
            "running": false,
            "state": "starting",
            "bind": "127.0.0.1:17321",
        })),
        None
    );
    assert_eq!(
        native_browser_gateway_status_value_port(&serde_json::json!({
            "running": true,
            "state": "running",
            "bind": "127.0.0.1",
        })),
        None
    );
}

#[test]
fn native_browser_gateway_status_extracts_running_bind_port() {
    assert_eq!(
        native_browser_gateway_status_value_port(&serde_json::json!({
            "running": true,
            "state": "running",
            "bind": "127.0.0.1:52887",
        })),
        Some(52887)
    );
}
