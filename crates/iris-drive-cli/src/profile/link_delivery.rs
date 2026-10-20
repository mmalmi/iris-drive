use anyhow::Result;

type AppKeyLinkTransportAttempt<T> = std::result::Result<T, String>;

pub(super) async fn attempt_app_key_link_delivery<FipsOutput, RelayOutput>(
    fips_send: Option<impl std::future::Future<Output = Result<FipsOutput>>>,
    relay_publish: impl std::future::Future<Output = Result<RelayOutput>>,
    fips_timeout: std::time::Duration,
    relay_timeout: std::time::Duration,
) -> (
    Option<AppKeyLinkTransportAttempt<FipsOutput>>,
    AppKeyLinkTransportAttempt<RelayOutput>,
) {
    let fips = match fips_send {
        Some(send) => Some(match tokio::time::timeout(fips_timeout, send).await {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("timed out sending approval over FIPS".to_string()),
        }),
        None => None,
    };
    let relay = match tokio::time::timeout(relay_timeout, relay_publish).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("timed out publishing approval over relays".to_string()),
    };
    (fips, relay)
}
