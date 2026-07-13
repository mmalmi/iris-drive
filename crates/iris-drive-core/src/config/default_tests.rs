use super::{AppConfig, CONFIG_SCHEMA_VERSION};

#[test]
fn defaults_enable_native_services() {
    let config = AppConfig::default();
    assert!(config.local_nhash_resolver_enabled);
    assert!(config.launch_on_startup);
    assert!(config.sync_enabled);
}

#[test]
fn missing_native_service_fields_load_enabled() {
    let raw = format!("schema_version = {CONFIG_SCHEMA_VERSION}\n");
    let config: AppConfig = toml::from_str(&raw).unwrap();
    assert!(config.local_nhash_resolver_enabled);
    assert!(config.launch_on_startup);
    assert!(config.sync_enabled);
}
