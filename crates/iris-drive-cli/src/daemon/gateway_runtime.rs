async fn start_daemon_browser_gateway(
    config_dir: &Path,
    embedded_hashtree: &EmbeddedHashtreeHost,
    gateway_port: u16,
) -> Result<GatewayServer> {
    let daemon = Daemon::open(config_dir).context("opening daemon for browser gateway")?;
    GatewayServer::bind_with_tree_and_htree_daemon(
        config_dir,
        daemon.tree_handle(),
        embedded_hashtree.status().base_url.clone(),
        GatewayBind::loopback_v4(gateway_port),
    )
    .await
    .context("starting browser gateway")
}

fn stopped_browser_gateway_status(
    embedded_hashtree_requested: bool,
    gateway_disabled_by: Option<&'static str>,
    gateway_error: Option<&str>,
    gateway_port: u16,
) -> Value {
    json!({
        "enabled": gateway_error.is_some(),
        "requested": embedded_hashtree_requested,
        "running": false,
        "disabled_by": gateway_disabled_by,
        "error": gateway_error,
        "host": iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
        "port": gateway_port,
    })
}
