use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::replica::{Fields, invalid};
use super::{
    CalendarData, CalendarError, CalendarEvent, DAY_MS, iso_to_millis_floor, millis_to_iso,
};

pub(super) fn genesis(owner: &str) -> CalendarData {
    let mut data = CalendarData::new(owner, 0);
    data.calendar_id = format!("calendar-{owner}");
    data.booking_links[0]["id"] = json!("booking-default");
    data.extra.insert("sharedWith".into(), json!([]));
    data.extra.insert("subscriptions".into(), json!([]));
    data
}

fn fields_at(fields: &mut Fields, prefix: &[&str], object: &Map<String, Value>) {
    fields.extend(object.iter().map(|(key, value)| {
        let path = prefix.iter().copied().chain([key.as_str()]);
        (path.map(str::to_owned).collect(), value.clone())
    }));
}

fn object(value: Value) -> Result<Map<String, Value>, CalendarError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(invalid("calendar record is not an object")),
    }
}

fn records_at(
    fields: &mut Fields,
    prefix: &[&str],
    key: &str,
    value: Value,
) -> Result<(), CalendarError> {
    if let Value::Array(values) = value {
        for value in values {
            let mut record = object(value)?;
            let id = record
                .remove(key)
                .and_then(|id| id.as_str().map(str::to_owned))
                .filter(|id| !id.is_empty())
                .ok_or_else(|| invalid("calendar record has no identity"))?;
            let path: Vec<_> = prefix.iter().copied().chain([id.as_str()]).collect();
            fields.insert(
                path.iter().map(|key| (*key).to_owned()).collect(),
                json!(true),
            );
            fields_at(fields, &path, &record);
        }
    }
    Ok(())
}

pub(super) fn event_fields(event: &CalendarEvent) -> Result<Fields, CalendarError> {
    let mut record = object(serde_json::to_value(event)?)?;
    let mut fields = Fields::new();
    let prefix = ["events", event.id.as_str()];
    fields.insert(prefix.map(str::to_owned).to_vec(), json!(true));
    let schedule = if event.all_day {
        let day = record
            .get("day")
            .and_then(Value::as_str)
            .or_else(|| event.start.get(..10))
            .ok_or_else(|| invalid("all-day event is missing its date"))?;
        let days = record
            .get("days")
            .and_then(Value::as_u64)
            .or_else(|| {
                let start = iso_to_millis_floor(&event.start)?;
                let end = iso_to_millis_floor(&event.end)?;
                u64::try_from(((end - start) / DAY_MS).max(1)).ok()
            })
            .ok_or_else(|| invalid("all-day event is missing its duration"))?;
        json!({"allDay":true,"day":day,"days":days})
    } else {
        json!({"allDay":false,"start":event.start,"end":event.end})
    };
    fields.insert(
        vec!["events".into(), event.id.clone(), "schedule".into()],
        schedule,
    );
    if let Some(Value::Array(guests)) = record.remove("guests") {
        for guest in guests {
            if let Some(npub) = guest.get("npub").and_then(Value::as_str) {
                fields.insert(
                    vec![
                        "events".into(),
                        event.id.clone(),
                        "guests".into(),
                        npub.into(),
                    ],
                    guest,
                );
            }
        }
    }
    for key in ["id", "calendar", "allDay", "start", "end", "day", "days"] {
        record.remove(key);
    }
    fields_at(&mut fields, &prefix, &record);
    Ok(fields)
}

pub(super) fn flatten(data: &CalendarData) -> Result<Fields, CalendarError> {
    let mut data_map = object(serde_json::to_value(data)?)?;
    let mut fields = Fields::new();
    let mut sharing = Map::new();
    for key in ["shareKey", "sharedWith"] {
        if let Some(value) = data_map.remove(key) {
            sharing.insert(key.into(), value);
        }
    }
    fields.insert(vec!["sharing".into()], Value::Object(sharing));
    if let Some(Value::Object(withdrawals)) = data_map.remove("shareWithdrawals") {
        fields_at(&mut fields, &["shareWithdrawals"], &withdrawals);
    }
    let mut availability = object(data_map.remove("availability").unwrap_or(Value::Null))?;
    if let Some(rules) = availability.remove("rules") {
        records_at(&mut fields, &["availability", "rules"], "id", rules)?;
    }
    fields_at(&mut fields, &["availability"], &availability);
    for (collection, key) in [("bookingLinks", "id"), ("subscriptions", "npub")] {
        if let Some(values) = data_map.remove(collection) {
            records_at(&mut fields, &[collection], key, values)?;
        }
    }
    data_map.remove("events");
    for event in &data.events {
        fields.extend(event_fields(event)?);
    }
    fields_at(&mut fields, &[], &data_map);
    Ok(fields)
}

pub(super) fn differences(mut before: Fields, after: Fields) -> Fields {
    let mut edits = Fields::new();
    for (path, value) in after {
        if before.remove(&path).as_ref() != Some(&value) {
            edits.insert(path, value);
        }
    }
    edits.extend(before.into_keys().map(|path| (path, Value::Null)));
    edits
}

fn assign(map: &mut Map<String, Value>, field: &str, value: Value) {
    if value.is_null() {
        map.remove(field);
    } else {
        map.insert(field.into(), value);
    }
}

fn initial_record(
    table: &mut BTreeMap<String, Map<String, Value>>,
    id: String,
    key: &str,
    value: &Value,
) {
    if value.as_bool() == Some(true) {
        let mut item = Map::new();
        item.insert(key.into(), json!(id));
        table.insert(id, item);
    }
}

fn into_array(table: BTreeMap<String, Map<String, Value>>) -> Value {
    Value::Array(table.into_values().map(Value::Object).collect())
}

fn install_schedule(event: &mut Map<String, Value>, schedule: Value) -> Result<(), CalendarError> {
    match schedule.get("allDay").and_then(Value::as_bool) {
        Some(true) => {
            let day = schedule
                .get("day")
                .and_then(Value::as_str)
                .filter(|day| day.len() == 10)
                .ok_or_else(|| invalid("invalid all-day date"))?;
            let start = format!("{day}T00:00:00.000Z");
            let days = schedule
                .get("days")
                .and_then(Value::as_i64)
                .filter(|days| *days > 0)
                .ok_or_else(|| invalid("invalid all-day duration"))?;
            let end = days
                .checked_mul(DAY_MS)
                .and_then(|duration| iso_to_millis_floor(&start)?.checked_add(duration))
                .filter(|end| *end <= 253_402_300_799_999)
                .ok_or_else(|| invalid("invalid all-day schedule"))?;
            for (key, value) in [
                ("day", json!(day)),
                ("days", json!(days)),
                ("allDay", json!(true)),
                ("start", json!(start)),
                ("end", json!(millis_to_iso(end))),
            ] {
                event.insert(key.into(), value);
            }
        }
        Some(false) => {
            for key in ["start", "end"] {
                let value = schedule
                    .get(key)
                    .filter(|value| value.is_string())
                    .ok_or_else(|| invalid("invalid timed schedule"))?;
                event.insert(key.into(), value.clone());
            }
            event.insert("allDay".into(), json!(false));
        }
        None => return Err(invalid("invalid event schedule")),
    }
    Ok(())
}

pub(super) fn materialize(fields: Fields) -> Result<CalendarData, CalendarError> {
    let mut data = Map::new();
    let mut deleted_events = Vec::new();
    let mut availability = Map::new();
    let mut withdrawals = Map::new();
    let mut rules = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut guests: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut bookings = BTreeMap::new();
    let mut subscriptions = BTreeMap::new();

    for (path, value) in fields {
        let path: Vec<_> = path.iter().map(String::as_str).collect();
        match path.as_slice() {
            ["sharing"] => {
                let mut value = value.as_object().cloned().unwrap_or_default();
                data.insert(
                    "sharedWith".into(),
                    value.remove("sharedWith").unwrap_or_else(|| json!([])),
                );
                assign(
                    &mut data,
                    "shareKey",
                    value.remove("shareKey").unwrap_or_default(),
                );
            }
            ["availability", field] => assign(&mut availability, field, value),
            ["availability", "rules", id] => initial_record(&mut rules, (*id).into(), "id", &value),
            ["availability", "rules", id, field] => {
                if let Some(rule) = rules.get_mut(*id) {
                    assign(rule, field, value);
                }
            }
            ["events", id] if value.is_null() => deleted_events.push((*id).into()),
            ["events", id] => initial_record(&mut events, (*id).into(), "id", &value),
            ["events", id, "guests", npub] if events.contains_key(*id) && !value.is_null() => {
                let mut guest = object(value)?;
                guest.insert("npub".into(), json!(npub));
                guests
                    .entry((*id).into())
                    .or_default()
                    .insert((*npub).into(), Value::Object(guest));
            }
            ["events", id, field] => {
                if let Some(event) = events.get_mut(*id) {
                    if *field == "schedule" {
                        install_schedule(event, value)?;
                    } else {
                        assign(event, field, value);
                    }
                }
            }
            ["bookingLinks", id] => initial_record(&mut bookings, (*id).into(), "id", &value),
            ["bookingLinks", id, field] => {
                if let Some(item) = bookings.get_mut(*id) {
                    assign(item, field, value);
                }
            }
            ["subscriptions", id] => {
                initial_record(&mut subscriptions, (*id).into(), "npub", &value)
            }
            ["subscriptions", id, field] => {
                if let Some(item) = subscriptions.get_mut(*id) {
                    assign(item, field, value);
                }
            }
            ["shareWithdrawals", npub] => assign(&mut withdrawals, npub, value),
            [field] => assign(&mut data, field, value),
            _ => {}
        }
    }
    for (event_id, guests) in guests {
        if let Some(event) = events.get_mut(&event_id) {
            event.insert(
                "guests".into(),
                Value::Array(guests.into_values().collect()),
            );
        }
    }
    availability.insert("rules".into(), into_array(rules));
    data.insert("availability".into(), Value::Object(availability));
    if !withdrawals.is_empty() {
        data.insert("shareWithdrawals".into(), Value::Object(withdrawals));
    }
    data.insert("events".into(), into_array(events));
    data.insert("bookingLinks".into(), into_array(bookings));
    data.insert("subscriptions".into(), into_array(subscriptions));
    let mut data: CalendarData = serde_json::from_value(Value::Object(data))?;
    data.deleted_events = deleted_events;
    data.events
        .sort_unstable_by(|a, b| (&a.start, &a.id).cmp(&(&b.start, &b.id)));
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{
        event_href_id, event_to_ics, href_event_id,
        replica::{Registers, Replica},
    };

    #[test]
    fn native_projection_matches_the_shared_web_replica_fixture() {
        let fixture: Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/calendar-replica-v1.json"
        ))
        .unwrap();
        let base = flatten(&genesis(fixture["owner"].as_str().unwrap())).unwrap();
        let replicas = fixture["replicas"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| {
                Replica::decode(
                    &serde_json::to_vec(value).unwrap(),
                    value["writer"].as_str().unwrap(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let expected = &fixture["expected"];
        for order in [[0, 1, 0], [1, 0, 1]] {
            let mut merged = Registers::default();
            for index in order {
                merged.merge(&replicas[index]);
            }
            let view = materialize(merged.overlay(base.clone())).unwrap();
            assert_eq!(json!(view.title), expected["title"]);
            assert_eq!(
                json!(
                    view.events
                        .iter()
                        .map(|event| &event.id)
                        .collect::<Vec<_>>()
                ),
                expected["eventIds"]
            );
            assert_eq!(json!(view.events[0].title), expected["firstTitle"]);
            assert_eq!(json!(view.events[0].location), expected["firstLocation"]);
            assert_eq!(view.events[1].extra["day"], expected["allDay"]);
            assert_eq!(view.events[1].extra["days"], expected["days"]);
            assert_eq!(
                event_href_id(&view.events[0].id),
                href_event_id("/caldav/calendar/uid%2Fa.ics")
            );
            let holiday = event_to_ics(&view.events[1], &view.title, 0);
            assert!(holiday.contains("DTSTART;VALUE=DATE:20261124"));
            assert!(holiday.contains("DTEND;VALUE=DATE:20261126"));
        }
    }
}
