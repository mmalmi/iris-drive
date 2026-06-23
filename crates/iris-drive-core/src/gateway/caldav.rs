#[allow(clippy::wildcard_imports)]
use super::*;

const CALDAV_ROOT: &str = "/caldav/";
const CALDAV_LEGACY_IDENTITY: &str = "iris";
const CALDAV_FEED: &str = "/caldav/calendar.ics";
const WELL_KNOWN_CALDAV: &str = "/.well-known/caldav";

#[derive(Clone, Debug)]
struct CaldavPaths {
    principal: String,
    home: String,
    calendar: String,
    calendar_feed: String,
}

impl CaldavPaths {
    fn new(identity: &str) -> Self {
        let identity = identity.trim();
        let identity = if identity.is_empty() {
            CALDAV_LEGACY_IDENTITY
        } else {
            identity
        };
        let identity = percent_encode_path_segment(identity);
        Self {
            principal: format!("/caldav/principals/{identity}/"),
            home: format!("/caldav/calendars/{identity}/"),
            calendar: format!("/caldav/calendars/{identity}/calendar/"),
            calendar_feed: format!("/caldav/calendars/{identity}/calendar.ics"),
        }
    }
}

pub(crate) fn is_caldav_path(path: &str) -> bool {
    path == WELL_KNOWN_CALDAV || path == "/caldav" || path.starts_with(CALDAV_ROOT)
}

pub(crate) async fn handle_caldav_request(
    state: GatewayState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, (StatusCode, String)> {
    if !caldav_host_allowed(headers) {
        return Err((StatusCode::BAD_REQUEST, "invalid CalDAV host".into()));
    }

    if uri.path() == WELL_KNOWN_CALDAV {
        return Ok(redirect_response(CALDAV_ROOT));
    }

    if method == Method::OPTIONS {
        return caldav_empty_response(StatusCode::NO_CONTENT);
    }

    match method.as_str() {
        "PROPFIND" => handle_caldav_propfind(&state, uri, headers).await,
        "REPORT" => handle_caldav_report(&state, uri, body).await,
        "GET" | "HEAD" => handle_caldav_get(&state, method, uri).await,
        "PUT" => handle_caldav_put(&state, uri, body).await,
        "DELETE" => handle_caldav_delete(&state, uri).await,
        _ => Err((StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into())),
    }
}

async fn handle_caldav_propfind(
    state: &GatewayState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let path = normalized_caldav_path(uri.path());
    let config = load_caldav_config(state)?;
    let paths = current_caldav_paths(&config);
    let legacy_paths = legacy_caldav_paths();
    let depth = headers
        .get("depth")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("0");
    let xml = if path == CALDAV_ROOT {
        crate::calendar::root_propfind_multistatus(CALDAV_ROOT, &paths.principal, &paths.home)
    } else if path == "/caldav/principals/" || is_principal_path(&path, &paths, &legacy_paths) {
        crate::calendar::principal_propfind_multistatus(&path, &paths.principal, &paths.home)
    } else if path == paths.home || path == legacy_paths.home {
        let data = load_caldav_calendar(state, &config).await?;
        crate::calendar::home_propfind_multistatus(&path, &paths.calendar, depth, &data)
    } else if path == paths.calendar || path == legacy_paths.calendar {
        let data = load_caldav_calendar(state, &config).await?;
        crate::calendar::collection_propfind_multistatus(&data, &path, depth)
    } else if let Some(calendar_href) = calendar_event_collection_href(&path, &paths, &legacy_paths)
    {
        let data = load_caldav_calendar(state, &config).await?;
        let id = crate::calendar::href_event_id(&path);
        let Some(event) = data
            .events
            .iter()
            .find(|event| event.id == id || crate::calendar::event_href_id(&event.id) == id)
        else {
            return Err((StatusCode::NOT_FOUND, "event not found".into()));
        };
        let href = format!(
            "{}{}.ics",
            calendar_href,
            percent_encode_path_segment(&crate::calendar::event_href_id(&event.id))
        );
        return event_propfind_response(&href, event);
    } else {
        return Err((StatusCode::NOT_FOUND, "CalDAV path not found".into()));
    };
    xml_response(StatusCode::MULTI_STATUS, xml)
}

fn event_propfind_response(
    href: &str,
    event: &crate::calendar::CalendarEvent,
) -> Result<Response, (StatusCode, String)> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><D:response><D:href>{}</D:href><D:propstat><D:prop><D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype><D:getetag>{}</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>",
        caldav_xml_escape(href),
        caldav_xml_escape(&crate::calendar::event_etag(event)),
    );
    xml_response(StatusCode::MULTI_STATUS, xml)
}

async fn handle_caldav_report(
    state: &GatewayState,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, (StatusCode, String)> {
    let path = normalized_caldav_path(uri.path());
    let config = load_caldav_config(state)?;
    let paths = current_caldav_paths(&config);
    let legacy_paths = legacy_caldav_paths();
    let Some(calendar_href) = calendar_collection_href(&path, &paths, &legacy_paths) else {
        return Err((
            StatusCode::NOT_FOUND,
            "CalDAV report target not found".into(),
        ));
    };
    let data = load_caldav_calendar(state, &config).await?;
    let body_text = String::from_utf8_lossy(body);
    let xml = if body_text.contains("calendar-multiget") {
        let hrefs = crate::calendar::extract_hrefs(&body_text);
        crate::calendar::calendar_multiget_multistatus(&data, &hrefs)
    } else {
        crate::calendar::calendar_query_multistatus(&data, calendar_href)
    };
    xml_response(StatusCode::MULTI_STATUS, xml)
}

async fn handle_caldav_get(
    state: &GatewayState,
    method: &Method,
    uri: &Uri,
) -> Result<Response, (StatusCode, String)> {
    let path = normalized_caldav_path(uri.path());
    let config = load_caldav_config(state)?;
    let paths = current_caldav_paths(&config);
    let legacy_paths = legacy_caldav_paths();
    let data = load_caldav_calendar(state, &config).await?;
    if path == CALDAV_ROOT
        || path == CALDAV_FEED
        || path == paths.calendar
        || path == paths.calendar_feed
        || path == legacy_paths.calendar
        || path == legacy_paths.calendar_feed
    {
        let body = crate::calendar::calendar_to_ics(&data, gateway_now_seconds() * 1000);
        return calendar_response(StatusCode::OK, method == Method::HEAD, body, None);
    }
    let id = crate::calendar::href_event_id(&path);
    let Some(event) = data
        .events
        .iter()
        .find(|event| event.id == id || crate::calendar::event_href_id(&event.id) == id)
    else {
        return Err((StatusCode::NOT_FOUND, "event not found".into()));
    };
    let body = crate::calendar::event_to_ics(event, &data.title, gateway_now_seconds() * 1000);
    calendar_response(
        StatusCode::OK,
        method == Method::HEAD,
        body,
        Some(crate::calendar::event_etag(event)),
    )
}

async fn handle_caldav_put(
    state: &GatewayState,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, (StatusCode, String)> {
    let path = normalized_caldav_path(uri.path());
    let config = load_caldav_config(state)?;
    let paths = current_caldav_paths(&config);
    let legacy_paths = legacy_caldav_paths();
    if calendar_event_collection_href(&path, &paths, &legacy_paths).is_none()
        || !has_ics_extension(&path)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "PUT requires an event .ics path".into(),
        ));
    }
    let id = crate::calendar::href_event_id(&path);
    let mut daemon = Daemon::open(state.config_dir.as_ref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let existed =
        crate::calendar::load_calendar_data(daemon.tree(), daemon.config(), "iris-caldav")
            .await
            .is_ok_and(|data| {
                data.events
                    .iter()
                    .any(|event| event.id == id || crate::calendar::event_href_id(&event.id) == id)
            });
    let event = crate::calendar::put_calendar_event_ics(&mut daemon, &id, body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    caldav_empty_response_with_etag(
        if existed {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::CREATED
        },
        &crate::calendar::event_etag(&event),
    )
}

async fn handle_caldav_delete(
    state: &GatewayState,
    uri: &Uri,
) -> Result<Response, (StatusCode, String)> {
    let path = normalized_caldav_path(uri.path());
    let config = load_caldav_config(state)?;
    let paths = current_caldav_paths(&config);
    let legacy_paths = legacy_caldav_paths();
    if calendar_event_collection_href(&path, &paths, &legacy_paths).is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "DELETE requires an event .ics path".into(),
        ));
    }
    let id = crate::calendar::href_event_id(&path);
    let mut daemon = Daemon::open(state.config_dir.as_ref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let changed = crate::calendar::delete_calendar_event(&mut daemon, &id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if changed {
        caldav_empty_response(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "event not found".into()))
    }
}

async fn load_caldav_calendar(
    state: &GatewayState,
    config: &AppConfig,
) -> Result<crate::calendar::CalendarData, (StatusCode, String)> {
    let owner = config
        .profile
        .as_ref()
        .map_or("iris-caldav", |profile| profile.app_key_pubkey.as_str());
    crate::calendar::load_calendar_data(state.tree.as_ref(), config, owner)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn load_caldav_config(state: &GatewayState) -> Result<AppConfig, (StatusCode, String)> {
    AppConfig::load_or_default_cached_profile(config_path_in(state.config_dir.as_ref()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

fn current_caldav_paths(config: &AppConfig) -> CaldavPaths {
    let identity = config.profile.as_ref().map_or_else(
        || CALDAV_LEGACY_IDENTITY.to_string(),
        |profile| crate::app_key_summary::pubkey_npub(&profile.app_key_pubkey),
    );
    CaldavPaths::new(&identity)
}

fn legacy_caldav_paths() -> CaldavPaths {
    CaldavPaths::new(CALDAV_LEGACY_IDENTITY)
}

fn is_principal_path(path: &str, paths: &CaldavPaths, legacy_paths: &CaldavPaths) -> bool {
    path == paths.principal || path == legacy_paths.principal
}

fn calendar_collection_href<'a>(
    path: &str,
    paths: &'a CaldavPaths,
    legacy_paths: &'a CaldavPaths,
) -> Option<&'a str> {
    if path == paths.calendar {
        Some(paths.calendar.as_str())
    } else if path == legacy_paths.calendar {
        Some(legacy_paths.calendar.as_str())
    } else {
        None
    }
}

fn calendar_event_collection_href<'a>(
    path: &str,
    paths: &'a CaldavPaths,
    legacy_paths: &'a CaldavPaths,
) -> Option<&'a str> {
    if path.starts_with(&paths.calendar) && has_ics_extension(path) {
        Some(paths.calendar.as_str())
    } else if path.starts_with(&legacy_paths.calendar) && has_ics_extension(path) {
        Some(legacy_paths.calendar.as_str())
    } else {
        None
    }
}

fn has_ics_extension(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ics"))
}

fn normalized_caldav_path(path: &str) -> String {
    if path == "/caldav" {
        return CALDAV_ROOT.into();
    }
    if path == CALDAV_ROOT || has_ics_extension(path) {
        return path.to_string();
    }
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["caldav", "principals" | "calendars", _] | ["caldav", "calendars", _, "calendar"] => {
            format!("{}/", path.trim_end_matches('/'))
        }
        _ => path.to_string(),
    }
}

fn caldav_host_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_host)
    else {
        return false;
    };
    is_loopback_host(&host)
        || host == LOCAL_PORTAL_HOST
        || host.ends_with(IRIS_LOCALHOST_SUFFIX)
        || host.ends_with(IRIS_LOCAL_SUFFIX)
}

fn caldav_empty_response(status: StatusCode) -> Result<Response, (StatusCode, String)> {
    try_finish_response(caldav_response_builder(status), Body::empty())
}

fn caldav_empty_response_with_etag(
    status: StatusCode,
    etag: &str,
) -> Result<Response, (StatusCode, String)> {
    try_finish_response(
        caldav_response_builder(status)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, "no-store"),
        Body::empty(),
    )
}

fn xml_response(status: StatusCode, xml: String) -> Result<Response, (StatusCode, String)> {
    try_finish_response(
        caldav_response_builder(status)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header(CONTENT_LENGTH, xml.len().to_string())
            .header(CACHE_CONTROL, "no-store"),
        Body::from(xml),
    )
}

fn calendar_response(
    status: StatusCode,
    head: bool,
    body: String,
    etag: Option<String>,
) -> Result<Response, (StatusCode, String)> {
    let mut builder = caldav_response_builder(status)
        .header(CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(CONTENT_LENGTH, body.len().to_string())
        .header(CACHE_CONTROL, "no-store");
    if let Some(etag) = etag {
        builder = builder.header(ETAG, etag);
    }
    try_finish_response(
        builder,
        if head {
            Body::empty()
        } else {
            Body::from(body)
        },
    )
}

fn caldav_response_builder(status: StatusCode) -> http::response::Builder {
    response_builder(status, false)
        .header("DAV", "1, 3, calendar-access")
        .header("MS-Author-Via", "DAV")
        .header("Allow", "OPTIONS, PROPFIND, REPORT, GET, HEAD, PUT, DELETE")
}

fn caldav_xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
