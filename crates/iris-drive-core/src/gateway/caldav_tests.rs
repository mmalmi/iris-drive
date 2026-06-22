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

#[tokio::test]
async fn gateway_caldav_put_report_get_and_delete_round_trip_calendar_event() {
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
    let ics = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:event-123@calendar.iris.to\r\nSUMMARY:WebDAV bridge\r\nDTSTART:20260622T100000Z\r\nDTEND:20260622T103000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    let options = http_request(
        server.local_addr(),
        "OPTIONS",
        "localhost",
        "/caldav/calendars/iris/calendar/",
        &[],
        b"",
    )
    .await;
    assert!(options.starts_with("HTTP/1.1 204 No Content"), "{options}");
    assert!(options.to_ascii_lowercase().contains("dav:"), "{options}");

    let put = http_request(
        server.local_addr(),
        "PUT",
        "localhost",
        "/caldav/calendars/iris/calendar/event-123.ics",
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
        "/caldav/calendars/iris/calendar/",
        &[("Depth", "1"), ("Content-Type", "application/xml")],
        br#"<?xml version="1.0"?><C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/><C:calendar-data/></D:prop></C:calendar-query>"#,
    )
    .await;
    assert!(report.starts_with("HTTP/1.1 207 Multi-Status"), "{report}");
    assert!(
        report.contains("/caldav/calendars/iris/calendar/event-123.ics"),
        "{report}"
    );
    assert!(report.contains("SUMMARY:WebDAV bridge"), "{report}");

    let get = http_request(
        server.local_addr(),
        "GET",
        "localhost",
        "/caldav/calendars/iris/calendar/event-123.ics",
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
        "/caldav/calendars/iris/calendar/event-123.ics",
        &[],
        b"",
    )
    .await;
    assert!(delete.starts_with("HTTP/1.1 204 No Content"), "{delete}");

    let report_after_delete = http_request(
        server.local_addr(),
        "REPORT",
        "localhost",
        "/caldav/calendars/iris/calendar/",
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
