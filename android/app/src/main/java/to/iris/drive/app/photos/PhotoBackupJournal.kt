package to.iris.drive.app.photos

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import java.io.File

/** Device-local consent and receipts must never migrate with Android Auto Backup. */
internal class PhotoBackupJournal(
    context: Context,
    path: File = File(context.noBackupFilesDir, "photo-backup.db"),
) : SQLiteOpenHelper(context, path.absolutePath, null, 2) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("CREATE TABLE consent (id INTEGER PRIMARY KEY CHECK(id = 1), profile TEXT NOT NULL, status TEXT NOT NULL, allow_metered INTEGER NOT NULL DEFAULT 0)")
        db.execSQL("CREATE TABLE receipt (profile TEXT NOT NULL, source TEXT NOT NULL, revision TEXT NOT NULL, PRIMARY KEY(profile, source)) WITHOUT ROWID")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        check(oldVersion == 1 && newVersion == 2)
        db.execSQL("ALTER TABLE consent ADD COLUMN allow_metered INTEGER NOT NULL DEFAULT 0")
    }

    fun enabledProfile(): String? =
        readableDatabase.rawQuery("SELECT profile FROM consent WHERE id=1", null).use {
            if (it.moveToFirst()) it.getString(0) else null
        }

    fun enable(profile: String) {
        writableDatabase.execSQL("INSERT OR REPLACE INTO consent(id, profile, status) VALUES(1, ?, 'Waiting to back up')", arrayOf(profile))
    }

    fun disable() { writableDatabase.delete("consent", null, null) }

    fun allowMetered(profile: String): Boolean =
        readableDatabase.rawQuery("SELECT allow_metered FROM consent WHERE profile=?", arrayOf(profile)).use {
            it.moveToFirst() && it.getInt(0) != 0
        }

    fun setAllowMetered(profile: String, allowed: Boolean) {
        val values = ContentValues().apply { put("allow_metered", allowed) }
        writableDatabase.update("consent", values, "profile=?", arrayOf(profile))
    }

    fun setStatus(profile: String, status: String) {
        val values = ContentValues().apply { put("status", status) }
        writableDatabase.update("consent", values, "profile=?", arrayOf(profile))
    }

    fun status(profile: String): String? =
        readableDatabase.rawQuery("SELECT status FROM consent WHERE profile=?", arrayOf(profile)).use {
            if (it.moveToFirst()) it.getString(0) else null
        }

    fun contains(profile: String, source: String, revision: String): Boolean =
        readableDatabase.rawQuery("SELECT 1 FROM receipt WHERE profile=? AND source=? AND revision=?", arrayOf(profile, source, revision)).use {
            it.moveToFirst()
        }

    fun record(profile: String, source: String, revision: String) {
        writableDatabase.execSQL("INSERT OR REPLACE INTO receipt VALUES(?, ?, ?)", arrayOf(profile, source, revision))
    }

    fun count(profile: String): Long =
        readableDatabase.rawQuery("SELECT COUNT(*) FROM receipt WHERE profile=?", arrayOf(profile)).use {
            it.moveToFirst()
            it.getLong(0)
        }
}
