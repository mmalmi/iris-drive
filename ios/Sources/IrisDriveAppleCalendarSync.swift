import CryptoKit
import EventKit
import Foundation
import UIKit

struct AppleCalendarSyncResult {
    var synced = false
    var unchanged = false
    var skippedReason = ""
    var eventsSynced = 0
    var eventsDeleted = 0
    var error = ""
}

@MainActor
final class IrisDriveAppleCalendarSync {
    static let shared = IrisDriveAppleCalendarSync()

    private let eventStore = EKEventStore()
    private let defaults = UserDefaults.standard
    private let enabledKey = "appleCalendarSyncEnabled"
    private let lastFingerprintKey = "appleCalendarLastFingerprint"
    private let calendarIdentifierKey = "appleCalendarIdentifier"
    private let eventURLScheme = "iris-drive"
    private let eventURLHost = "calendar-event"
    private let calendarTitle = "Iris Drive"

    private init() {}

    var isEnabled: Bool {
        defaults.bool(forKey: enabledKey)
    }

    var isFullAccessGranted: Bool {
        EKEventStore.authorizationStatus(for: .event) == .fullAccess
    }

    var isActive: Bool {
        isEnabled && isFullAccessGranted
    }

    var accessStatusLabel: String {
        switch EKEventStore.authorizationStatus(for: .event) {
        case .notDetermined:
            return "Permission not requested"
        case .restricted:
            return "Calendar access is restricted"
        case .denied:
            return "Calendar access denied"
        case .writeOnly:
            return "Full calendar access required"
        case .fullAccess:
            return isEnabled ? "Continuous sync enabled" : "Off"
        @unknown default:
            return "Calendar access unavailable"
        }
    }

    func setEnabled(_ enabled: Bool) {
        defaults.set(enabled, forKey: enabledKey)
    }

    func requestFullAccess() async throws -> Bool {
        if isFullAccessGranted {
            return true
        }
        return try await withCheckedThrowingContinuation { continuation in
            eventStore.requestFullAccessToEvents { granted, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: granted)
                }
            }
        }
    }

    func syncIfEnabled(
        dataDir: String,
        isSetupComplete: Bool,
        force: Bool = false
    ) -> AppleCalendarSyncResult {
        guard isSetupComplete else {
            return AppleCalendarSyncResult(skippedReason: "not_setup")
        }
        guard isEnabled else {
            return AppleCalendarSyncResult(skippedReason: "disabled")
        }
        guard isFullAccessGranted else {
            return AppleCalendarSyncResult(skippedReason: "permission_required")
        }

        do {
            let exportJson = IrisDriveNativeCore.exportCalendarJson(dataDir: dataDir)
            return try sync(exportJson: exportJson, force: force)
        } catch {
            return AppleCalendarSyncResult(
                error: error.localizedDescription.isEmpty
                    ? "Apple Calendar sync failed"
                    : error.localizedDescription
            )
        }
    }

    func sync(exportJson: String, force: Bool = false) throws -> AppleCalendarSyncResult {
        let fingerprint = calendarFingerprint(exportJson)
        if !force && defaults.string(forKey: lastFingerprintKey) == fingerprint {
            return AppleCalendarSyncResult(unchanged: true)
        }

        let snapshot = try parseIrisCalendarExportJson(exportJson)
        let calendar = try ensureCalendar()
        let drafts = snapshot.events.map { AppleCalendarEventDraft(event: $0, snapshot: snapshot) }
        let existing = existingIrisEvents(in: calendar)
        var synced = 0

        for draft in drafts {
            let event = existing[draft.irisId] ?? EKEvent(eventStore: eventStore)
            apply(draft: draft, to: event, calendar: calendar)
            try eventStore.save(event, span: .futureEvents, commit: false)
            synced += 1
        }

        let wanted = Set(drafts.map(\.irisId))
        var deleted = 0
        for (irisId, event) in existing where !wanted.contains(irisId) {
            try eventStore.remove(event, span: .futureEvents, commit: false)
            deleted += 1
        }

        if synced > 0 || deleted > 0 {
            try eventStore.commit()
        }
        defaults.set(fingerprint, forKey: lastFingerprintKey)
        return AppleCalendarSyncResult(
            synced: true,
            eventsSynced: synced,
            eventsDeleted: deleted
        )
    }

    func calendarFingerprint(_ value: String) -> String {
        let digest = SHA256.hash(data: Data(value.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    private func ensureCalendar() throws -> EKCalendar {
        if let identifier = defaults.string(forKey: calendarIdentifierKey),
           let calendar = eventStore.calendar(withIdentifier: identifier) {
            calendar.title = calendarTitle
            if calendar.cgColor == nil {
                calendar.cgColor = UIColor.systemTeal.cgColor
            }
            try eventStore.saveCalendar(calendar, commit: true)
            return calendar
        }

        if let existing = eventStore.calendars(for: .event).first(where: { calendar in
            calendar.title == calendarTitle
        }) {
            defaults.set(existing.calendarIdentifier, forKey: calendarIdentifierKey)
            return existing
        }

        guard let source = writableCalendarSource() else {
            throw AppleCalendarSyncError.noWritableSource
        }
        let calendar = EKCalendar(for: .event, eventStore: eventStore)
        calendar.title = calendarTitle
        calendar.cgColor = UIColor.systemTeal.cgColor
        calendar.source = source
        try eventStore.saveCalendar(calendar, commit: true)
        defaults.set(calendar.calendarIdentifier, forKey: calendarIdentifierKey)
        return calendar
    }

    private func writableCalendarSource() -> EKSource? {
        if let source = eventStore.defaultCalendarForNewEvents?.source {
            return source
        }
        if let local = eventStore.sources.first(where: { $0.sourceType == .local }) {
            return local
        }
        return eventStore.sources.first { source in
            source.calendars(for: .event).contains { $0.allowsContentModifications }
        }
    }

    private func existingIrisEvents(in calendar: EKCalendar) -> [String: EKEvent] {
        let window = broadEventSearchWindow()
        let predicate = eventStore.predicateForEvents(
            withStart: window.start,
            end: window.end,
            calendars: [calendar]
        )
        var events: [String: EKEvent] = [:]
        for event in eventStore.events(matching: predicate) {
            guard let irisId = irisEventID(from: event.url) else { continue }
            events[irisId] = event
        }
        return events
    }

    private func apply(
        draft: AppleCalendarEventDraft,
        to event: EKEvent,
        calendar: EKCalendar
    ) {
        event.calendar = calendar
        event.title = draft.title.isEmpty ? "Untitled event" : draft.title
        event.startDate = draft.startDate
        event.endDate = draft.endDate
        event.isAllDay = draft.allDay
        event.location = draft.location.isEmpty ? nil : draft.location
        event.notes = draft.notes.isEmpty ? nil : draft.notes
        event.timeZone = draft.timeZone
        event.availability = .busy
        event.url = irisEventURL(for: draft.irisId)
        event.recurrenceRules = draft.recurrenceRule.map { [$0] }
    }

    private func irisEventURL(for id: String) -> URL? {
        var components = URLComponents()
        components.scheme = eventURLScheme
        components.host = eventURLHost
        components.queryItems = [URLQueryItem(name: "id", value: id)]
        return components.url
    }

    private func irisEventID(from url: URL?) -> String? {
        guard let url,
              url.scheme == eventURLScheme,
              url.host == eventURLHost,
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else {
            return nil
        }
        return components.queryItems?
            .first(where: { $0.name == "id" })?
            .value?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .nilIfEmpty
    }

    private func broadEventSearchWindow() -> (start: Date, end: Date) {
        let calendar = Calendar(identifier: .gregorian)
        let start = calendar.date(from: DateComponents(year: 1970, month: 1, day: 1))
            ?? Date(timeIntervalSince1970: 0)
        let end = calendar.date(from: DateComponents(year: 2100, month: 1, day: 1))
            ?? Date(timeIntervalSince1970: 4_102_444_800)
        return (start, end)
    }
}

private struct AppleCalendarEventDraft {
    var irisId: String
    var title: String
    var startDate: Date
    var endDate: Date
    var allDay: Bool
    var timeZone: TimeZone?
    var location: String
    var notes: String
    var recurrenceRule: EKRecurrenceRule?

    init(event: IrisCalendarEvent, snapshot: IrisCalendarSnapshot) {
        irisId = event.id
        title = event.title
        allDay = event.allDay
        location = event.location
        notes = event.notes
        timeZone = event.allDay ? nil : TimeZone(identifier: snapshot.timezone) ?? .current

        let parsedStart = appleCalendarEventDate(event.start, allDay: event.allDay)
        var parsedEnd = appleCalendarEventDate(event.end, allDay: event.allDay)
        if parsedEnd <= parsedStart {
            parsedEnd = Calendar.current.date(
                byAdding: event.allDay ? .day : .hour,
                value: 1,
                to: parsedStart
            ) ?? parsedStart.addingTimeInterval(event.allDay ? 86_400 : 3_600)
        }
        startDate = parsedStart
        endDate = parsedEnd
        recurrenceRule = event.recurrence.flatMap { recurrence in
            appleCalendarRecurrenceRule(recurrence, allDay: event.allDay)
        }
    }
}

private enum AppleCalendarSyncError: LocalizedError {
    case noWritableSource

    var errorDescription: String? {
        switch self {
        case .noWritableSource:
            return "No writable Apple Calendar account is available"
        }
    }
}

private func appleCalendarEventDate(_ raw: String, allDay: Bool) -> Date {
    let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    if allDay {
        return appleCalendarLocalDate(value)
    }
    if let date = internetDateFormatter.date(from: value) {
        return date
    }
    if let date = fractionalInternetDateFormatter.date(from: value) {
        return date
    }
    if let date = offsetDateFormatter.date(from: value) {
        return date
    }
    return appleCalendarLocalDate(value)
}

private func appleCalendarLocalDate(_ raw: String) -> Date {
    let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    let datePart = String(trimmed.prefix(10))
    let parts = datePart.split(separator: "-").compactMap { Int($0) }
    if parts.count == 3 {
        var components = DateComponents()
        components.calendar = Calendar(identifier: .gregorian)
        components.year = parts[0]
        components.month = parts[1]
        components.day = parts[2]
        return components.date ?? Date(timeIntervalSince1970: 0)
    }
    return Date(timeIntervalSince1970: 0)
}

private func appleCalendarRecurrenceRule(
    _ recurrence: IrisCalendarRecurrence,
    allDay: Bool
) -> EKRecurrenceRule? {
    let frequency: EKRecurrenceFrequency
    switch recurrence.frequency.lowercased() {
    case "daily":
        frequency = .daily
    case "weekly":
        frequency = .weekly
    case "monthly":
        frequency = .monthly
    case "yearly":
        frequency = .yearly
    default:
        return nil
    }
    let end = recurrence.until.isEmpty
        ? nil
        : EKRecurrenceEnd(end: appleCalendarEventDate(recurrence.until, allDay: allDay))
    return EKRecurrenceRule(
        recurrenceWith: frequency,
        interval: max(1, recurrence.interval),
        daysOfTheWeek: nil,
        daysOfTheMonth: nil,
        monthsOfTheYear: nil,
        weeksOfTheYear: nil,
        daysOfTheYear: nil,
        setPositions: nil,
        end: end
    )
}

private let internetDateFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime]
    return formatter
}()

private let fractionalInternetDateFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private let offsetDateFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.dateFormat = "yyyy-MM-dd'T'HH:mm:ssXXXXX"
    return formatter
}()

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
