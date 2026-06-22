use std::collections::HashMap;

use hashtree_core::{Cid, HashTree, LinkType, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::AppConfig;
use crate::{Daemon, DaemonError};

pub const CALENDAR_TREE_NAME: &str = "calendar";
pub const CALENDAR_DATA_FILE: &str = "calendar.json";
const CALENDAR_UID_SUFFIX: &str = "@calendar.iris.to";

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("calendar data: {0}")]
    Data(String),
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
            extra: serde_json::Map::new(),
        }
    }

    fn touch(&mut self, actor: &str, now_ms: i64) {
        self.updated_at = now_ms;
        self.updated_by = actor.to_string();
        self.events
            .sort_by(|left, right| left.start.cmp(&right.start));
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

pub async fn load_calendar_data<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
    owner_npub: &str,
) -> Result<CalendarData, CalendarError> {
    let Some(root) = current_calendar_root(config)? else {
        return Ok(CalendarData::new(owner_npub, now_millis()));
    };
    let Some(file) = tree
        .resolve(&root, CALENDAR_DATA_FILE)
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?
    else {
        return Ok(CalendarData::new(owner_npub, now_millis()));
    };
    let Some(bytes) = tree
        .read_file_range_cid(&file, 0, Some(1024 * 1024))
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?
    else {
        return Ok(CalendarData::new(owner_npub, now_millis()));
    };
    let mut data: CalendarData = serde_json::from_slice(&bytes)?;
    if data.owner_npub.trim().is_empty() {
        data.owner_npub = owner_npub.to_string();
    }
    Ok(data)
}

pub async fn put_calendar_event_ics(
    daemon: &mut Daemon,
    resource_id: &str,
    body: &[u8],
) -> Result<CalendarEvent, CalendarError> {
    let actor = calendar_actor(daemon.config());
    let now = now_millis();
    let mut data = load_calendar_data(daemon.tree(), daemon.config(), &actor).await?;
    let ics = std::str::from_utf8(body).map_err(|e| CalendarError::Data(e.to_string()))?;
    let event = event_from_ics(ics, resource_id, now)?;
    data.events.retain(|existing| existing.id != event.id);
    data.events.push(event.clone());
    data.touch(&actor, now);
    save_calendar_data(daemon, &data).await?;
    Ok(event)
}

pub async fn delete_calendar_event(
    daemon: &mut Daemon,
    resource_id: &str,
) -> Result<bool, CalendarError> {
    let actor = calendar_actor(daemon.config());
    let now = now_millis();
    let mut data = load_calendar_data(daemon.tree(), daemon.config(), &actor).await?;
    let len_before = data.events.len();
    data.events
        .retain(|event| event.id != resource_id && event_href_id(&event.id) != resource_id);
    let changed = data.events.len() != len_before;
    if changed {
        data.touch(&actor, now);
        save_calendar_data(daemon, &data).await?;
    }
    Ok(changed)
}

pub async fn save_calendar_data(
    daemon: &mut Daemon,
    data: &CalendarData,
) -> Result<(), CalendarError> {
    let bytes = serde_json::to_vec_pretty(data)?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    let tree = daemon.tree();
    let base_root = if let Some(root) = current_calendar_root(daemon.config())? {
        root
    } else {
        tree.put_directory(Vec::new())
            .await
            .map_err(|e| CalendarError::Hashtree(e.to_string()))?
    };
    let (file_cid, size) = tree
        .put_file(&bytes)
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?;
    let mut meta = HashMap::new();
    meta.insert(
        "contentType".into(),
        Value::String("application/json".into()),
    );
    meta.insert("mimeType".into(), Value::String("application/json".into()));
    let visible_root = tree
        .set_entry_with_meta(
            &base_root,
            &[],
            CALENDAR_DATA_FILE,
            &file_cid,
            size,
            LinkType::Blob,
            Some(meta),
        )
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?;
    daemon
        .import_visible_root_for_drive(CALENDAR_TREE_NAME, visible_root)
        .await?;
    Ok(())
}

fn current_calendar_root(config: &AppConfig) -> Result<Option<Cid>, CalendarError> {
    let root = config
        .profile
        .as_ref()
        .and_then(|account| {
            config
                .drive(CALENDAR_TREE_NAME)
                .and_then(|drive| drive.app_key_roots.get(&account.app_key_pubkey))
        })
        .map(|root| root.root_cid.as_str())
        .or_else(|| {
            config
                .drive(CALENDAR_TREE_NAME)
                .and_then(|drive| drive.last_root_cid.as_deref())
        });
    root.map(Cid::parse)
        .transpose()
        .map_err(|e| CalendarError::Data(e.to_string()))
}

fn calendar_actor(config: &AppConfig) -> String {
    config
        .profile
        .as_ref()
        .map(|profile| profile.app_key_pubkey.clone())
        .unwrap_or_else(|| "iris-caldav".into())
}

pub fn event_to_ics(event: &CalendarEvent, calendar_name: &str, now_ms: i64) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Iris//Iris Calendar CalDAV//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        property_line("X-WR-CALNAME", calendar_name),
        "BEGIN:VEVENT".to_string(),
        property_line("UID", &export_uid(&event.id)),
        format!("DTSTAMP:{}", millis_to_ics_utc(now_ms)),
        format!("CREATED:{}", millis_to_ics_utc(event.created_at)),
        format!("LAST-MODIFIED:{}", millis_to_ics_utc(event.updated_at)),
        property_line("SUMMARY", &event.title),
    ];
    if event.all_day {
        lines.push(format!(
            "DTSTART;VALUE=DATE:{}",
            iso_to_ics_date(&event.start)
        ));
        lines.push(format!("DTEND;VALUE=DATE:{}", iso_to_ics_date(&event.end)));
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
    lines.push("END:VCALENDAR".into());
    format!(
        "{}\r\n",
        lines
            .into_iter()
            .map(fold_ics_line)
            .collect::<Vec<_>>()
            .join("\r\n")
    )
}

pub fn calendar_query_multistatus(data: &CalendarData, collection_href: &str) -> String {
    let mut responses = String::new();
    for event in &data.events {
        let href = format!(
            "{}{}.ics",
            collection_href.trim_end_matches('/'),
            format!("/{}", percent_encode(&event_href_id(&event.id)))
        );
        let ics = event_to_ics(event, &data.title, now_millis());
        responses.push_str(&format!(
            "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:getetag>{}</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            xml_escape(&href),
            xml_escape(&event_etag(event)),
            xml_escape(&ics),
        ));
    }
    multistatus(&responses)
}

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
            responses.push_str(&format!(
                "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:getetag>{}</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                xml_escape(href),
                xml_escape(&event_etag(event)),
                xml_escape(&ics),
            ));
        } else {
            responses.push_str(&format!(
                "<D:response><D:href>{}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>",
                xml_escape(href),
            ));
        }
    }
    multistatus(&responses)
}

pub fn collection_propfind_multistatus(data: &CalendarData, href: &str, depth: &str) -> String {
    let mut responses = collection_prop_response(href, &data.title);
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

pub fn principal_propfind_multistatus(href: &str) -> String {
    let props = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris</D:displayname><D:resourcetype><D:collection/><D:principal/></D:resourcetype><C:calendar-home-set><D:href>/caldav/calendars/iris/</D:href></C:calendar-home-set><D:current-user-principal><D:href>/caldav/principals/iris/</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href)
    );
    multistatus(&props)
}

pub fn home_propfind_multistatus(href: &str, depth: &str, data: &CalendarData) -> String {
    let mut responses = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris Calendars</D:displayname><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href)
    );
    if depth != "0" {
        responses.push_str(&collection_prop_response(
            "/caldav/calendars/iris/calendar/",
            &data.title,
        ));
    }
    multistatus(&responses)
}

pub fn root_propfind_multistatus(href: &str) -> String {
    let props = format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>Iris CalDAV</D:displayname><D:resourcetype><D:collection/></D:resourcetype><D:current-user-principal><D:href>/caldav/principals/iris/</D:href></D:current-user-principal><D:principal-collection-set><D:href>/caldav/principals/</D:href></D:principal-collection-set></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href)
    );
    multistatus(&props)
}

fn collection_prop_response(href: &str, title: &str) -> String {
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><C:supported-calendar-component-set><C:comp name=\"VEVENT\"/></C:supported-calendar-component-set><D:current-user-privilege-set><D:privilege><D:read/></D:privilege><D:privilege><D:write/></D:privilege></D:current-user-privilege-set><D:getctag>{}</D:getctag><D:sync-token>{}</D:sync-token></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href),
        xml_escape(title),
        xml_escape(&calendar_collection_tag(title)),
        xml_escape(&calendar_collection_tag(title)),
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
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\">{responses}</D:multistatus>"
    )
}

pub fn extract_hrefs(xml: &str) -> Vec<String> {
    let mut hrefs = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<D:href>").or_else(|| rest.find("<href>")) {
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

pub fn href_event_id(href: &str) -> String {
    let last = href
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href);
    let raw = last.strip_suffix(".ics").unwrap_or(last);
    percent_decode(raw).unwrap_or_else(|| raw.to_string())
}

pub fn event_href_id(id: &str) -> String {
    id.strip_suffix(CALENDAR_UID_SUFFIX)
        .unwrap_or(id)
        .to_string()
}

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
    let duration = if all_day { 86_400_000 } else { 1_800_000 };
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
                Some(next @ ('\\' | ';' | ',')) => out.push(next),
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

fn calendar_collection_tag(title: &str) -> String {
    let digest = Sha256::digest(title.as_bytes());
    format!("\"{}\"", hex::encode(&digest[..12]))
}

#[cfg(test)]
mod tests {
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
}
