#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IRIS_CALENDAR_DIR="${IRIS_CALENDAR_DIR:-"$HOME/src/iris-calendar"}"
PYTHON_BIN="${PYTHON:-python3}"

if [[ ! -f "$IRIS_CALENDAR_DIR/package.json" ]]; then
  echo "iris-calendar repo not found at $IRIS_CALENDAR_DIR" >&2
  exit 1
fi
VITEST_BIN="${VITEST_BIN:-"$IRIS_CALENDAR_DIR/node_modules/.bin/vitest"}"
if [[ ! -x "$VITEST_BIN" ]]; then
  echo "vitest not found at $VITEST_BIN; run pnpm install in $IRIS_CALENDAR_DIR first" >&2
  exit 1
fi

tmp="$(mktemp -d)"
run_id="$(date +%s)-$$"
seed_test="$IRIS_CALENDAR_DIR/tests/caldav-e2e-seed-$run_id.test.ts"
verify_test="$IRIS_CALENDAR_DIR/tests/caldav-e2e-verify-$run_id.test.ts"
cleanup() {
  rm -f "$seed_test" "$verify_test"
  rm -rf "$tmp"
}
trap cleanup EXIT

seed_json="$tmp/iris-calendar-seed.json"
output_json="$tmp/iris-drive-after-caldav.json"
venv="$tmp/venv"

app_title="${IRIS_DRIVE_CALDAV_E2E_APP_TITLE:-Iris Calendar source -> CalDAV e2e}"
client_title="${IRIS_DRIVE_CALDAV_E2E_CLIENT_TITLE:-python-caldav -> Iris Calendar e2e}"
owner_npub="${IRIS_DRIVE_CALDAV_E2E_OWNER_NPUB:-npub1caldavirissourcee2e}"
calendar_import="${IRIS_CALENDAR_DIR}/src/lib/calendar"

cat >"$seed_test" <<EOF
import { describe, expect, it } from 'vitest';
import { writeFileSync } from 'node:fs';
import {
  createInitialCalendarData,
  eventsForDay,
  serializeCalendarData,
  upsertCalendarEvent,
} from '${calendar_import}';

describe('Iris Calendar CalDAV e2e seed', () => {
  it('writes canonical calendar JSON with an app-created event', () => {
    const output = process.env.IRIS_DRIVE_CALDAV_E2E_SEED_JSON;
    const title = process.env.IRIS_DRIVE_CALDAV_E2E_APP_TITLE;
    const owner = process.env.IRIS_DRIVE_CALDAV_E2E_OWNER_NPUB ?? 'npub1caldavirissourcee2e';
    expect(output).toBeTruthy();
    expect(title).toBeTruthy();

    let data = createInitialCalendarData(owner, 1783296000000);
    data = upsertCalendarEvent(data, {
      title: title!,
      start: '2026-08-20T09:00',
      end: '2026-08-20T09:30',
      color: 'green',
      location: 'Iris Calendar source',
      notes: 'created by Iris Calendar source for CalDAV e2e',
    }, owner, 1783296001000);

    expect(data.events.map(event => event.title)).toContain(title);
    expect(eventsForDay(data.events, new Date('2026-08-20T12:00:00Z')).map(event => event.title))
      .toContain(title);
    writeFileSync(output!, serializeCalendarData(data));
  });
});
EOF

cat >"$verify_test" <<EOF
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { eventsForDay, parseCalendarData } from '${calendar_import}';

describe('Iris Calendar CalDAV e2e verify', () => {
  it('parses CalDAV client changes from canonical calendar JSON', () => {
    const input = process.env.IRIS_DRIVE_CALDAV_E2E_OUTPUT_JSON;
    const appTitle = process.env.IRIS_DRIVE_CALDAV_E2E_APP_TITLE;
    const clientTitle = process.env.IRIS_DRIVE_CALDAV_E2E_CLIENT_TITLE;
    const owner = process.env.IRIS_DRIVE_CALDAV_E2E_OWNER_NPUB ?? 'npub1caldavirissourcee2e';
    expect(input).toBeTruthy();
    expect(appTitle).toBeTruthy();
    expect(clientTitle).toBeTruthy();

    const data = parseCalendarData(readFileSync(input!), owner);
    expect(data).not.toBeNull();
    const titles = data!.events.map(event => event.title);
    expect(titles).toContain(appTitle);
    expect(titles).toContain(clientTitle);
    expect(eventsForDay(data!.events, new Date('2026-08-21T12:00:00Z')).map(event => event.title))
      .toContain(clientTitle);
  });
});
EOF

echo "[caldav-e2e] generating Iris Calendar source fixture"
IRIS_DRIVE_CALDAV_E2E_SEED_JSON="$seed_json" \
IRIS_DRIVE_CALDAV_E2E_APP_TITLE="$app_title" \
IRIS_DRIVE_CALDAV_E2E_OWNER_NPUB="$owner_npub" \
  bash -c 'cd "$1" && "$2" run "$3" --config "$1/vitest.config.ts"' \
    bash "$IRIS_CALENDAR_DIR" "$VITEST_BIN" "$seed_test"

echo "[caldav-e2e] installing python-caldav client in a temp venv"
"$PYTHON_BIN" -m venv "$venv"
"$venv/bin/python" -m pip install --disable-pip-version-check --quiet 'caldav==1.3.9'

echo "[caldav-e2e] running Iris Drive gateway against python-caldav"
IRIS_DRIVE_CALDAV_E2E_SEED_JSON="$seed_json" \
IRIS_DRIVE_CALDAV_E2E_OUTPUT_JSON="$output_json" \
IRIS_DRIVE_CALDAV_E2E_CLIENT_SCRIPT="$ROOT/scripts/caldav-real-client-e2e.py" \
IRIS_DRIVE_CALDAV_E2E_PYTHON="$venv/bin/python" \
IRIS_DRIVE_CALDAV_E2E_APP_TITLE="$app_title" \
IRIS_DRIVE_CALDAV_E2E_CLIENT_TITLE="$client_title" \
  cargo test -p iris-drive-core --test caldav_iris_calendar_e2e \
    real_caldav_client_round_trips_with_iris_calendar_source_json -- --exact --nocapture

echo "[caldav-e2e] verifying CalDAV output with Iris Calendar source parser"
IRIS_DRIVE_CALDAV_E2E_OUTPUT_JSON="$output_json" \
IRIS_DRIVE_CALDAV_E2E_APP_TITLE="$app_title" \
IRIS_DRIVE_CALDAV_E2E_CLIENT_TITLE="$client_title" \
IRIS_DRIVE_CALDAV_E2E_OWNER_NPUB="$owner_npub" \
  bash -c 'cd "$1" && "$2" run "$3" --config "$1/vitest.config.ts"' \
    bash "$IRIS_CALENDAR_DIR" "$VITEST_BIN" "$verify_test"

echo "[caldav-e2e] ok: Iris Calendar source event and python-caldav event both round-tripped"
