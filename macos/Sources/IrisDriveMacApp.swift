import FileProvider
import SwiftUI

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"
private let irisDriveAppGroupIdentifier = "group.to.iris.drive"

@main
struct IrisDriveMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @ObservedObject private var status = IrisDriveStatus.shared

    var body: some Scene {
        WindowGroup(irisDriveDisplayName) {
            VStack(spacing: 12) {
                Text(irisDriveDisplayName)
                    .font(.title2)
                Text(status.message)
                    .foregroundStyle(.secondary)
            }
            .frame(width: 420, height: 180)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var daemon: Process?

    func applicationDidFinishLaunching(_ notification: Notification) {
        updateStatus("Starting sync")
        registerFileProviderDomain()
        bootstrapAndStartDaemon()
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateStatus("Stopping sync")
        daemon?.terminate()
        daemon = nil
    }

    private func bootstrapAndStartDaemon() {
        let idrive = idriveExecutableURL()
        if idrive == nil {
            NSLog("Iris Drive bundled idrive helper not found; falling back to PATH")
        }

        let paths = runtimePaths()
        do {
            try FileManager.default.createDirectory(
                at: paths.configDirectory,
                withIntermediateDirectories: true
            )
            try FileManager.default.createDirectory(
                at: paths.workingDirectory,
                withIntermediateDirectories: true
            )

            let status = try runIDrive(idrive, arguments: ["status"], paths: paths)
            let initialized = statusJSON(from: status)["initialized"] as? Bool ?? false
            if !initialized {
                _ = try runIDrive(idrive, arguments: ["init"], paths: paths)
            }

            let latestStatus = initialized
                ? status
                : try runIDrive(idrive, arguments: ["status"], paths: paths)
            if primaryDriveRootCID(from: latestStatus) == nil {
                _ = try runIDrive(
                    idrive,
                    arguments: ["import", paths.workingDirectory.path],
                    paths: paths
                )
            }

            startDaemon(idrive, paths: paths)
        } catch {
            NSLog("Iris Drive daemon bootstrap failed: \(error)")
            updateStatus("Sync failed")
        }
    }

    private func startDaemon(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        guard daemon == nil else { return }

        let process = Process()
        configure(process, executable: idrive, arguments: ["daemon"], paths: paths)
        pipeLogs(from: process, label: "idrive")

        do {
            try process.run()
            daemon = process
            NSLog("Iris Drive sync daemon started")
            updateStatus("Sync running")
        } catch {
            NSLog("Iris Drive daemon failed to start: \(error)")
            updateStatus("Sync failed")
        }
    }

    private func runIDrive(
        _ idrive: URL?,
        arguments: [String],
        paths: IrisDriveRuntimePaths
    ) throws -> Data {
        let process = Process()
        configure(process, executable: idrive, arguments: arguments, paths: paths)

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        try process.run()
        process.waitUntilExit()

        let output = stdout.fileHandleForReading.readDataToEndOfFile()
        if process.terminationStatus != 0 {
            let errorText = String(
                data: stderr.fileHandleForReading.readDataToEndOfFile(),
                encoding: .utf8
            ) ?? ""
            throw NSError(
                domain: "IrisDriveMac",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: errorText]
            )
        }

        return output
    }

    private func configure(
        _ process: Process,
        executable: URL?,
        arguments: [String],
        paths: IrisDriveRuntimePaths
    ) {
        var environment = ProcessInfo.processInfo.environment
        environment["IRIS_DRIVE_CONFIG_DIR"] = paths.configDirectory.path
        process.environment = environment

        if let executable {
            process.executableURL = executable
            process.arguments = arguments
        } else {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = ["idrive"] + arguments
        }
    }

    private func pipeLogs(from process: Process, label: String) {
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        for pipe in [stdout, stderr] {
            pipe.fileHandleForReading.readabilityHandler = { handle in
                let data = handle.availableData
                guard !data.isEmpty, let text = String(data: data, encoding: .utf8) else {
                    return
                }
                NSLog("\(label): \(text.trimmingCharacters(in: .whitespacesAndNewlines))")
            }
        }
    }

    private func idriveExecutableURL() -> URL? {
        let bundled = Bundle.main.executableURL?
            .deletingLastPathComponent()
            .appendingPathComponent("idrive")
        if let bundled, FileManager.default.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        return nil
    }

    private func runtimePaths() -> IrisDriveRuntimePaths {
        let baseDirectory: URL
        if let override = ProcessInfo.processInfo.environment["IRIS_DRIVE_APP_BASE_DIR"],
           !override.isEmpty {
            baseDirectory = URL(fileURLWithPath: override, isDirectory: true)
        } else {
            baseDirectory = FileManager.default
                .containerURL(forSecurityApplicationGroupIdentifier: irisDriveAppGroupIdentifier)
                ?? fallbackApplicationSupportDirectory()
        }

        return IrisDriveRuntimePaths(
            configDirectory: baseDirectory.appendingPathComponent("Config", isDirectory: true),
            workingDirectory: baseDirectory.appendingPathComponent("Drive", isDirectory: true)
        )
    }

    private func fallbackApplicationSupportDirectory() -> URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return base.appendingPathComponent(irisDriveDisplayName, isDirectory: true)
    }

    private func statusJSON(from data: Data) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    private func primaryDriveRootCID(from data: Data) -> String? {
        let json = statusJSON(from: data)
        guard let drives = json["drives"] as? [[String: Any]] else {
            return nil
        }
        return drives
            .first(where: { $0["drive_id"] as? String == "main" })?["last_root_cid"] as? String
    }

    private func updateStatus(_ message: String) {
        DispatchQueue.main.async {
            IrisDriveStatus.shared.message = message
        }
    }
}

private struct IrisDriveRuntimePaths {
    let configDirectory: URL
    let workingDirectory: URL
}

private final class IrisDriveStatus: ObservableObject {
    static let shared = IrisDriveStatus()

    @Published var message = "Starting sync"
}

private func registerFileProviderDomain() {
    let domain = NSFileProviderDomain(
        identifier: irisDriveDomainIdentifier,
        displayName: irisDriveDisplayName
    )
    #if DEBUG
    domain.testingModes = [.alwaysEnabled]
    #endif

    NSFileProviderManager.add(domain) { error in
        if let error {
            NSLog("Iris Drive FileProvider registration failed: \(error)")
        } else {
            NSLog("Iris Drive FileProvider domain registered")
        }
    }
}
