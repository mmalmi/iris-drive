import AppKit
import Foundation

private let irisDriveDefaultUpdatePollInterval: TimeInterval = 6 * 60 * 60

extension AppDelegate {
    func setAutoInstallUpdates(_ enabled: Bool) {
        let status = IrisDriveStatus.shared
        status.autoInstallUpdates = enabled
        UserDefaults.standard.set(enabled, forKey: IrisDriveStatus.autoInstallUpdatesKey)
        if enabled, status.updateAvailable, status.updateCanInstall {
            installUpdate()
        }
    }

    func checkForUpdates(manual: Bool = true) {
        if screenshotFixtureMode {
            if manual {
                IrisDriveStatus.shared.updateStatus = "Fixture mode"
            }
            return
        }
        let status = IrisDriveStatus.shared
        guard !status.updateChecking, !status.updateInstalling else {
            return
        }
        status.updateChecking = true
        if manual {
            status.updateStatus = "Checking for updates"
        }
        let dataDir = (runtimePathsForMenu ?? runtimePaths()).configDirectory.path
        let version = appVersion
        DispatchQueue.global(qos: .utility).async {
            let result = IrisDriveDesktopCore.updateCheck(
                dataDir: dataDir,
                currentVersion: version,
                mode: "app"
            )
            DispatchQueue.main.async {
                self.applyUpdateCheck(result, manual: manual)
            }
        }
    }

    func startAutomaticUpdateChecks() {
        let status = IrisDriveStatus.shared
        guard !screenshotFixtureMode, status.autoCheckUpdates else {
            stopAutomaticUpdateChecks()
            return
        }
        if !startupUpdateCheckDone {
            startupUpdateCheckDone = true
            checkForUpdates(manual: false)
        }
        guard updatePollTimer == nil else {
            return
        }
        updatePollTimer = Timer.scheduledTimer(
            withTimeInterval: updatePollInterval,
            repeats: true
        ) { [weak self] _ in
            guard let self else { return }
            guard IrisDriveStatus.shared.autoCheckUpdates else {
                self.stopAutomaticUpdateChecks()
                return
            }
            self.checkForUpdates(manual: false)
        }
    }

    func stopAutomaticUpdateChecks() {
        updatePollTimer?.invalidate()
        updatePollTimer = nil
    }

    func installUpdate() {
        let status = IrisDriveStatus.shared
        guard status.updateCanInstall, !status.updateInstalling else {
            if status.updateAsset.isEmpty {
                status.updateStatus = "No macOS update asset found"
            }
            return
        }
        status.updateInstalling = true
        status.updateStatus = "Downloading \(status.updateVersion)"
        let dataDir = (runtimePathsForMenu ?? runtimePaths()).configDirectory.path
        let version = appVersion
        let downloadDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("IrisDriveDownloads", isDirectory: true)
            .path
        DispatchQueue.global(qos: .utility).async {
            let result = IrisDriveDesktopCore.updateDownload(
                dataDir: dataDir,
                currentVersion: version,
                mode: "app",
                downloadDir: downloadDir
            )
            DispatchQueue.main.async {
                self.finishUpdateDownload(result)
            }
        }
    }

    private var updatePollInterval: TimeInterval {
        let raw = ProcessInfo.processInfo.environment["IRIS_DRIVE_UPDATE_POLL_SECONDS"] ?? ""
        if let seconds = TimeInterval(raw), seconds > 0 {
            return seconds
        }
        return irisDriveDefaultUpdatePollInterval
    }

    private func applyUpdateCheck(_ result: [String: Any], manual: Bool) {
        let status = IrisDriveStatus.shared
        status.updateChecking = false
        if let error = result["error"] as? String,
           !error.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            status.updateAvailable = false
            status.updateCanInstall = false
            status.updateAsset = ""
            status.updateStatus = manual ? error : ""
            return
        }
        let available = result["available"] as? Bool ?? false
        let asset = result["asset"] as? String ?? ""
        let tag = result["tag"] as? String ?? ""
        status.updateAvailable = available
        status.updateVersion = tag
        status.updateAsset = asset
        status.updateCanInstall = available && !asset.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if available {
            if status.updateCanInstall {
                status.updateStatus = "Update \(tag) available"
                if status.autoInstallUpdates && asset.lowercased().hasSuffix(".app.tar.gz") {
                    installUpdate()
                }
            } else {
                status.updateStatus = "Update \(tag) found without a macOS asset"
            }
        } else {
            status.updateStatus = manual ? "Up to date" : ""
        }
    }

    private func finishUpdateDownload(_ result: [String: Any]) {
        let status = IrisDriveStatus.shared
        if let error = result["error"] as? String,
           !error.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            status.updateInstalling = false
            status.updateStatus = error
            return
        }
        guard let path = result["path"] as? String, !path.isEmpty else {
            status.updateInstalling = false
            status.updateStatus = "Updater did not return a downloaded file"
            return
        }
        do {
            try installDownloadedUpdate(URL(fileURLWithPath: path))
        } catch {
            status.updateInstalling = false
            status.updateStatus = error.localizedDescription
        }
    }

    private func installDownloadedUpdate(_ archiveURL: URL) throws {
        let status = IrisDriveStatus.shared
        if archiveURL.lastPathComponent.lowercased().hasSuffix(".app.tar.gz") {
            status.updateStatus = "Installing \(status.updateVersion)"
            let unpackDir = FileManager.default.temporaryDirectory
                .appendingPathComponent("IrisDriveUpdate-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: unpackDir, withIntermediateDirectories: true)
            try runUpdateProcess(
                "/usr/bin/tar",
                arguments: ["-xzf", archiveURL.path, "-C", unpackDir.path]
            )
            guard let newApp = findIrisDriveApp(in: unpackDir) else {
                throw NSError(
                    domain: "IrisDriveUpdate",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Downloaded update did not contain Iris Drive.app"]
                )
            }
            let currentApp = Bundle.main.bundleURL
            let installScript = try updateInstallScript()
            let relaunchScript = try updateRelaunchScript()
            let paths = runtimePathsForMenu ?? runtimePaths()
            let logURL = FileManager.default.temporaryDirectory
                .appendingPathComponent("iris-drive-install-update.log")

            do {
                try runUpdateProcess(
                    "/bin/sh",
                    arguments: [installScript.path, currentApp.path, newApp.path, logURL.path]
                )
            } catch {
                try runPrivilegedUpdateInstall(
                    script: installScript,
                    currentApp: currentApp,
                    newApp: newApp,
                    logURL: logURL
                )
            }
            guard FileManager.default.isExecutableFile(
                atPath: currentApp
                    .appendingPathComponent("Contents/MacOS/Iris Drive", isDirectory: false)
                    .path
            ) else {
                throw updateError("Installed update is missing the Iris Drive executable")
            }

            let relaunch = Process()
            relaunch.executableURL = URL(fileURLWithPath: "/bin/sh")
            relaunch.arguments = [
                relaunchScript.path,
                currentApp.path,
                "\(ProcessInfo.processInfo.processIdentifier)",
                paths.configDirectory.path,
                logURL.path,
            ]
            try relaunch.run()
            installingAppUpdate = true
            NSApp.terminate(nil)
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([archiveURL])
            status.updateInstalling = false
            status.updateStatus = "Downloaded \(archiveURL.lastPathComponent)"
        }
    }

    private func findIrisDriveApp(in root: URL) -> URL? {
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else {
            return nil
        }
        for case let url as URL in enumerator {
            if url.pathExtension == "app",
               (url.lastPathComponent == "Iris Drive.app" || url.lastPathComponent == "IrisDriveMac.app") {
                return url
            }
        }
        return nil
    }

    private func updateInstallScript() throws -> URL {
        guard let bundled = Bundle.main.url(
            forResource: "iris-drive-install-update",
            withExtension: "sh"
        ) else {
            throw updateError("Iris Drive update installer is missing")
        }
        let script = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-install-update-\(UUID().uuidString).sh")
        try FileManager.default.copyItem(at: bundled, to: script)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        return script
    }

    private func updateRelaunchScript() throws -> URL {
        let script = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-relaunch-update-\(UUID().uuidString).sh")
        let contents = """
        #!/bin/sh
        set -eu
        current_app="$1"
        old_pid="$2"
        config_dir="${3:-}"
        log_path="${4:-${TMPDIR:-/tmp}/iris-drive-install-update.log}"

        exec >>"$log_path" 2>&1
        echo "iris-drive updater: waiting to relaunch after pid $old_pid"
        count=0
        while kill -0 "$old_pid" 2>/dev/null && [ "$count" -lt 240 ]; do
            sleep 0.25
            count=$((count + 1))
        done
        if kill -0 "$old_pid" 2>/dev/null; then
            echo "iris-drive updater: old app pid $old_pid did not exit"
            exit 1
        fi

        if [ -n "$config_dir" ] && [ -x "$current_app/Contents/MacOS/idrive" ]; then
            "$current_app/Contents/MacOS/idrive" --config-dir "$config_dir" service start --json >/dev/null 2>&1 || true
        fi

        echo "iris-drive updater: opening updated app"
        /usr/bin/open "$current_app"
        """
        try contents.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        return script
    }

    private func runPrivilegedUpdateInstall(
        script: URL,
        currentApp: URL,
        newApp: URL,
        logURL: URL
    ) throws {
        let appleScript = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-authorize-update-\(UUID().uuidString).applescript")
        let contents = """
        on run argv
            set helperPath to item 1 of argv
            set currentApp to item 2 of argv
            set newApp to item 3 of argv
            set logPath to item 4 of argv
            do shell script quoted form of helperPath & " " & quoted form of currentApp & " " & quoted form of newApp & " " & quoted form of logPath with administrator privileges
        end run
        """
        try contents.write(to: appleScript, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: appleScript) }

        let process = Process()
        let errorPipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = [
            appleScript.path,
            script.path,
            currentApp.path,
            newApp.path,
            logURL.path,
        ]
        process.standardError = errorPipe
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let details = String(
                data: errorPipe.fileHandleForReading.readDataToEndOfFile(),
                encoding: .utf8
            )?.trimmingCharacters(in: .whitespacesAndNewlines)
            let message = details?.isEmpty == false
                ? "Administrator install failed: \(details!)"
                : "Administrator install was cancelled"
            throw updateError(message)
        }
    }

    private func runUpdateProcess(_ executable: String, arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        try process.run()
        process.waitUntilExit()
        if process.terminationStatus != 0 {
            throw updateError("\(URL(fileURLWithPath: executable).lastPathComponent) failed")
        }
    }

    private func updateError(_ message: String) -> NSError {
        NSError(
            domain: "IrisDriveUpdate",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }
}
