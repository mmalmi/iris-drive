import AppKit
import Darwin
import Foundation

extension AppDelegate {
    var launchedHidden: Bool {
        CommandLine.arguments.contains(irisDriveHiddenLaunchArgument)
    }

    func setLaunchOnStartup(_ enabled: Bool) {
        guard !launchAgentSyncDisabled else {
            updateStatus("Launch on startup disabled for development run")
            refreshStatus()
            NSSound.beep()
            return
        }
        do {
            try configureLaunchAgent(enabled: enabled, loadCurrentSession: true)
            launchOnStartupSynced = enabled
            IrisDriveStatus.shared.launchOnStartup = enabled
            dispatchNativeAction(
                ["type": "set_launch_on_startup", "enabled": enabled],
                progress: "Saving startup option",
                success: enabled ? "Launch on startup enabled" : "Launch on startup disabled"
            )
        } catch {
            updateStatus(error.localizedDescription)
            refreshStatus()
            NSSound.beep()
        }
    }

    func suppressHiddenLaunchWindowIfNeeded(remainingAttempts: Int = 4) {
        guard launchedHidden && !hiddenLaunchWindowSuppressed else {
            return
        }
        if let window = NSApp.windows.first(where: { $0.title == irisDriveDisplayName })
            ?? NSApp.windows.first {
            window.orderOut(nil)
            NSApp.hide(nil)
            hiddenLaunchWindowSuppressed = true
            return
        }
        guard remainingAttempts > 0 else {
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            self?.suppressHiddenLaunchWindowIfNeeded(remainingAttempts: remainingAttempts - 1)
        }
    }

    func syncLaunchAgentIfNeeded(enabled: Bool) {
        guard !launchAgentSyncDisabled else {
            if launchOnStartupSynced != enabled {
                NSLog("Iris Drive LaunchAgent sync skipped for development run")
                launchOnStartupSynced = enabled
            }
            return
        }
        guard launchOnStartupSynced != enabled else {
            return
        }
        do {
            try configureLaunchAgent(enabled: enabled, loadCurrentSession: false)
            launchOnStartupSynced = enabled
        } catch {
            updateStatus(error.localizedDescription)
        }
    }

    private func configureLaunchAgent(enabled: Bool, loadCurrentSession: Bool) throws {
        let manager = FileManager.default
        let agentsDir = manager.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        let plistURL = agentsDir.appendingPathComponent("to.iris.drive.macos.plist")
        if enabled {
            guard let executable = Bundle.main.executableURL?.path else {
                throw LaunchAgentError.missingExecutable
            }
            try manager.createDirectory(at: agentsDir, withIntermediateDirectories: true)
            try launchAgentPlist(executable: executable)
                .write(to: plistURL, atomically: true, encoding: .utf8)
            if loadCurrentSession {
                _ = runLaunchctl(["bootstrap", "gui/\(getuid())", plistURL.path])
            }
        } else {
            if loadCurrentSession {
                _ = runLaunchctl(["bootout", "gui/\(getuid())", plistURL.path])
            }
            if manager.fileExists(atPath: plistURL.path) {
                try manager.removeItem(at: plistURL)
            }
        }
    }

    private var launchAgentSyncDisabled: Bool {
        if IrisDriveEnvironment.flag("IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC") {
            return true
        }
        let bundlePath = Bundle.main.bundleURL.standardizedFileURL.path
        return bundlePath.contains("/macos/.build/")
            || bundlePath.contains("/DerivedData/")
    }
}

private enum LaunchAgentError: LocalizedError {
    case missingExecutable

    var errorDescription: String? {
        switch self {
        case .missingExecutable:
            return "App executable was not found."
        }
    }
}

private func launchAgentPlist(executable: String) -> String {
    """
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
        <key>Label</key>
        <string>to.iris.drive.macos</string>
        <key>ProgramArguments</key>
        <array>
            <string>\(xmlEscaped(executable))</string>
            <string>\(irisDriveHiddenLaunchArgument)</string>
        </array>
        <key>RunAtLoad</key>
        <true/>
    </dict>
    </plist>
    """
}

private func xmlEscaped(_ value: String) -> String {
    value
        .replacingOccurrences(of: "&", with: "&amp;")
        .replacingOccurrences(of: "<", with: "&lt;")
        .replacingOccurrences(of: ">", with: "&gt;")
        .replacingOccurrences(of: "\"", with: "&quot;")
}

private func runLaunchctl(_ arguments: [String]) -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
    process.arguments = arguments
    do {
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus == 0
    } catch {
        NSLog("Iris Drive launchctl failed: \(error)")
        return false
    }
}
