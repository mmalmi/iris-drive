use std::collections::BTreeSet;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RelayConfigError {
    #[error("relay URL is required")]
    EmptyUrl,
}

pub fn normalize_relay_url(value: &str) -> Result<String, RelayConfigError> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(RelayConfigError::EmptyUrl);
    }
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("wss://{trimmed}"))
    }
}

pub fn normalize_relay_urls(relays: &[String]) -> Result<Vec<String>, RelayConfigError> {
    let mut relays = relays.to_vec();
    dedupe_relay_urls(&mut relays)?;
    Ok(relays)
}

pub fn dedupe_relay_urls(relays: &mut Vec<String>) -> Result<(), RelayConfigError> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(relays.len());
    for relay in relays.iter() {
        let url = normalize_relay_url(relay)?;
        if seen.insert(url.clone()) {
            normalized.push(url);
        }
    }
    *relays = normalized;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_urls_are_normalized_to_configured_websocket_form() {
        assert_eq!(
            normalize_relay_url(" relay.example/ ").unwrap(),
            "wss://relay.example"
        );
        assert_eq!(
            normalize_relay_url("ws://relay.example/").unwrap(),
            "ws://relay.example"
        );
        assert_eq!(
            normalize_relay_url("wss://relay.example/path/").unwrap(),
            "wss://relay.example/path"
        );
        assert_eq!(
            normalize_relay_url("   ").unwrap_err(),
            RelayConfigError::EmptyUrl
        );
    }

    #[test]
    fn relay_urls_are_deduped_after_normalization() {
        let mut relays = vec![
            " relay.example/ ".to_owned(),
            "wss://relay.example".to_owned(),
            "ws://relay.example".to_owned(),
        ];

        dedupe_relay_urls(&mut relays).unwrap();

        assert_eq!(
            relays,
            vec![
                "wss://relay.example".to_owned(),
                "ws://relay.example".to_owned()
            ]
        );
    }
}
