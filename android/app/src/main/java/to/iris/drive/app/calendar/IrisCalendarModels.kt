package to.iris.drive.app.calendar

import org.json.JSONObject

internal data class IrisCalendarSnapshot(
    val calendarId: String,
    val title: String,
    val ownerNpub: String,
    val timezone: String,
    val events: List<IrisCalendarEvent>,
)

internal data class IrisCalendarEvent(
    val id: String,
    val title: String,
    val start: String,
    val end: String,
    val allDay: Boolean,
    val location: String,
    val notes: String,
    val recurrence: IrisCalendarRecurrence?,
)

internal data class IrisCalendarRecurrence(
    val frequency: String,
    val interval: Int,
    val until: String,
)

internal fun parseIrisCalendarExportJson(jsonText: String): IrisCalendarSnapshot {
    val root = JSONObject(jsonText)
    val error = root.optString("error").trim()
    if (error.isNotEmpty()) {
        throw IllegalStateException(error)
    }
    val calendar = root.optJSONObject("calendar")
        ?: throw IllegalStateException("Iris calendar export was empty")
    val eventsJson = calendar.optJSONArray("events")
    val events = buildList {
        if (eventsJson != null) {
            for (index in 0 until eventsJson.length()) {
                val event = eventsJson.optJSONObject(index) ?: continue
                val recurrenceJson = event.optJSONObject("recurrence")
                add(
                    IrisCalendarEvent(
                        id = event.optString("id").trim(),
                        title = event.optString("title").trim(),
                        start = event.optString("start").trim(),
                        end = event.optString("end").trim(),
                        allDay = event.optBoolean("allDay"),
                        location = event.optString("location").trim(),
                        notes = event.optString("notes").trim(),
                        recurrence = recurrenceJson?.let {
                            IrisCalendarRecurrence(
                                frequency = it.optString("frequency").trim(),
                                interval = it.optInt("interval", 1).coerceAtLeast(1),
                                until = it.optString("until").trim(),
                            )
                        },
                    ),
                )
            }
        }
    }.filter { it.id.isNotBlank() && it.start.isNotBlank() }

    return IrisCalendarSnapshot(
        calendarId = calendar.optString("calendarId").trim(),
        title = calendar.optString("title").trim().ifBlank { "Iris Calendar" },
        ownerNpub = calendar.optString("ownerNpub").trim(),
        timezone = calendar.optString("timezone").trim().ifBlank { "UTC" },
        events = events,
    )
}
