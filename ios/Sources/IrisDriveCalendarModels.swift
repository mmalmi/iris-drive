import Foundation

struct IrisCalendarExport: Decodable {
    var calendar: IrisCalendarSnapshot?
    var error: String
}

struct IrisCalendarSnapshot: Decodable {
    var calendarId: String
    var title: String
    var ownerNpub: String
    var timezone: String
    var events: [IrisCalendarEvent]
}

struct IrisCalendarEvent: Decodable {
    var id: String
    var title: String
    var start: String
    var end: String
    var allDay: Bool
    var location: String
    var notes: String
    var recurrence: IrisCalendarRecurrence?
}

struct IrisCalendarRecurrence: Decodable {
    var frequency: String
    var interval: Int
    var until: String
}

enum IrisCalendarExportError: LocalizedError {
    case native(String)
    case empty
    case invalid

    var errorDescription: String? {
        switch self {
        case .native(let message):
            return message
        case .empty:
            return "Iris calendar export was empty"
        case .invalid:
            return "Iris calendar export returned invalid JSON"
        }
    }
}

func parseIrisCalendarExportJson(_ jsonText: String) throws -> IrisCalendarSnapshot {
    guard let data = jsonText.data(using: .utf8) else {
        throw IrisCalendarExportError.invalid
    }
    let export = try JSONDecoder().decode(IrisCalendarExport.self, from: data)
    let error = export.error.trimmingCharacters(in: .whitespacesAndNewlines)
    if !error.isEmpty {
        throw IrisCalendarExportError.native(error)
    }
    guard let calendar = export.calendar else {
        throw IrisCalendarExportError.empty
    }
    return IrisCalendarSnapshot(
        calendarId: calendar.calendarId.trimmingCharacters(in: .whitespacesAndNewlines),
        title: calendar.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "Iris Calendar"
            : calendar.title.trimmingCharacters(in: .whitespacesAndNewlines),
        ownerNpub: calendar.ownerNpub.trimmingCharacters(in: .whitespacesAndNewlines),
        timezone: calendar.timezone.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "UTC"
            : calendar.timezone.trimmingCharacters(in: .whitespacesAndNewlines),
        events: calendar.events
            .map(normalizedIrisCalendarEvent)
            .filter { !$0.id.isEmpty && !$0.start.isEmpty }
    )
}

private func normalizedIrisCalendarEvent(_ event: IrisCalendarEvent) -> IrisCalendarEvent {
    IrisCalendarEvent(
        id: event.id.trimmingCharacters(in: .whitespacesAndNewlines),
        title: event.title.trimmingCharacters(in: .whitespacesAndNewlines),
        start: event.start.trimmingCharacters(in: .whitespacesAndNewlines),
        end: event.end.trimmingCharacters(in: .whitespacesAndNewlines),
        allDay: event.allDay,
        location: event.location.trimmingCharacters(in: .whitespacesAndNewlines),
        notes: event.notes.trimmingCharacters(in: .whitespacesAndNewlines),
        recurrence: event.recurrence.map { recurrence in
            IrisCalendarRecurrence(
                frequency: recurrence.frequency.trimmingCharacters(in: .whitespacesAndNewlines),
                interval: max(1, recurrence.interval),
                until: recurrence.until.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        }
    )
}
