import Foundation

extension AppDelegate {
    var daemonServiceSupervisionEnabled: Bool {
        !IrisDriveEnvironment.flag("IRIS_DRIVE_DISABLE_DAEMON_SERVICE")
    }

    func startDaemonService(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        DispatchQueue.global(qos: .utility).async {
            do {
                let data = try self.runIDrive(
                    idrive,
                    arguments: ["service", "install", "--launch", "--json"],
                    paths: paths
                )
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
            } catch {
                NSLog("Iris Drive daemon service failed to start: \(error)")
                DispatchQueue.main.async {
                    self.daemonServiceActive = false
                    self.setDaemonRunning(false)
                    self.updateStatus("Sync failed")
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

    private static func serviceRunning(from data: Data) -> Bool? {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        return json["running"] as? Bool
    }
}
