#!/usr/bin/env python3
"""Exercise Iris Drive CalDAV with python-caldav as a real client."""

from __future__ import annotations

import os
import sys
import time
import uuid

try:
    from caldav import DAVClient
except ImportError as exc:  # pragma: no cover - the shell wrapper installs this.
    raise SystemExit("python package 'caldav' is required") from exc


def main() -> int:
    url = require_env("IRIS_CALDAV_E2E_URL")
    expected_title = require_env("IRIS_CALDAV_E2E_EXPECTED_TITLE")
    client_title = require_env("IRIS_CALDAV_E2E_CLIENT_TITLE")

    try:
        client = DAVClient(
            url=url,
            timeout=5,
            features={"http.multiplexing": False},
        )
    except TypeError:
        client = DAVClient(url=url, timeout=5)
    principal = client.principal()
    calendars = principal.calendars()
    if not calendars:
        raise AssertionError(f"no calendars discovered from {url}")
    calendar = calendars[0]

    wait_for_title(calendar, expected_title, "Iris Calendar source event")

    uid = f"caldav-e2e-{uuid.uuid4().hex}@calendar.iris.to"
    calendar.save_event(client_event_ics(uid, client_title))
    wait_for_title(calendar, client_title, "CalDAV client-created event")

    print(f"discovered {len(calendars)} calendar(s)")
    print(f"observed source title: {expected_title}")
    print(f"created client title: {client_title}")
    return 0


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise AssertionError(f"{name} is required")
    return value


def wait_for_title(calendar, title: str, label: str) -> None:
    deadline = time.monotonic() + 10
    last_titles: list[str] = []
    while True:
        last_titles = event_titles(calendar)
        if title in last_titles:
            return
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"{label} {title!r} not visible through CalDAV; saw {last_titles!r}"
            )
        time.sleep(0.25)


def event_titles(calendar) -> list[str]:
    titles: list[str] = []
    for event in calendar.events():
        data = event_data(event)
        for line in unfolded_lines(data):
            if line.upper().startswith("SUMMARY:"):
                titles.append(unescape_ics_text(line.split(":", 1)[1]))
    return titles


def event_data(event) -> str:
    data = getattr(event, "data", None)
    if not data:
        event.load()
        data = getattr(event, "data", None)
    if isinstance(data, bytes):
        return data.decode("utf-8", "replace")
    return "" if data is None else str(data)


def unfolded_lines(text: str) -> list[str]:
    lines: list[str] = []
    for raw in text.replace("\r\n", "\n").replace("\r", "\n").split("\n"):
        if raw.startswith((" ", "\t")) and lines:
            lines[-1] += raw[1:]
        elif raw.strip():
            lines.append(raw)
    return lines


def unescape_ics_text(value: str) -> str:
    return (
        value.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
    )


def client_event_ics(uid: str, title: str) -> str:
    return "\r\n".join(
        [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Iris Drive//CalDAV E2E Client//EN",
            "BEGIN:VEVENT",
            f"UID:{uid}",
            "DTSTAMP:20260706T120000Z",
            f"SUMMARY:{escape_ics_text(title)}",
            "DTSTART:20260821T130000Z",
            "DTEND:20260821T133000Z",
            "LOCATION:python-caldav",
            "DESCRIPTION:created by python-caldav during Iris Calendar interop e2e",
            "END:VEVENT",
            "END:VCALENDAR",
            "",
        ]
    )


def escape_ics_text(value: str) -> str:
    return (
        value.replace("\\", "\\\\")
        .replace("\n", "\\n")
        .replace(";", "\\;")
        .replace(",", "\\,")
    )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # pragma: no cover - prints useful test stderr.
        print(f"caldav e2e failed: {exc}", file=sys.stderr)
        raise
