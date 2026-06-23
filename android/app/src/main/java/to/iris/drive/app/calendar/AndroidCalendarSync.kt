package to.iris.drive.app.calendar

import android.Manifest
import android.annotation.SuppressLint
import android.content.ContentResolver
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import androidx.core.content.ContextCompat
import java.time.Instant
import java.time.LocalDate
import java.time.OffsetDateTime
import java.time.ZoneOffset
import java.util.TimeZone
import kotlin.math.max

internal data class AndroidCalendarSyncResult(
    val calendarName: String,
    val eventsSynced: Int,
    val eventsDeleted: Int,
)

internal data class AndroidCalendarTarget(
    val accountName: String = "Iris Drive",
    val accountType: String = CalendarContract.ACCOUNT_TYPE_LOCAL,
    val calendarName: String = "iris-drive",
)

internal data class AndroidCalendarEventDraft(
    val syncId: String,
    val title: String,
    val startMillis: Long,
    val endMillis: Long,
    val allDay: Boolean,
    val timezone: String,
    val location: String,
    val description: String,
    val rrule: String?,
    val duration: String?,
)

internal object AndroidCalendarSync {
    val DefaultTarget = AndroidCalendarTarget()

    fun hasPermissions(context: Context): Boolean =
        ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.READ_CALENDAR,
        ) == PackageManager.PERMISSION_GRANTED &&
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.WRITE_CALENDAR,
            ) == PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    fun sync(
        context: Context,
        snapshot: IrisCalendarSnapshot,
        target: AndroidCalendarTarget = DefaultTarget,
    ): AndroidCalendarSyncResult {
        if (!hasPermissions(context)) {
            throw SecurityException("Calendar permission is required")
        }
        val resolver = context.contentResolver
        val calendarId = ensureCalendar(resolver, snapshot.title, target)
        val drafts = androidCalendarEventDrafts(snapshot)
        val existing = existingEventsBySyncId(resolver, calendarId)
        var synced = 0

        for (draft in drafts) {
            val values = contentValuesForDraft(calendarId, draft)
            val existingId = existing[draft.syncId]
            if (existingId == null) {
                resolver.insert(syncAdapterUri(Events.CONTENT_URI, target), values)
            } else {
                resolver.update(
                    syncAdapterUri(ContentUris.withAppendedId(Events.CONTENT_URI, existingId), target),
                    values,
                    null,
                    null,
                )
            }
            synced += 1
        }

        val wanted = drafts.mapTo(mutableSetOf()) { it.syncId }
        val deleted = deleteStaleEvents(resolver, existing, wanted, target)
        return AndroidCalendarSyncResult(
            calendarName = snapshot.title.ifBlank { "Iris Calendar" },
            eventsSynced = synced,
            eventsDeleted = deleted,
        )
    }

    private fun ensureCalendar(
        resolver: ContentResolver,
        displayName: String,
        target: AndroidCalendarTarget,
    ): Long {
        findCalendarId(resolver, target)?.let { calendarId ->
            val values = ContentValues().apply {
                put(Calendars.CALENDAR_DISPLAY_NAME, displayName.ifBlank { "Iris Calendar" })
                put(Calendars.VISIBLE, 1)
                put(Calendars.SYNC_EVENTS, 1)
            }
            resolver.update(
                syncAdapterUri(ContentUris.withAppendedId(Calendars.CONTENT_URI, calendarId), target),
                values,
                null,
                null,
            )
            return calendarId
        }

        val values = ContentValues().apply {
            put(Calendars.ACCOUNT_NAME, target.accountName)
            put(Calendars.ACCOUNT_TYPE, target.accountType)
            put(Calendars.NAME, target.calendarName)
            put(Calendars.CALENDAR_DISPLAY_NAME, displayName.ifBlank { "Iris Calendar" })
            put(Calendars.CALENDAR_COLOR, 0xFF167C80.toInt())
            put(Calendars.CALENDAR_ACCESS_LEVEL, Calendars.CAL_ACCESS_OWNER)
            put(Calendars.OWNER_ACCOUNT, target.accountName)
            put(Calendars.VISIBLE, 1)
            put(Calendars.SYNC_EVENTS, 1)
        }
        val uri = resolver.insert(syncAdapterUri(Calendars.CONTENT_URI, target), values)
            ?: throw IllegalStateException("Android calendar provider did not create a calendar")
        return ContentUris.parseId(uri)
    }

    private fun findCalendarId(resolver: ContentResolver, target: AndroidCalendarTarget): Long? =
        resolver.query(
            Calendars.CONTENT_URI,
            arrayOf(Calendars._ID),
            "${Calendars.ACCOUNT_NAME}=? AND ${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.NAME}=?",
            arrayOf(target.accountName, target.accountType, target.calendarName),
            null,
        )?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getLong(0) else null
        }

    private fun existingEventsBySyncId(
        resolver: ContentResolver,
        calendarId: Long,
    ): MutableMap<String, Long> {
        val events = mutableMapOf<String, Long>()
        resolver.query(
            Events.CONTENT_URI,
            arrayOf(Events._ID, Events._SYNC_ID),
            "${Events.CALENDAR_ID}=?",
            arrayOf(calendarId.toString()),
            null,
        )?.use { cursor ->
            while (cursor.moveToNext()) {
                val id = cursor.getLong(0)
                val syncId = cursor.getString(1)?.trim().orEmpty()
                if (syncId.isNotBlank()) {
                    events[syncId] = id
                }
            }
        }
        return events
    }

    private fun deleteStaleEvents(
        resolver: ContentResolver,
        existing: Map<String, Long>,
        wanted: Set<String>,
        target: AndroidCalendarTarget,
    ): Int {
        var deleted = 0
        for ((syncId, rowId) in existing) {
            if (syncId !in wanted) {
                deleted += resolver.delete(
                    syncAdapterUri(ContentUris.withAppendedId(Events.CONTENT_URI, rowId), target),
                    null,
                    null,
                )
            }
        }
        return deleted
    }

    private fun contentValuesForDraft(calendarId: Long, draft: AndroidCalendarEventDraft): ContentValues =
        ContentValues().apply {
            put(Events.CALENDAR_ID, calendarId)
            put(Events._SYNC_ID, draft.syncId)
            put(Events.TITLE, draft.title.ifBlank { "Untitled event" })
            put(Events.DTSTART, draft.startMillis)
            put(Events.EVENT_TIMEZONE, draft.timezone)
            put(Events.ALL_DAY, if (draft.allDay) 1 else 0)
            put(Events.EVENT_LOCATION, draft.location)
            put(Events.DESCRIPTION, draft.description)
            put(Events.STATUS, Events.STATUS_CONFIRMED)
            put(Events.AVAILABILITY, Events.AVAILABILITY_BUSY)
            if (draft.rrule == null) {
                put(Events.DTEND, draft.endMillis)
                putNull(Events.RRULE)
                putNull(Events.DURATION)
            } else {
                put(Events.RRULE, draft.rrule)
                put(Events.DURATION, draft.duration)
                putNull(Events.DTEND)
            }
        }

    internal fun syncAdapterUri(uri: Uri, target: AndroidCalendarTarget = DefaultTarget): Uri =
        uri.buildUpon()
            .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
            .appendQueryParameter(Calendars.ACCOUNT_NAME, target.accountName)
            .appendQueryParameter(Calendars.ACCOUNT_TYPE, target.accountType)
            .build()
}

internal fun androidCalendarEventDrafts(snapshot: IrisCalendarSnapshot): List<AndroidCalendarEventDraft> =
    snapshot.events.map { event ->
        val startMillis = eventMillis(event.start, event.allDay)
        val endMillis = eventMillis(event.end, event.allDay)
        val durationMillis = max(1_000L, endMillis - startMillis)
        val rrule = event.recurrence?.toAndroidRRule()
        AndroidCalendarEventDraft(
            syncId = event.id,
            title = event.title.ifBlank { "Untitled event" },
            startMillis = startMillis,
            endMillis = endMillis,
            allDay = event.allDay,
            timezone = if (event.allDay) "UTC" else snapshot.timezone.ifBlank { TimeZone.getDefault().id },
            location = event.location,
            description = event.notes,
            rrule = rrule,
            duration = rrule?.let { androidDuration(durationMillis, event.allDay) },
        )
    }

private fun IrisCalendarRecurrence.toAndroidRRule(): String? {
    val frequency = when (frequency.lowercase()) {
        "daily" -> "DAILY"
        "weekly" -> "WEEKLY"
        "monthly" -> "MONTHLY"
        "yearly" -> "YEARLY"
        else -> return null
    }
    val parts = mutableListOf("FREQ=$frequency")
    if (interval > 1) {
        parts += "INTERVAL=$interval"
    }
    if (until.isNotBlank()) {
        parts += "UNTIL=${until.replace("-", "")}"
    }
    return parts.joinToString(";")
}

private fun eventMillis(raw: String, allDay: Boolean): Long {
    val trimmed = raw.trim()
    if (trimmed.isBlank()) return 0L
    if (allDay) {
        return eventLocalDate(trimmed)
            .atStartOfDay(ZoneOffset.UTC)
            .toInstant()
            .toEpochMilli()
    }
    return runCatching { Instant.parse(trimmed).toEpochMilli() }
        .recoverCatching { OffsetDateTime.parse(trimmed).toInstant().toEpochMilli() }
        .getOrElse {
            eventLocalDate(trimmed).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
        }
}

private fun eventLocalDate(raw: String): LocalDate =
    if (raw.length >= 10) {
        LocalDate.parse(raw.substring(0, 10))
    } else {
        Instant.parse(raw).atZone(ZoneOffset.UTC).toLocalDate()
    }

private fun androidDuration(durationMillis: Long, allDay: Boolean): String =
    if (allDay) {
        "P${max(1L, durationMillis / 86_400_000L)}D"
    } else {
        "PT${max(1L, durationMillis / 1_000L)}S"
    }
