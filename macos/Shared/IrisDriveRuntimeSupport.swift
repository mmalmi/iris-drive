import Foundation
import Security

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
    private static let applicationSupportName = "Iris Drive"

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
            return shared.appendingPathComponent(applicationSupportName, isDirectory: true)
        }
        if let existing = existingAppGroupApplicationSupportDirectory() {
            return existing
        }
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return base.appendingPathComponent(applicationSupportName, isDirectory: true)
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

    private static func existingAppGroupApplicationSupportDirectory() -> URL? {
        let groupContainers = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("Group Containers", isDirectory: true)
        let containers = (try? FileManager.default.contentsOfDirectory(
            at: groupContainers,
            includingPropertiesForKeys: [.contentModificationDateKey],
            options: [.skipsHiddenFiles]
        )) ?? []

        let candidates = containers.compactMap { container -> (url: URL, modified: Date)? in
            guard container.lastPathComponent.hasSuffix(".to.iris.drive") else { return nil }
            let support = container.appendingPathComponent(applicationSupportName, isDirectory: true)
            let config = support
                .appendingPathComponent("Config", isDirectory: true)
                .appendingPathComponent("config.toml", isDirectory: false)
            guard FileManager.default.fileExists(atPath: config.path) else { return nil }
            let modified = (try? config.resourceValues(
                forKeys: [.contentModificationDateKey]
            ).contentModificationDate) ?? Date.distantPast
            return (support, modified)
        }

        return candidates.sorted { left, right in
            if left.modified == right.modified {
                return left.url.path < right.url.path
            }
            return left.modified > right.modified
        }.first?.url
    }
}

enum IrisDriveCodeSigning {
    static func currentTeamIdentifier() -> String? {
        if let entitlement = currentEntitlementString("com.apple.developer.team-identifier") {
            return entitlement
        }
        return currentStaticCodeTeamIdentifier()
    }

    static func currentEntitlementValue(_ name: String) -> Any? {
        guard let task = SecTaskCreateFromSelf(nil),
              let value = SecTaskCopyValueForEntitlement(task, name as CFString, nil)
        else {
            return nil
        }
        return value
    }

    private static func currentEntitlementString(_ name: String) -> String? {
        guard let value = currentEntitlementValue(name) as? String else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func currentStaticCodeTeamIdentifier() -> String? {
        guard let executableURL = Bundle.main.executableURL else {
            return nil
        }

        var staticCode: SecStaticCode?
        let createStatus = SecStaticCodeCreateWithPath(
            executableURL as CFURL,
            SecCSFlags(),
            &staticCode
        )
        guard createStatus == errSecSuccess, let staticCode else {
            return nil
        }

        var signingInfo: CFDictionary?
        let copyStatus = SecCodeCopySigningInformation(
            staticCode,
            SecCSFlags(rawValue: kSecCSSigningInformation),
            &signingInfo
        )
        guard copyStatus == errSecSuccess,
              let info = signingInfo as? [String: Any],
              let value = info[kSecCodeInfoTeamIdentifier as String] as? String
        else {
            return nil
        }

        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
