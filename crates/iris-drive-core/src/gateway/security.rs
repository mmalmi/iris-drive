#[allow(clippy::wildcard_imports)]
use super::*;

/// Content origins identify untrusted trees, never authority over the local account.
pub(super) fn private_api_host_allowed(headers: &HeaderMap) -> bool {
    request_host(headers).is_some_and(|host| is_private_api_host(&host))
}

fn is_private_api_host(host: &str) -> bool {
    is_loopback_host(host) || host == LOCAL_PORTAL_HOST
}

pub(super) fn request_host(headers: &HeaderMap) -> Option<String> {
    let authority = request_authority(headers)?;
    Some(normalize_host(authority.host()))
}

fn request_authority(headers: &HeaderMap) -> Option<http::uri::Authority> {
    let mut hosts = headers.get_all(HOST).iter();
    let value = hosts.next()?.to_str().ok()?;
    if hosts.next().is_some() {
        return None;
    }
    let authority = value.parse::<http::uri::Authority>().ok()?;
    if authority.as_str().contains('@') {
        return None;
    }
    Some(authority)
}

fn parsed_origin(value: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(value).ok()?;
    // Origin is a serialized scheme/host/port tuple, not a URL with userinfo or a path.
    if !matches!(url.scheme(), "http" | "https") || url.origin().ascii_serialization() != value {
        return None;
    }
    Some(url)
}

pub(super) fn require_same_origin(headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let mut origins = headers.get_all(ORIGIN).iter();
    let Some(origin) = origins.next() else {
        // Native clients do not send Origin. Browsers send it for WebSockets and writes.
        return Ok(());
    };
    let allowed = origins.next().is_none()
        && origin
            .to_str()
            .ok()
            .and_then(parsed_origin)
            .is_some_and(|origin| {
                request_authority(headers).is_some_and(|authority| {
                    origin.scheme() == "http"
                        && origin.host_str().map(normalize_host)
                            == Some(normalize_host(authority.host()))
                        && origin.port_or_known_default()
                            == Some(authority.port_u16().unwrap_or(80))
                })
            });
    if allowed {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "origin is not allowed".into()))
    }
}

pub(super) fn share_action_cors_origin(
    headers: &HeaderMap,
) -> Result<Option<HeaderValue>, (StatusCode, String)> {
    let mut origins = headers.get_all(ORIGIN).iter();
    let Some(origin) = origins.next() else {
        return Ok(None);
    };
    let allowed = origins.next().is_none()
        && origin
            .to_str()
            .ok()
            .and_then(parsed_origin)
            .is_some_and(|url| {
                url.as_str() == "https://drive.iris.to/"
                    || (url.scheme() == "http"
                        && url
                            .host_str()
                            .map(normalize_host)
                            .is_some_and(|host| is_private_api_host(&host)))
            });
    if allowed {
        Ok(Some(origin.clone()))
    } else {
        Err((StatusCode::FORBIDDEN, "origin is not allowed".into()))
    }
}
