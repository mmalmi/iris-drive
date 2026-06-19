import Foundation

extension AppDelegate {
    var daemonServiceSupervisionEnabled: Bool {
        !IrisDriveEnvironment.flag("IRIS_DRIVE_DISABLE_DAEMON_SERVICE")
    }

    func startAppManagedDaemon(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        guard daemon == nil else { return }
        guard !IrisDriveStatus.shared.revoked else {
            updateStatus("Device removed")
            setDaemonRunning(false)
            return
        }
        guard localProfileExists(paths: paths) else {
            updateStatus("Setup needed")
            setDaemonRunning(false)
            return
        }
        daemonRestartWorkItem?.cancel()
        daemonRestartWorkItem = nil

        let process = Process()
        configure(
            process,
            executable: idrive,
            arguments: ["daemon", "--watch-interval", "0"],
            paths: paths
        )
        pipeLogs(from: process, label: "idrive")

        do {
            try process.run()
            daemon = process
            irisDriveDebugLog("Iris Drive sync daemon started")
            setDaemonRunning(true)
            updateStatus("Sync on")
            if IrisDriveStatus.shared.setupComplete {
                prepareFileProviderRuntime(paths: paths, idrive: idrive)
            }
            startStatusRefreshTimer(interval: 5.0)
            refreshStatus()
        } catch {
            NSLog("Iris Drive daemon failed to start: \(error)")
            updateStatus("Sync failed")
            setDaemonRunning(false)
            scheduleDaemonRestart(paths: paths)
        }
    }

    func startDaemonService(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        DispatchQueue.global(qos: .utility).async {
            do {
                if currentProcessHasEntitlement("com.apple.security.app-sandbox") {
                    let data = try self.runIDrive(
                        idrive,
                        arguments: ["service", "status", "--json"],
                        paths: paths
                    )
                    if Self.serviceInstalled(from: data) {
                        var serviceData = data
                        if Self.serviceRunning(from: data) != true || Self.daemonStatusNeedsRestart(paths: paths) {
                            do {
                                serviceData = try self.runIDrive(
                                    idrive,
                                    arguments: ["service", "start", "--json"],
                                    paths: paths
                                )
                            } catch {
                                NSLog("Iris Drive daemon service refresh skipped: \(error)")
                            }
                        }
                        if Self.serviceRunning(from: serviceData) == true || Self.serviceRunning(from: data) == true {
                            self.attachDaemonService(data: serviceData, idrive: idrive, paths: paths)
                            return
                        }
                    }
                    irisDriveDebugLog(
                        "Iris Drive macOS app sandbox cannot install LaunchAgents directly; " +
                        "falling back to app-managed daemon"
                    )
                    DispatchQueue.main.async {
                        self.daemonServiceActive = false
                        self.updateStatus("Sync starting")
                        self.startAppManagedDaemon(idrive, paths: paths)
                    }
                    return
                }

                let data = try self.runIDrive(
                    idrive,
                    arguments: ["service", "install", "--launch", "--json"],
                    paths: paths
                )
                self.attachDaemonService(data: data, idrive: idrive, paths: paths)
            } catch {
                NSLog("Iris Drive daemon service failed to start: \(error)")
                irisDriveDebugLog("Iris Drive daemon service failed to start; falling back: \(error)")
                DispatchQueue.main.async {
                    self.daemonServiceActive = false
                    self.updateStatus("Sync starting")
                    self.startAppManagedDaemon(idrive, paths: paths)
                }
            }
        }
    }

    func stopDaemonService(paths: IrisDriveRuntimePaths) {
        let idrive = idriveExecutableURL()
        updateStatus("Sync stopping")
        DispatchQueue.global(qos: .utility).async {
            do {
                _ = try self.runIDrive(
                    idrive,
                    arguments: ["service", "stop", "--json"],
                    paths: paths
                )
            } catch {
                NSLog("Iris Drive daemon service stop failed: \(error)")
            }
            DispatchQueue.main.async {
                self.daemonServiceActive = false
                self.setDaemonRunning(false)
                self.updateStatus("Sync paused")
            }
        }
    }

    func restartDaemonService(
        _ idrive: URL?,
        paths: IrisDriveRuntimePaths,
        refreshDefinition: Bool = false
    ) {
        updateStatus(refreshDefinition ? "Updating service" : "Restarting service")
        DispatchQueue.global(qos: .utility).async {
            do {
                let arguments = refreshDefinition
                    ? ["service", "install", "--launch", "--json"]
                    : ["service", "start", "--json"]
                let data = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                self.attachDaemonService(data: data, idrive: idrive, paths: paths)
            } catch {
                NSLog("Iris Drive daemon service restart failed: \(error)")
                DispatchQueue.main.async {
                    self.updateStatus("Service restart failed")
                    self.refreshStatus()
                }
            }
        }
    }

    private static func serviceRunning(from data: Data) -> Bool? {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        return json["running"] as? Bool
    }

    private static func serviceInstalled(from data: Data) -> Bool {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        return json["installed"] as? Bool ?? false
    }

    private static func daemonStatusNeedsRestart(paths: IrisDriveRuntimePaths) -> Bool {
        let statusURL = paths.configDirectory.appendingPathComponent("daemon-status.json")
        guard let data = try? Data(contentsOf: statusURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              json["running"] as? Bool == true,
              json["fresh"] as? Bool != false
        else {
            return false
        }
        let version = json["binary_version"] as? String ?? ""
        let expected = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        return version.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || IrisDriveStatus.versionsDiffer(version, expected)
    }

    private func attachDaemonService(data: Data, idrive: URL?, paths: IrisDriveRuntimePaths) {
        let running = Self.serviceRunning(from: data) ?? true
        DispatchQueue.main.async {
            self.daemonServiceActive = true
            self.setDaemonRunning(running)
            self.updateStatus(running ? "Sync on" : "Sync starting")
            if IrisDriveStatus.shared.setupComplete {
                self.prepareFileProviderRuntime(paths: paths, idrive: idrive)
            }
            self.startStatusRefreshTimer(interval: 5.0)
            self.startExternalDaemonStatusWatcher(paths: paths)
            self.refreshStatus()
        }
    }
}
