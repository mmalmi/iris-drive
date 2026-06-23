package to.iris.drive.app.calendar

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class IrisCalendarModelsTest {
    @Test
    fun parsesCalendarExportAndBuildsAndroidEventDrafts() {
        val snapshot = parseIrisCalendarExportJson(
            """
            {
              "calendar": {
                "calendarId": "calendar-1",
                "title": "Work",
                "ownerNpub": "npub1owner",
                "timezone": "Europe/Helsinki",
                "events": [{
                  "id": "event-1",
                  "title": "Planning",
                  "start": "2026-06-23T09:00:00.000Z",
                  "end": "2026-06-23T10:30:00.000Z",
                  "allDay": false,
                  "location": "Helsinki",
                  "notes": "Bring agenda",
                  "recurrence": {"frequency": "weekly", "interval": 2, "until": "2026-08-01"}
                }]
              },
              "error": ""
            }
            """.trimIndent(),
        )

        assertEquals("Work", snapshot.title)
        assertEquals("Europe/Helsinki", snapshot.timezone)
        assertEquals("Planning", snapshot.events.single().title)

        val draft = androidCalendarEventDrafts(snapshot).single()
        assertEquals("event-1", draft.syncId)
        assertEquals("Planning", draft.title)
        assertEquals(1_782_205_200_000L, draft.startMillis)
        assertEquals(1_782_210_600_000L, draft.endMillis)
        assertEquals("Europe/Helsinki", draft.timezone)
        assertEquals("FREQ=WEEKLY;INTERVAL=2;UNTIL=20260801", draft.rrule)
        assertEquals("PT5400S", draft.duration)
    }

    @Test
    fun allDayEventsUseUtcDateBounds() {
        val snapshot = IrisCalendarSnapshot(
            calendarId = "calendar-1",
            title = "Calendar",
            ownerNpub = "npub1owner",
            timezone = "Europe/Helsinki",
            events = listOf(
                IrisCalendarEvent(
                    id = "all-day",
                    title = "Holiday",
                    start = "2026-06-23T00:00:00.000Z",
                    end = "2026-06-24T00:00:00.000Z",
                    allDay = true,
                    location = "",
                    notes = "",
                    recurrence = null,
                ),
            ),
        )

        val draft = androidCalendarEventDrafts(snapshot).single()

        assertEquals(true, draft.allDay)
        assertEquals("UTC", draft.timezone)
        assertEquals(1_782_172_800_000L, draft.startMillis)
        assertEquals(1_782_259_200_000L, draft.endMillis)
        assertNull(draft.rrule)
        assertNull(draft.duration)
    }

    @Test(expected = IllegalStateException::class)
    fun nativeExportErrorsAreSurfaced() {
        parseIrisCalendarExportJson("""{"error":"calendar unavailable"}""")
    }
}
