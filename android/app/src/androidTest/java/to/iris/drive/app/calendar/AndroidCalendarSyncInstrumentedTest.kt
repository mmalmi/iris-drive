package to.iris.drive.app.calendar

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidCalendarSyncInstrumentedTest {
    @get:Rule
    val calendarPermissions: GrantPermissionRule =
        GrantPermissionRule.grant(
            Manifest.permission.READ_CALENDAR,
            Manifest.permission.WRITE_CALENDAR,
        )

    @Test
    fun syncWritesEventToAndroidCalendarProvider() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        assumeTrue(AndroidCalendarSync.hasPermissions(context))
        val suffix = UUID.randomUUID().toString()
        val target = AndroidCalendarTarget(
            accountName = "Iris Drive Test $suffix",
            calendarName = "iris-drive-test-$suffix",
        )
        var calendarId: Long? = null
        try {
            val snapshot = IrisCalendarSnapshot(
                calendarId = "calendar-test-$suffix",
                title = "Iris Drive Test",
                ownerNpub = "npub1test",
                timezone = "UTC",
                events = listOf(
                    IrisCalendarEvent(
                        id = "event-test-$suffix",
                        title = "Provider smoke",
                        start = "2026-06-23T09:00:00.000Z",
                        end = "2026-06-23T09:30:00.000Z",
                        allDay = false,
                        location = "Android",
                        notes = "created by Iris Drive instrumentation",
                        recurrence = null,
                    ),
                ),
            )

            val result = AndroidCalendarSync.sync(context, snapshot, target)
            calendarId = findCalendarId(context, target)

            assertEquals(1, result.eventsSynced)
            assertNotNull(calendarId)
            assertEquals("Provider smoke", eventTitle(context, calendarId!!, "event-test-$suffix"))
        } finally {
            calendarId?.let { deleteCalendar(context, it, target) }
        }
    }

    private fun findCalendarId(context: Context, target: AndroidCalendarTarget): Long? =
        context.contentResolver.query(
            Calendars.CONTENT_URI,
            arrayOf(Calendars._ID),
            "${Calendars.ACCOUNT_NAME}=? AND ${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.NAME}=?",
            arrayOf(target.accountName, target.accountType, target.calendarName),
            null,
        )?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getLong(0) else null
        }

    private fun eventTitle(context: Context, calendarId: Long, syncId: String): String? =
        context.contentResolver.query(
            Events.CONTENT_URI,
            arrayOf(Events.TITLE),
            "${Events.CALENDAR_ID}=? AND ${Events._SYNC_ID}=?",
            arrayOf(calendarId.toString(), syncId),
            null,
        )?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }

    private fun deleteCalendar(context: Context, calendarId: Long, target: AndroidCalendarTarget) {
        context.contentResolver.delete(
            AndroidCalendarSync.syncAdapterUri(
                ContentUris.withAppendedId(Calendars.CONTENT_URI, calendarId),
                target,
            ),
            null,
            null,
        )
    }
}
