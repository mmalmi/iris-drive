import Foundation
import UIKit

private let iosDebugStateFileName = "debug-state.json"
private let configMutationAuditDefaultsKey = "configMutationAuditV1"
private let configMutationAuditMaxEvents = 20
private let foregroundSyncAuditDefaultsKey = "foregroundSyncAuditV1"
private let foregroundSyncAuditMaxEvents = 40

struct ConfigIdentitySnapshot: Codable {
    var hasProfile: Bool
    var setupState: String
    var profileId: String
    var currentAppKeyNpub: String
    var currentAppKeyLabel: String
}

private struct ConfigMutationAuditEvent: Codable {
    var timestamp: String
    var action: String
    var debugAction: String
    var before: ConfigIdentitySnapshot
    var after: ConfigIdentitySnapshot
    var error: String
}

private struct ForegroundSyncAuditEvent: Codable {
    var timestamp: String
    var phase: String
    var pendingBefore: UInt64?
    var pendingAfter: UInt64?
    var syncRunning: Bool
    var appActive: Bool
    var hadError: Bool
}

@MainActor
extension IrisDriveMobileModel {
    func writeDebugState(_ json: String) {
        #if DEBUG
        writeDebugState(
            json,
            to: IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent(iosDebugStateFileName, isDirectory: false)
        )
        if let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
            writeDebugState(
                json,
                to: documents.appendingPathComponent(iosDebugStateFileName, isDirectory: false)
            )
        }
        #endif
    }

    private func writeDebugState(_ json: String, to url: URL) {
        #if DEBUG
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let debugJson = jsonWithConfigMutationAudit(json)
        try? debugJson.write(to: url, atomically: true, encoding: .utf8)
        #endif
    }

    func configIdentitySnapshot() -> ConfigIdentitySnapshot {
        let profile = lastState?.ui.profile
        return ConfigIdentitySnapshot(
            hasProfile: profile != nil,
            setupState: lastState?.ui.setupState ?? "",
            profileId: profile?.profileId ?? "",
            currentAppKeyNpub: profile?.currentAppKeyNpub ?? "",
            currentAppKeyLabel: profile?.appKeyLabel ?? ""
        )
    }

    func recordConfigMutation(action: String, before: ConfigIdentitySnapshot) {
        let event = ConfigMutationAuditEvent(
            timestamp: ISO8601DateFormatter().string(from: Date()),
            action: action,
            debugAction: ProcessInfo.processInfo.environment["IRIS_DRIVE_DEBUG_ACTION"] ?? "",
            before: before,
            after: configIdentitySnapshot(),
            error: lastState?.error ?? ""
        )
        var events = configMutationAuditEvents()
        events.append(event)
        if events.count > configMutationAuditMaxEvents {
            events.removeFirst(events.count - configMutationAuditMaxEvents)
        }
        guard let data = try? JSONEncoder().encode(events) else { return }
        defaults.set(data, forKey: configMutationAuditDefaultsKey)
        writeDebugState(nativeStateJsonForAudit())
    }

    private func configMutationAuditEvents() -> [ConfigMutationAuditEvent] {
        guard let data = defaults.data(forKey: configMutationAuditDefaultsKey),
              let events = try? JSONDecoder().decode([ConfigMutationAuditEvent].self, from: data)
        else {
            return []
        }
        return events
    }

    private func jsonWithConfigMutationAudit(_ json: String) -> String {
        guard let data = json.data(using: .utf8),
              var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else {
            return json
        }
        if let auditData = try? JSONEncoder().encode(configMutationAuditEvents()),
           let audit = try? JSONSerialization.jsonObject(with: auditData) {
            object["ios_config_mutation_audit"] = audit
        }
        if let auditData = try? JSONEncoder().encode(foregroundSyncAuditEvents()),
           let audit = try? JSONSerialization.jsonObject(with: auditData) {
            object["ios_foreground_sync_audit"] = audit
        }
        guard let output = try? JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys]
        ) else {
            return json
        }
        return String(data: output, encoding: .utf8) ?? json
    }

    func recordForegroundSyncAudit(
        phase: String,
        pendingBefore: UInt64?,
        pendingAfter: UInt64?
    ) {
        #if DEBUG
        appendForegroundSyncAudit(
            phase: phase,
            pendingBefore: pendingBefore,
            pendingAfter: pendingAfter
        )
        if !lastAppliedStateJson.isEmpty {
            writeDebugState(lastAppliedStateJson)
        }
        #endif
    }

    func appendForegroundSyncAudit(
        phase: String,
        pendingBefore: UInt64?,
        pendingAfter: UInt64?
    ) {
        #if DEBUG
        let event = ForegroundSyncAuditEvent(
            timestamp: ISO8601DateFormatter().string(from: Date()),
            phase: phase,
            pendingBefore: pendingBefore,
            pendingAfter: pendingAfter,
            syncRunning: syncRunning,
            appActive: UIApplication.shared.applicationState == .active,
            hadError: !(lastState?.error.isEmpty ?? true)
        )
        var events = foregroundSyncAuditEvents()
        events.append(event)
        if events.count > foregroundSyncAuditMaxEvents {
            events.removeFirst(events.count - foregroundSyncAuditMaxEvents)
        }
        guard let data = try? JSONEncoder().encode(events) else { return }
        defaults.set(data, forKey: foregroundSyncAuditDefaultsKey)
        NSLog(
            "Iris Drive foreground sync phase=%@ pending_before=%@ pending_after=%@ " +
                "sync_running=%@ app_active=%@ error=%@",
            phase,
            pendingBefore.map(String.init) ?? "unknown",
            pendingAfter.map(String.init) ?? "unknown",
            syncRunning ? "true" : "false",
            event.appActive ? "true" : "false",
            event.hadError ? "true" : "false"
        )
        #endif
    }

    private func foregroundSyncAuditEvents() -> [ForegroundSyncAuditEvent] {
        #if DEBUG
        guard let data = defaults.data(forKey: foregroundSyncAuditDefaultsKey),
              let events = try? JSONDecoder().decode([ForegroundSyncAuditEvent].self, from: data)
        else {
            return []
        }
        return events
        #else
        return []
        #endif
    }
}
