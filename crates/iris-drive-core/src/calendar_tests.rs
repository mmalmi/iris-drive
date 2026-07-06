use super::*;
use crate::Daemon;
use crate::config::{AppConfig, Drive};
use crate::paths::{config_path_in, key_path_in};
use crate::profile::Profile;
use tempfile::tempdir;

fn init_calendar_config(dir: &std::path::Path) {
    let account = Profile::create(dir, Some("calendar-test".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    std::fs::write(
        key_path_in(dir),
        account.app_key.keys().secret_key().to_secret_hex(),
    )
    .unwrap();
    config.save(config_path_in(dir)).unwrap();
}

#[tokio::test]
async fn calendar_put_ics_upserts_event_in_calendar_tree() {
    let dir = tempdir().unwrap();
    init_calendar_config(dir.path());
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let ics = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:event-123@calendar.iris.to\r\nSUMMARY:Bridge test\r\nDTSTART:20260622T100000Z\r\nDTEND:20260622T103000Z\r\nLOCATION:Helsinki\r\nDESCRIPTION:from CalDAV\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    put_calendar_event_ics(&mut daemon, "event-123", ics)
        .await
        .unwrap();
    let data = load_calendar_data(daemon.tree(), daemon.config(), "npub-test")
        .await
        .unwrap();

    assert_eq!(data.events.len(), 1);
    assert_eq!(data.events[0].id, "event-123");
    assert_eq!(data.events[0].title, "Bridge test");
    assert_eq!(data.events[0].location.as_deref(), Some("Helsinki"));
    assert_eq!(data.events[0].notes.as_deref(), Some("from CalDAV"));
}

#[tokio::test]
async fn calendar_delete_event_removes_it_from_calendar_tree() {
    let dir = tempdir().unwrap();
    init_calendar_config(dir.path());
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let ics = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:delete-me@calendar.iris.to\r\nSUMMARY:Delete me\r\nDTSTART:20260622T100000Z\r\nDTEND:20260622T103000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    put_calendar_event_ics(&mut daemon, "delete-me", ics)
        .await
        .unwrap();
    delete_calendar_event(&mut daemon, "delete-me")
        .await
        .unwrap();
    let data = load_calendar_data(daemon.tree(), daemon.config(), "npub-test")
        .await
        .unwrap();

    assert!(data.events.is_empty());
}

#[test]
fn caldav_report_lists_event_href_and_calendar_data() {
    let mut data = CalendarData::new("npub-test", 1_782_070_400_000);
    data.events.push(CalendarEvent {
        id: "event-123".into(),
        title: "Report me".into(),
        start: "2026-06-22T10:00:00.000Z".into(),
        end: "2026-06-22T10:30:00.000Z".into(),
        all_day: false,
        color: "violet".into(),
        location: None,
        notes: None,
        recurrence: None,
        recurrence_source_id: None,
        created_at: 1_782_070_400_000,
        updated_at: 1_782_070_400_000,
    });

    let xml = calendar_query_multistatus(&data, "/caldav/calendars/iris/calendar/");

    assert!(xml.contains("<D:href>/caldav/calendars/iris/calendar/event-123.ics</D:href>"));
    assert!(xml.contains("BEGIN:VCALENDAR"));
    assert!(xml.contains("SUMMARY:Report me"));
}

#[test]
fn caldav_multiget_extracts_apple_prefixed_href_tags() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<B:calendar-multiget xmlns:B="urn:ietf:params:xml:ns:caldav">
  <A:prop xmlns:A="DAV:"><A:getetag/><B:calendar-data/></A:prop>
  <A:href xmlns:A="DAV:">/caldav/calendars/iris/calendar/event-123.ics</A:href>
  <href>/caldav/calendars/iris/calendar/plain.ics</href>
  <D:href>/caldav/calendars/iris/calendar/dav.ics</D:href>
</B:calendar-multiget>"#;

    assert_eq!(
        extract_hrefs(xml),
        vec![
            "/caldav/calendars/iris/calendar/event-123.ics",
            "/caldav/calendars/iris/calendar/plain.ics",
            "/caldav/calendars/iris/calendar/dav.ics",
        ]
    );
}

#[test]
fn caldav_collection_tag_changes_when_events_change() {
    let mut data = CalendarData::new("npub-test", 1_782_070_400_000);
    let empty = collection_propfind_multistatus(&data, "/caldav/calendars/iris/calendar/", "0");
    data.events.push(CalendarEvent {
        id: "event-123".into(),
        title: "Tag me".into(),
        start: "2026-06-22T10:00:00.000Z".into(),
        end: "2026-06-22T10:30:00.000Z".into(),
        all_day: false,
        color: "violet".into(),
        location: None,
        notes: None,
        recurrence: None,
        recurrence_source_id: None,
        created_at: 1_782_070_400_000,
        updated_at: 1_782_070_400_000,
    });
    let with_event =
        collection_propfind_multistatus(&data, "/caldav/calendars/iris/calendar/", "0");

    assert_ne!(empty, with_event);
    assert!(with_event.contains("xmlns:CS=\"http://calendarserver.org/ns/\""));
    assert!(with_event.contains("<CS:getctag>"));
    assert!(with_event.contains("<D:sync-token>urn:iris-drive:caldav-sync:"));
    assert!(!with_event.contains("<D:getctag>"));
    assert!(!empty.contains("event-123"));
    assert!(!with_event.contains("event-123"));
}
