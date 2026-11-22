use std::collections::HashMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::DaemonError;

mod mutations;
mod projection;
mod replica;
mod replica_store;
mod replication;

use mutations::current_calendar_root;
pub use mutations::{
    delete_calendar_event, load_calendar_data, put_calendar_event_ics, save_calendar_data,
};
pub(crate) use mutations::{delete_calendar_event_checked, put_calendar_event_ics_checked};
pub use replica::{REPLICA_LABEL, replica_writer};
pub use replica_store::local_replica_tree;
pub(crate) use replica_store::{MAX_REPLICAS, account_replica_drive};
pub(crate) use replication::{
    append_missing_roots, apply_discovered_roots, publish_pending_replica,
};

pub const CALENDAR_TREE_NAME: &str = "calendar";
pub const CALENDAR_DATA_FILE: &str = "calendar.json";
const CALENDAR_UID_SUFFIX: &str = "@calendar.iris.to";
const MAX_CALENDAR_BYTES: u64 = 16 * 1024 * 1024;
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("calendar data: {0}")]
    Data(String),
    #[error("calendar resource precondition failed")]
    Precondition,
    #[error("hashtree: {0}")]
    Hashtree(String),
    #[error("daemon: {0}")]
    Daemon(#[from] DaemonError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarData {
    pub version: u32,
    pub calendar_id: String,
    pub title: String,
    pub owner_npub: String,
    pub timezone: String,
    pub time_format: String,
    pub start_of_week: String,
    #[serde(default)]
    pub events: Vec<CalendarEvent>,
    pub availability: Value,
    #[serde(default)]
    pub booking_links: Vec<Value>,
    pub updated_at: i64,
    pub updated_by: String,
    #[serde(skip)]
    deleted_events: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<CalendarRecurrenceRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_source_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarRecurrenceRule {
    pub frequency: String,
    pub interval: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

impl CalendarData {
    #[must_use]
    pub fn new(owner_npub: &str, now_ms: i64) -> Self {
        Self {
            version: 1,
            calendar_id: format!("calendar-{now_ms}"),
            title: "Calendar".into(),
            owner_npub: owner_npub.to_string(),
            timezone: "UTC".into(),
            time_format: "auto".into(),
            start_of_week: "monday".into(),
            events: Vec::new(),
            availability: default_availability(),
            booking_links: vec![json!({
                "id": format!("booking-{now_ms}"),
                "slug": "meet",
                "title": "Meet",
                "durationMinutes": 30,
                "enabled": true,
                "createdAt": now_ms,
                "updatedAt": now_ms,
            })],
            updated_at: now_ms,
            updated_by: owner_npub.to_string(),
            deleted_events: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

fn default_availability() -> Value {
    json!({
        "timezone": "UTC",
        "slotDurationMinutes": 30,
        "bufferBeforeMinutes": 0,
        "bufferAfterMinutes": 0,
        "minimumNoticeMinutes": 120,
        "rules": (0..7).map(|weekday| json!({
            "id": format!("weekday-{weekday}"),
            "weekday": weekday,
            "enabled": (1..=5).contains(&weekday),
            "startTime": "09:00",
            "endTime": "17:00",
        })).collect::<Vec<_>>(),
    })
}

#[must_use]
pub fn calendar_to_ics(data: &CalendarData, now_ms: i64) -> String {
    let mut lines = calendar_header_lines(&data.title);
    for event in &data.events {
        lines.extend(event_component_lines(event, now_ms));
    }
    lines.push("END:VCALENDAR".into());
    ics_document(lines)
}

#[must_use]
pub fn event_to_ics(event: &CalendarEvent, calendar_name: &str, now_ms: i64) -> String {
    let mut lines = calendar_header_lines(calendar_name);
    lines.extend(event_component_lines(event, now_ms));
    lines.push("END:VCALENDAR".into());
    ics_document(lines)
}

fn calendar_header_lines(calendar_name: &str) -> Vec<String> {
    vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Iris//Iris Calendar CalDAV//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        property_line("X-WR-CALNAME", calendar_name),
    ]
}

fn event_component_lines(event: &CalendarEvent, now_ms: i64) -> Vec<String> {
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        property_line("UID", &export_uid(&event.id)),
        format!("DTSTAMP:{}", millis_to_ics_utc(now_ms)),
        format!("CREATED:{}", millis_to_ics_utc(event.created_at)),
        format!("LAST-MODIFIED:{}", millis_to_ics_utc(event.updated_at)),
        property_line("SUMMARY", &event.title),
    ];
    if event.all_day {
        let (start, end) = all_day_ics_dates(event);
        lines.push(format!("DTSTART;VALUE=DATE:{start}"));
        lines.push(format!("DTEND;VALUE=DATE:{end}"));
    } else {
        lines.push(format!("DTSTART:{}", iso_to_ics_utc(&event.start)));
        lines.push(format!("DTEND:{}", iso_to_ics_utc(&event.end)));
    }
    if let Some(location) = event.location.as_ref().filter(|value| !value.is_empty()) {
        lines.push(property_line("LOCATION", location));
    }
    if let Some(notes) = event.notes.as_ref().filter(|value| !value.is_empty()) {
        lines.push(property_line("DESCRIPTION", notes));
    }
    if let Some(rrule) = event.recurrence.as_ref().and_then(recurrence_to_rrule) {
        lines.push(rrule);
    }
    lines.push("END:VEVENT".into());
    lines
}

fn all_day_ics_dates(event: &CalendarEvent) -> (String, String) {
    if let Some(day) = event.extra.get("day").and_then(Value::as_str)
        && let Some(start) = iso_to_millis_floor(&format!("{day}T00:00:00"))
    {
        let days = event.extra.get("days").and_then(Value::as_i64).unwrap_or(1);
        let end = millis_to_iso(start + days.clamp(1, 3660) * DAY_MS);
        return (iso_to_ics_date(day), iso_to_ics_date(&end));
    }
    (iso_to_ics_date(&event.start), iso_to_ics_date(&event.end))
}

fn ics_document(lines: Vec<String>) -> String {
    format!(
        "{}\r\n",
        lines
            .into_iter()
            .map(fold_ics_line)
            .collect::<Vec<_>>()
            .join("\r\n")
    )
}

#[must_use]
pub fn calendar_query_multistatus(data: &CalendarData, collection_href: &str) -> String {
    let mut responses = String::new();
    for event in &data.events {
        let href = format!(
            "{}/{}.ics",
            collection_href.trim_end_matches('/'),
            percent_encode(&event_href_id(&event.id))
        );
        let ics = event_to_ics(event, &data.title, now_millis());
        let _ = write!(
            responses,
            "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:getetag>{}</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            xml_escape(&href),
            xml_escape(&event_etag(event)),
            xml_escape(&ics),
        );
    }
    multistatus(&responses)
}

#[must_use]
pub fn calendar_multiget_multistatus(data: &CalendarData, hrefs: &[String]) -> String {
    let mut responses = String::new();
    for href in hrefs {
        let id = href_event_id(href);
        if let Some(event) = data
            .events
            .iter()
            .find(|event| event.id == id || event_href_id(&event.id) == id)
        {
            let ics = event_to_ics(event, &data.title, now_millis());
            let _ = write!(
                responses,
                "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:getetag>{}</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                xml_escape(href),
                xml_escape(&event_etag(event)),
                xml_escape(&ics),
            );
        } else {
            let _ = write!(
                responses,
                "<D:response><D:href>{}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>",
                xml_escape(href),
            );
        }
    }
    multistatus(&responses)
}

#[must_use]
pub fn calendar_sync_collection_multistatus(data: &CalendarData, collection_href: &str) -> String {
    calendar_sync_report(data, collection_href, false)
}

pub(crate) fn calendar_sync_report(
    data: &CalendarData,
    collection_href: &str,
    incremental: bool,
) -> String {
    let mut responses = String::new();
    for event in &data.events {
        let event_href = format!(
            "{}/{}.ics",
            collection_href.trim_end_matches('/'),
            percent_encode(&event_href_id(&event.id))
        );
        responses.push_str(&event_prop_response(&event_href, event));
    }
    if incremental {
        for id in &data.deleted_events {
            let href = format!(
                "{}/{}.ics",
                collection_href.trim_end_matches('/'),
                percent_encode(&event_href_id(id))
            );
            let _ = write!(
                responses,
                "<D:response><D:href>{}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>",
                xml_escape(&href),
            );
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><D:sync-token>{}</D:sync-token>{responses}</D:multistatus>",
        xml_escape(&calendar_sync_token(data)),
    )
}

#[must_use]
pub fn collection_propfind_multistatus(data: &CalendarData, href: &str, depth: &str) -> String {
    let mut responses = collection_prop_response(href, data);
    if depth != "0" {
        for event in &data.events {
            let event_href = format!(
                "{}/{}.ics",
                href.trim_end_matches('/'),
                percent_encode(&event_href_id(&event.id))
            );
            responses.push_str(&event_prop_response(&event_href, event));
        }
    }
    multistatus(&responses)
}

#[must_use]
pub fn principal_propfind_multistatus(href: &str, principal_href: &str, home_href: &str) -> String {
    let props = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris</D:displayname><D:resourcetype><D:collection/><D:principal/></D:resourcetype><C:calendar-home-set><D:href>{}</D:href></C:calendar-home-set><D:current-user-principal><D:href>{}</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href),
        xml_escape(home_href),
        xml_escape(principal_href),
    );
    multistatus(&props)
}

#[must_use]
pub fn home_propfind_multistatus(
    href: &str,
    calendar_href: &str,
    depth: &str,
    data: &CalendarData,
) -> String {
    let mut responses = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris Calendars</D:displayname><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href)
    );
    if depth != "0" {
        responses.push_str(&collection_prop_response(calendar_href, data));
    }
    multistatus(&responses)
}

#[must_use]
pub fn root_propfind_multistatus(href: &str, principal_href: &str, home_href: &str) -> String {
    let props = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris CalDAV</D:displayname><D:resourcetype><D:collection/></D:resourcetype><D:current-user-principal><D:href>{}</D:href></D:current-user-principal><D:principal-collection-set><D:href>/caldav/principals/</D:href></D:principal-collection-set><C:calendar-home-set><D:href>{}</D:href></C:calendar-home-set></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href),
        xml_escape(principal_href),
        xml_escape(home_href),
    );
    multistatus(&props)
}

fn collection_prop_response(href: &str, data: &CalendarData) -> String {
    let collection_tag = calendar_collection_ctag(data);
    let sync_token = calendar_sync_token(data);
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype><D:getetag>{}</D:getetag><C:calendar-description>{}</C:calendar-description><C:supported-calendar-component-set><C:comp name=\"VEVENT\"/></C:supported-calendar-component-set><D:supported-report-set><D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report><D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report><D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report></D:supported-report-set><D:current-user-privilege-set><D:privilege><D:read/></D:privilege><D:privilege><D:write/></D:privilege></D:current-user-privilege-set><CS:getctag>{}</CS:getctag><D:sync-token>{}</D:sync-token><IC:calendar-color symbolic-color=\"custom\">#AC7F5E</IC:calendar-color><IC:calendar-order>0</IC:calendar-order></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href),
        xml_escape(&data.title),
        xml_escape(&collection_tag),
        xml_escape(&data.title),
        xml_escape(&collection_tag),
        xml_escape(&sync_token),
    )
}

fn event_prop_response(href: &str, event: &CalendarEvent) -> String {
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype><D:getetag>{}</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href),
        xml_escape(&event_etag(event)),
    )
}

fn multistatus(responses: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\" xmlns:CS=\"http://calendarserver.org/ns/\" xmlns:IC=\"http://apple.com/ns/ical/\">{responses}</D:multistatus>"
    )
}

#[must_use]
pub fn extract_hrefs(xml: &str) -> Vec<String> {
    let mut hrefs = Vec::new();
    let mut rest = xml;
    while let Some(start) = find_xml_start_tag(rest, "href") {
        let after = &rest[start..];
        let Some(close_start) = after.find('>') else {
            break;
        };
        let value_start = close_start + 1;
        let Some(end) = after[value_start..].find("</") else {
            break;
        };
        hrefs.push(xml_unescape(&after[value_start..value_start + end]));
        rest = &after[value_start + end..];
    }
    hrefs
}

pub(crate) fn has_calendar_sync_token(xml: &str) -> bool {
    find_xml_start_tag(xml, "sync-token").is_some_and(|start| {
        xml[start..]
            .split_once('>')
            .and_then(|(_, tail)| tail.split_once('<'))
            .is_some_and(|(value, _)| !value.trim().is_empty())
    })
}

fn find_xml_start_tag(xml: &str, name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(relative_start) = xml[offset..].find('<') {
        let start = offset + relative_start;
        let after_open = &xml[start + 1..];
        if after_open.starts_with('/') || after_open.starts_with('!') || after_open.starts_with('?')
        {
            offset = start + 1;
            continue;
        }
        let close = after_open.find('>')?;
        let tag = after_open[..close].trim();
        if !tag.ends_with('/') && xml_tag_local_name(tag).eq_ignore_ascii_case(name) {
            return Some(start);
        }
        offset = start + close + 2;
    }
    None
}

fn xml_tag_local_name(tag: &str) -> &str {
    let name = tag
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    name.rsplit_once(':')
        .map_or(name, |(_, local_name)| local_name)
}

#[must_use]
pub fn href_event_id(href: &str) -> String {
    let last = href
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href);
    let raw = last.strip_suffix(".ics").unwrap_or(last);
    percent_decode(raw).unwrap_or_else(|| raw.to_string())
}

#[must_use]
pub fn event_href_id(id: &str) -> String {
    id.strip_suffix(CALENDAR_UID_SUFFIX)
        .unwrap_or(id)
        .to_string()
}

#[must_use]
pub fn event_etag(event: &CalendarEvent) -> String {
    let json = serde_json::to_vec(event).unwrap_or_default();
    let digest = Sha256::digest(json);
    format!("\"{}\"", hex::encode(&digest[..12]))
}

pub fn event_from_ics(
    ics: &str,
    fallback_resource_id: &str,
    now_ms: i64,
) -> Result<CalendarEvent, CalendarError> {
    let lines = event_content_lines(ics)?;
    let uid = value_for(&lines, "UID").unwrap_or_else(|| fallback_resource_id.to_string());
    let id = uid
        .strip_suffix(CALENDAR_UID_SUFFIX)
        .unwrap_or(&uid)
        .to_string();
    let title = value_for(&lines, "SUMMARY")
        .map(|value| unescape_ics_text(&value))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Untitled event".into());
    let start = line_for(&lines, "DTSTART")
        .ok_or_else(|| CalendarError::Data("VEVENT missing DTSTART".into()))?;
    let all_day = start
        .params
        .get("VALUE")
        .is_some_and(|value| value.eq_ignore_ascii_case("DATE"))
        || start.value.len() == 8;
    let start_iso = parse_ics_date(&start.value, all_day)?;
    let end_iso = match line_for(&lines, "DTEND") {
        Some(end) => parse_ics_date(&end.value, all_day)?,
        None => default_end_iso(&start_iso, all_day),
    };
    let created_at = value_for(&lines, "CREATED")
        .and_then(|value| parse_ics_millis(&value))
        .unwrap_or(now_ms);
    let updated_at = value_for(&lines, "LAST-MODIFIED")
        .and_then(|value| parse_ics_millis(&value))
        .unwrap_or(now_ms);
    let mut extra = serde_json::Map::new();
    if all_day
        && let Some(day) = start_iso.get(..10)
        && let Some((start, end)) =
            iso_to_millis_floor(&start_iso).zip(iso_to_millis_floor(&end_iso))
    {
        extra.insert("day".into(), json!(day));
        extra.insert("days".into(), json!(((end - start) / DAY_MS).max(1)));
    }
    Ok(CalendarEvent {
        id,
        title,
        start: start_iso,
        end: end_iso,
        all_day,
        color: "violet".into(),
        location: value_for(&lines, "LOCATION").and_then(|value| optional_text(&value)),
        notes: value_for(&lines, "DESCRIPTION").and_then(|value| optional_text(&value)),
        recurrence: value_for(&lines, "RRULE").and_then(|value| recurrence_from_rrule(&value)),
        recurrence_source_id: None,
        created_at,
        updated_at,
        extra,
    })
}

#[derive(Debug, Clone)]
struct ContentLine {
    name: String,
    params: HashMap<String, String>,
    value: String,
}

fn event_content_lines(ics: &str) -> Result<Vec<ContentLine>, CalendarError> {
    let mut lines = Vec::new();
    let mut in_event = false;
    for line in unfold_ics_lines(ics) {
        let Some(parsed) = parse_content_line(&line) else {
            continue;
        };
        if parsed.name == "BEGIN" && parsed.value.eq_ignore_ascii_case("VEVENT") {
            in_event = true;
            continue;
        }
        if parsed.name == "END" && parsed.value.eq_ignore_ascii_case("VEVENT") {
            return Ok(lines);
        }
        if in_event {
            lines.push(parsed);
        }
    }
    Err(CalendarError::Data("missing VEVENT".into()))
}

fn unfold_ics_lines(ics: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in ics.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.push_str(&raw[1..]);
            }
        } else if !raw.trim().is_empty() {
            out.push(raw.to_string());
        }
    }
    out
}

fn parse_content_line(line: &str) -> Option<ContentLine> {
    let (name_and_params, value) = line.split_once(':')?;
    let mut parts = name_and_params.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let mut params = HashMap::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        params.insert(
            key.to_ascii_uppercase(),
            value.trim_matches('"').to_string(),
        );
    }
    Some(ContentLine {
        name,
        params,
        value: value.to_string(),
    })
}

fn line_for<'a>(lines: &'a [ContentLine], name: &str) -> Option<&'a ContentLine> {
    lines.iter().find(|line| line.name == name)
}

fn value_for(lines: &[ContentLine], name: &str) -> Option<String> {
    line_for(lines, name).map(|line| line.value.clone())
}

fn parse_ics_date(raw: &str, all_day: bool) -> Result<String, CalendarError> {
    let value = raw.trim();
    if all_day || value.len() == 8 {
        if value.len() != 8 {
            return Err(CalendarError::Data(format!(
                "invalid all-day date: {value}"
            )));
        }
        return Ok(format!(
            "{}-{}-{}T00:00:00.000Z",
            &value[0..4],
            &value[4..6],
            &value[6..8]
        ));
    }
    let value = value.trim_end_matches('Z');
    if value.len() < 13 {
        return Err(CalendarError::Data(format!("invalid datetime: {raw}")));
    }
    let second = if value.len() >= 15 {
        &value[13..15]
    } else {
        "00"
    };
    Ok(format!(
        "{}-{}-{}T{}:{}:{second}.000Z",
        &value[0..4],
        &value[4..6],
        &value[6..8],
        &value[9..11],
        &value[11..13]
    ))
}

fn parse_ics_millis(raw: &str) -> Option<i64> {
    let iso = parse_ics_date(raw, false).ok()?;
    iso_to_millis_floor(&iso)
}

fn iso_to_millis_floor(iso: &str) -> Option<i64> {
    let year = iso.get(0..4)?.parse::<i64>().ok()?;
    let month = iso.get(5..7)?.parse::<i64>().ok()?;
    let day = iso.get(8..10)?.parse::<i64>().ok()?;
    let hour = iso.get(11..13)?.parse::<i64>().ok()?;
    let minute = iso.get(14..16)?.parse::<i64>().ok()?;
    let second = iso.get(17..19)?.parse::<i64>().ok()?;
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000)
}

fn default_end_iso(start_iso: &str, all_day: bool) -> String {
    let start = iso_to_millis_floor(start_iso).unwrap_or_else(now_millis);
    let duration = if all_day { DAY_MS } else { 1_800_000 };
    millis_to_iso(start.saturating_add(duration))
}

fn recurrence_from_rrule(value: &str) -> Option<CalendarRecurrenceRule> {
    let parts = value
        .split(';')
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.to_ascii_uppercase(), value.to_ascii_lowercase()))
        .collect::<HashMap<_, _>>();
    let frequency = match parts.get("FREQ")?.as_str() {
        "daily" | "weekly" | "monthly" | "yearly" => parts.get("FREQ")?.clone(),
        _ => return None,
    };
    let interval = parts
        .get("INTERVAL")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let until = parts.get("UNTIL").and_then(|value| {
        if value.len() >= 8 {
            Some(format!(
                "{}-{}-{}",
                &value[0..4],
                &value[4..6],
                &value[6..8]
            ))
        } else {
            None
        }
    });
    Some(CalendarRecurrenceRule {
        frequency,
        interval,
        until,
    })
}

fn recurrence_to_rrule(rule: &CalendarRecurrenceRule) -> Option<String> {
    let freq = match rule.frequency.as_str() {
        "daily" => "DAILY",
        "weekly" => "WEEKLY",
        "monthly" => "MONTHLY",
        "yearly" => "YEARLY",
        _ => return None,
    };
    let mut parts = vec![format!("FREQ={freq}")];
    if rule.interval > 1 {
        parts.push(format!("INTERVAL={}", rule.interval));
    }
    if let Some(until) = rule.until.as_ref() {
        parts.push(format!("UNTIL={}", until.replace('-', "")));
    }
    Some(format!("RRULE:{}", parts.join(";")))
}

fn property_line(name: &str, value: &str) -> String {
    format!("{name}:{}", escape_ics_text(value))
}

fn escape_ics_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn unescape_ics_text(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(next) => out.push(next),
                None => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn optional_text(value: &str) -> Option<String> {
    let text = unescape_ics_text(value).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn export_uid(id: &str) -> String {
    if id.contains('@') {
        id.to_string()
    } else {
        format!("{id}{CALENDAR_UID_SUFFIX}")
    }
}

fn fold_ics_line(line: String) -> String {
    if line.len() <= 74 {
        return line;
    }
    let mut out = String::new();
    for (index, chunk) in line.as_bytes().chunks(74).enumerate() {
        if index > 0 {
            out.push_str("\r\n ");
        }
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    out
}

fn iso_to_ics_utc(iso: &str) -> String {
    let date = iso.get(0..10).unwrap_or("1970-01-01").replace('-', "");
    let time = iso.get(11..19).unwrap_or("00:00:00").replace(':', "");
    format!("{date}T{time}Z")
}

fn iso_to_ics_date(iso: &str) -> String {
    iso.get(0..10).unwrap_or("1970-01-01").replace('-', "")
}

fn millis_to_ics_utc(ms: i64) -> String {
    iso_to_ics_utc(&millis_to_iso(ms))
}

fn millis_to_iso(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.000Z")
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut out, "%{byte:02X}");
            }
        }
    }
    out
}

fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = hex_value(*bytes.get(index + 1)?)?;
            let lo = hex_value(*bytes.get(index + 2)?)?;
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn calendar_collection_ctag(data: &CalendarData) -> String {
    format!("\"{}\"", calendar_collection_digest(data))
}

fn calendar_sync_token(data: &CalendarData) -> String {
    format!(
        "urn:iris-drive:caldav-sync:{}",
        calendar_collection_digest(data)
    )
}

fn calendar_collection_digest(data: &CalendarData) -> String {
    let mut digest = Sha256::new();
    digest.update(data.title.as_bytes());
    let mut event_tags = data
        .events
        .iter()
        .map(|event| (event_href_id(&event.id), event_etag(event)))
        .collect::<Vec<_>>();
    event_tags.sort();
    for (event_id, event_tag) in event_tags {
        digest.update([0]);
        digest.update(event_id.as_bytes());
        digest.update([0]);
        digest.update(event_tag.as_bytes());
    }
    let digest = digest.finalize();
    hex::encode(&digest[..12])
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
