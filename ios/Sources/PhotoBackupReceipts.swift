import Foundation
import SQLite3

/// A receipt means the original was committed to this profile's local Drive.
final class PhotoBackupReceipts {
    private var db: OpaquePointer?
    private var lookup: OpaquePointer?
    private var insert: OpaquePointer?
    private let profile: String

    init(url: URL, profile: String) throws {
        self.profile = profile
        guard sqlite3_open(url.path, &db) == SQLITE_OK else {
            let error = databaseError()
            close()
            throw error
        }
        do {
            guard sqlite3_exec(db, "CREATE TABLE IF NOT EXISTS saved (profile TEXT, resource TEXT, PRIMARY KEY(profile, resource)) WITHOUT ROWID", nil, nil, nil) == SQLITE_OK,
                  sqlite3_prepare_v2(db, "SELECT 1 FROM saved WHERE profile=? AND resource=?", -1, &lookup, nil) == SQLITE_OK,
                  sqlite3_prepare_v2(db, "INSERT OR IGNORE INTO saved VALUES (?, ?)", -1, &insert, nil) == SQLITE_OK else {
                throw databaseError()
            }
        } catch {
            close()
            throw error
        }
    }

    deinit { close() }

    func contains(_ resource: String) throws -> Bool {
        try step(lookup, resource) == SQLITE_ROW
    }

    func remember(_ resource: String) throws {
        _ = try step(insert, resource)
    }

    private func step(_ statement: OpaquePointer?, _ resource: String) throws -> Int32 {
        defer { sqlite3_reset(statement) }
        let copy = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        guard sqlite3_bind_text(statement, 1, profile, -1, copy) == SQLITE_OK,
              sqlite3_bind_text(statement, 2, resource, -1, copy) == SQLITE_OK else { throw databaseError() }
        let result = sqlite3_step(statement)
        guard result == SQLITE_ROW || result == SQLITE_DONE else { throw databaseError() }
        return result
    }

    private func databaseError() -> NSError {
        NSError(domain: "IrisDrivePhotoBackup", code: 1, userInfo: [NSLocalizedDescriptionKey: "Couldn’t save the photo backup history."])
    }

    private func close() {
        sqlite3_finalize(lookup)
        sqlite3_finalize(insert)
        sqlite3_close(db)
        lookup = nil
        insert = nil
        db = nil
    }
}
