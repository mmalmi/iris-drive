#[allow(clippy::wildcard_imports)]
use super::*;
use crate::config::Drive;
use crate::paths::config_path_in;
use crate::profile::Profile;
use std::net::SocketAddr;
use std::path::Path;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn init_account_config(dir: &Path) {
    let account = Profile::create(dir, Some("gateway-test".into())).unwrap();
    let mut cfg = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(account.state.root_scope_id()));
    cfg.save(config_path_in(dir)).unwrap();
}

fn account_caldav_identity(dir: &Path) -> String {
    let cfg = AppConfig::load_or_default_cached_profile(config_path_in(dir)).unwrap();
    crate::app_key_summary::pubkey_npub(&cfg.profile.as_ref().unwrap().app_key_pubkey)
}

#[tokio::test]
async fn gateway_caldav_put_report_get_and_delete_round_trip_calendar_event() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let identity = account_caldav_identity(cfg_dir.path());
    let calendar_href = format!("/caldav/calendars/{identity}/calendar/");
    let event_href = format!("{calendar_href}event-123.ics");
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let ics = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:event-123@calendar.iris.to\r\nSUMMARY:WebDAV bridge\r\nDTSTART:20260622T100000Z\r\nDTEND:20260622T103000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    let options = http_request(
        server.local_addr(),
        "OPTIONS",
        "localhost",
        &calendar_href,
        &[],
        b"",
    )
    .await;
    assert!(options.starts_with("HTTP/1.1 204 No Content"), "{options}");
    assert!(options.to_ascii_lowercase().contains("dav:"), "{options}");

    let apple_proppatch = http_request(
        server.local_addr(),
        "PROPPATCH",
        "localhost",
        &calendar_href,
        &[("Content-Type", "application/xml")],
        br#"<?xml version="1.0" encoding="UTF-8"?><A:propertyupdate xmlns:A="DAV:"><A:set><A:prop><D:calendar-order xmlns:D="http://apple.com/ns/ical/">0</D:calendar-order></A:prop></A:set></A:propertyupdate>"#,
    )
    .await;
    assert!(
        apple_proppatch.starts_with("HTTP/1.1 204 No Content"),
        "{apple_proppatch}"
    );

    let apple_collection_propfind = http_request(
        server.local_addr(),
        "PROPFIND",
        "localhost",
        &calendar_href,
        &[("Depth", "0"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0" encoding="UTF-8"?><A:propfind xmlns:A="DAV:"><A:prop><C:getctag xmlns:C="http://calendarserver.org/ns/"/><A:sync-token/></A:prop></A:propfind>"#,
    )
    .await;
    assert!(
        apple_collection_propfind.starts_with("HTTP/1.1 207 Multi-Status"),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind.contains("xmlns:CS=\"http://calendarserver.org/ns/\""),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind.contains("<CS:getctag>"),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind
            .contains("<D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype>"),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind.contains("<D:getetag>&quot;"),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind.contains("<D:supported-report-set>"),
        "{apple_collection_propfind}"
    );
    assert!(
        !apple_collection_propfind.contains("<D:getctag>"),
        "{apple_collection_propfind}"
    );
    assert!(
        apple_collection_propfind.contains("<D:sync-token>urn:iris-drive:caldav-sync:"),
        "{apple_collection_propfind}"
    );
    assert!(
        !apple_collection_propfind.contains("HTTP/1.1 404 Not Found"),
        "{apple_collection_propfind}"
    );

    let put = http_request(
        server.local_addr(),
        "PUT",
        "localhost",
        &event_href,
        &[("Content-Type", "text/calendar; charset=utf-8")],
        ics,
    )
    .await;
    assert!(put.starts_with("HTTP/1.1 201 Created"), "{put}");
    assert!(put.to_ascii_lowercase().contains("etag:"), "{put}");

    let report = http_request(
        server.local_addr(),
        "REPORT",
        "localhost",
        &calendar_href,
        &[("Depth", "1"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0"?><C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/><C:calendar-data/></D:prop></C:calendar-query>"#,
    )
    .await;
    assert!(report.starts_with("HTTP/1.1 207 Multi-Status"), "{report}");
    assert!(report.contains(&event_href), "{report}");
    assert!(report.contains("SUMMARY:WebDAV bridge"), "{report}");

    let apple_multiget_body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><B:calendar-multiget xmlns:B="urn:ietf:params:xml:ns:caldav"><A:prop xmlns:A="DAV:"><A:getetag/><B:calendar-data/></A:prop><A:href xmlns:A="DAV:">{event_href}</A:href></B:calendar-multiget>"#
    );
    let apple_multiget = http_request(
        server.local_addr(),
        "REPORT",
        "localhost",
        &calendar_href,
        &[("Depth", "1"), ("Content-Type", "application/xml")],
        apple_multiget_body.as_bytes(),
    )
    .await;
    assert!(
        apple_multiget.starts_with("HTTP/1.1 207 Multi-Status"),
        "{apple_multiget}"
    );
    assert!(apple_multiget.contains(&event_href), "{apple_multiget}");
    assert!(
        apple_multiget.contains("SUMMARY:WebDAV bridge"),
        "{apple_multiget}"
    );

    let apple_sync_collection = http_request(
        server.local_addr(),
        "REPORT",
        "localhost",
        &calendar_href,
        &[("Depth", "1"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0" encoding="UTF-8"?><A:sync-collection xmlns:A="DAV:"><A:sync-token>"old-token"</A:sync-token><A:sync-level>1</A:sync-level><A:prop><A:getetag/><A:getcontenttype/></A:prop></A:sync-collection>"#,
    )
    .await;
    assert!(
        apple_sync_collection.starts_with("HTTP/1.1 207 Multi-Status"),
        "{apple_sync_collection}"
    );
    assert!(
        apple_sync_collection.contains("<D:sync-token>"),
        "{apple_sync_collection}"
    );
    assert!(
        apple_sync_collection.contains("urn:iris-drive:caldav-sync:"),
        "{apple_sync_collection}"
    );
    assert!(
        apple_sync_collection.contains(&event_href),
        "{apple_sync_collection}"
    );
    assert!(
        apple_sync_collection
            .contains("<D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype>"),
        "{apple_sync_collection}"
    );

    let get = http_request(
        server.local_addr(),
        "GET",
        "localhost",
        &event_href,
        &[],
        b"",
    )
    .await;
    assert!(get.starts_with("HTTP/1.1 200 OK"), "{get}");
    assert!(get.contains("BEGIN:VCALENDAR"), "{get}");
    assert!(get.contains("SUMMARY:WebDAV bridge"), "{get}");

    let delete = http_request(
        server.local_addr(),
        "DELETE",
        "localhost",
        &event_href,
        &[],
        b"",
    )
    .await;
    assert!(delete.starts_with("HTTP/1.1 204 No Content"), "{delete}");

    let report_after_delete = http_request(
        server.local_addr(),
        "REPORT",
        "localhost",
        &calendar_href,
        &[("Depth", "1"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0"?><C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/><C:calendar-data/></D:prop></C:calendar-query>"#,
    )
    .await;
    assert!(
        !report_after_delete.contains("SUMMARY:WebDAV bridge"),
        "{report_after_delete}"
    );

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_caldav_root_propfind_exposes_calendar_home_for_apple_calendar() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let identity = account_caldav_identity(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let propfind = http_request(
        server.local_addr(),
        "PROPFIND",
        "localhost",
        "/caldav/",
        &[("Depth", "0"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0"?><D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><C:calendar-home-set/><D:current-user-principal/></D:prop></D:propfind>"#,
    )
    .await;
    assert!(
        propfind.starts_with("HTTP/1.1 207 Multi-Status"),
        "{propfind}"
    );
    assert!(propfind.contains("<C:calendar-home-set>"), "{propfind}");
    assert!(
        propfind.contains(&format!("<D:href>/caldav/calendars/{identity}/</D:href>")),
        "{propfind}"
    );
    assert!(
        propfind.contains(&format!("<D:href>/caldav/principals/{identity}/</D:href>")),
        "{propfind}"
    );

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_caldav_root_get_serves_calendar_subscription_feed() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let get = http_request(
        server.local_addr(),
        "GET",
        "localhost",
        "/caldav/",
        &[],
        b"",
    )
    .await;
    assert!(get.starts_with("HTTP/1.1 200 OK"), "{get}");
    assert!(get.contains("content-type: text/calendar"), "{get}");
    assert!(get.contains("BEGIN:VCALENDAR"), "{get}");
    assert!(get.contains("VERSION:2.0"), "{get}");
    assert!(get.contains("X-WR-CALNAME:Calendar"), "{get}");
    assert!(get.contains("END:VCALENDAR"), "{get}");

    server.shutdown().await.unwrap();
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}
