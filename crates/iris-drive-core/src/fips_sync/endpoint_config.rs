use std::net::SocketAddrV4;

use hashtree_fips_transport::{
    BoundFipsEndpoint, FipsEndpointOptions, FipsTransportError, bind_fips_endpoint,
    bind_fips_endpoint_at_local_rendezvous,
};

const LOCAL_RENDEZVOUS_ADDR_ENV: &str = "IRIS_DRIVE_FIPS_LOCAL_RENDEZVOUS_ADDR";

pub(super) async fn bind_drive_fips_endpoint(
    options: FipsEndpointOptions,
) -> Result<BoundFipsEndpoint, FipsTransportError> {
    let Some(value) = std::env::var(LOCAL_RENDEZVOUS_ADDR_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Box::pin(bind_fips_endpoint(options)).await;
    };
    let address = parse_local_rendezvous_addr(&value)?;
    Box::pin(bind_fips_endpoint_at_local_rendezvous(options, address)).await
}

fn parse_local_rendezvous_addr(value: &str) -> Result<SocketAddrV4, FipsTransportError> {
    let address = value.trim().parse::<SocketAddrV4>().map_err(|error| {
        FipsTransportError::Endpoint(format!(
            "{LOCAL_RENDEZVOUS_ADDR_ENV} must be an IPv4 loopback address: {error}"
        ))
    })?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(FipsTransportError::Endpoint(format!(
            "{LOCAL_RENDEZVOUS_ADDR_ENV} must be a non-zero IPv4 loopback address"
        )));
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_rendezvous_override_requires_nonzero_ipv4_loopback() {
        assert_eq!(
            parse_local_rendezvous_addr("127.0.0.1:32112").unwrap(),
            "127.0.0.1:32112".parse::<SocketAddrV4>().unwrap()
        );
        assert!(parse_local_rendezvous_addr("0.0.0.0:32112").is_err());
        assert!(parse_local_rendezvous_addr("127.0.0.1:0").is_err());
        assert!(parse_local_rendezvous_addr("[::1]:32112").is_err());
    }
}
