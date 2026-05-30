import Foundation

enum IrisDriveEnvironment {
    static func flag(_ name: String) -> Bool {
        guard let value = ProcessInfo.processInfo.environment[name]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        else {
            return false
        }
        return ["1", "true", "yes", "on"].contains(value)
    }
}

enum IrisDriveAppGroup {
    private static let appGroupName = "to.iris.drive"
    private static let legacyAppGroupIdentifier = "group.to.iris.drive"

    static func containerURL(teamIdentifier: String?) -> URL? {
        for identifier in identifiers(teamIdentifier: teamIdentifier) {
            if let url = FileManager.default.containerURL(
                forSecurityApplicationGroupIdentifier: identifier
            ) {
                return url
            }
        }
        return nil
    }

    static func applicationSupportDirectory(teamIdentifier: String?) -> URL {
        if let shared = containerURL(teamIdentifier: teamIdentifier) {
            return shared.appendingPathComponent("Iris Drive", isDirectory: true)
        }
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return base.appendingPathComponent("Iris Drive", isDirectory: true)
    }

    private static func identifiers(teamIdentifier: String?) -> [String] {
        var identifiers = [String]()
        if let teamIdentifier = teamIdentifier?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !teamIdentifier.isEmpty {
            identifiers.append("\(teamIdentifier).\(appGroupName)")
        }
        if IrisDriveEnvironment.flag("IRIS_DRIVE_ENABLE_LEGACY_APP_GROUP") {
            identifiers.append(legacyAppGroupIdentifier)
        }

        var seen = Set<String>()
        return identifiers.filter { seen.insert($0).inserted }
    }
}
