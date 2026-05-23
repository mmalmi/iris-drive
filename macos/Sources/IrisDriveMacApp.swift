import AppKit
import FileProvider
import SwiftUI

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"
private let irisDriveControlPanelWindowID = "control-panel"
private let irisDriveAppGroupIdentifier = "group.to.iris.drive"
private let irisDriveShowControlPanelNotification =
    Notification.Name("to.iris.drive.showControlPanel")
private let irisDriveAssociatedHosts: Set<String> = [
    "drive.iris.to",
    "docs.iris.to",
    "video.iris.to",
    "maps.iris.to",
    "boards.iris.to",
    "git.iris.to",
]

@main
struct IrisDriveMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @Environment(\.openWindow) private var openWindow
    @ObservedObject private var status = IrisDriveStatus.shared

    var body: some Scene {
        WindowGroup(irisDriveDisplayName, id: irisDriveControlPanelWindowID) {
            IrisDriveControlPanel(status: status, controller: appDelegate)
                .frame(minWidth: 780, minHeight: 520)
                .onAppear {
                    appDelegate.configureOpenControlPanelWindow {
                        openWindow(id: irisDriveControlPanelWindowID)
                    }
                }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var daemon: Process?
    private var userRequestedSyncStop = false
    private var daemonRestartWorkItem: DispatchWorkItem?
    private var statusItem: NSStatusItem?
    private var statusMenuItem: NSMenuItem?
    private var copyLinkMenuItem: NSMenuItem?
    private var openLinkMenuItem: NSMenuItem?
    private var startSyncMenuItem: NSMenuItem?
    private var stopSyncMenuItem: NSMenuItem?
    private var runtimePathsForMenu: IrisDriveRuntimePaths?
    private var fileProviderDomainState = FileProviderDomainState.unknown
    private var windowObserver: NSObjectProtocol?
    private var openControlPanelWindow: (() -> Void)?

    func applicationDidFinishLaunching(_ notification: Notification) {
        if handOffToExistingInstanceIfNeeded() {
            return
        }
        installSingleInstanceNotificationObserver()
        installStatusItem()
        installWindowObserver()
        observeWindows()
        updateStatus("Starting sync")
        registerFileProviderDomain { [weak self] state in
            DispatchQueue.main.async {
                self?.fileProviderDomainState = state
            }
        }
        bootstrapAndStartDaemon()
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateStatus("Stopping sync")
        stopSync()
        if let windowObserver {
            NotificationCenter.default.removeObserver(windowObserver)
        }
        DistributedNotificationCenter.default().removeObserver(
            self,
            name: irisDriveShowControlPanelNotification,
            object: nil
        )
    }

    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        showControlPanel()
        return false
    }

    func application(
        _ application: NSApplication,
        continue userActivity: NSUserActivity,
        restorationHandler: @escaping ([NSUserActivityRestoring]) -> Void
    ) -> Bool {
        guard userActivity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = userActivity.webpageURL,
              isIrisWebURL(url)
        else {
            return false
        }
        handleIrisWebURL(url)
        return true
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender.title == irisDriveDisplayName,
              IrisDriveStatus.shared.closeToMenuBarOnClose
        else {
            return true
        }
        sender.orderOut(nil)
        NSLog("Iris Drive control panel hidden to menu bar")
        return false
    }

    @objc func showControlPanel() {
        NSApp.unhide(nil)
        NSApp.activate(ignoringOtherApps: true)
        observeWindows()
        if let window = mainWindow() {
            window.makeKeyAndOrderFront(nil)
            return
        }
        openControlPanelWindow?()
        DispatchQueue.main.async { [weak self] in
            self?.observeWindows()
            self?.mainWindow()?.makeKeyAndOrderFront(nil)
        }
    }

    func configureOpenControlPanelWindow(_ openWindow: @escaping () -> Void) {
        openControlPanelWindow = openWindow
    }

    @objc private func handleShowControlPanelNotification(_ notification: Notification) {
        showControlPanel()
    }

    private func isIrisWebURL(_ url: URL) -> Bool {
        guard url.scheme == "https",
              let host = url.host?.lowercased()
        else {
            return false
        }
        return irisDriveAssociatedHosts.contains(host)
    }

    private func handleIrisWebURL(_ url: URL) {
        showControlPanel()
        updateStatus("Iris link opened")
        NSLog("Iris Drive opened universal link: \(url.absoluteString)")
    }

    private func installSingleInstanceNotificationObserver() {
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(handleShowControlPanelNotification),
            name: irisDriveShowControlPanelNotification,
            object: nil
        )
    }

    private func handOffToExistingInstanceIfNeeded() -> Bool {
        guard let bundleIdentifier = Bundle.main.bundleIdentifier else {
            return false
        }
        let currentPID = ProcessInfo.processInfo.processIdentifier
        guard let existing = NSRunningApplication
            .runningApplications(withBundleIdentifier: bundleIdentifier)
            .first(where: { !$0.isTerminated && $0.processIdentifier != currentPID })
        else {
            return false
        }

        DistributedNotificationCenter.default().postNotificationName(
            irisDriveShowControlPanelNotification,
            object: nil,
            userInfo: nil,
            deliverImmediately: true
        )
        existing.activate(options: [.activateAllWindows])
        NSLog(
            "Iris Drive instance already running at pid \(existing.processIdentifier); exiting duplicate pid \(currentPID)"
        )
        NSApp.terminate(nil)
        return true
    }

    @objc func setCloseToMenuBarOnClose(_ enabled: Bool) {
        UserDefaults.standard.set(enabled, forKey: IrisDriveStatus.closeToMenuBarOnCloseKey)
        IrisDriveStatus.shared.closeToMenuBarOnClose = enabled
        NSLog("Iris Drive menu bar on close set to \(enabled)")
    }

    @objc func showDriveFolder() {
        showMountedDriveFolder()
    }

    @objc func copyDriveLink() {
        guard let link = currentSnapshotLink(), !link.isEmpty else {
            NSSound.beep()
            return
        }
        copyText(link, statusMessage: "Snapshot copied")
        IrisDriveStatus.shared.copyStatus = "Copied"
        NSLog("Iris Drive private link copied")
    }

    @objc func copyOwnerKey() {
        copyText(IrisDriveStatus.shared.ownerNpub, statusMessage: "Owner key copied")
    }

    @objc func copyDeviceKey() {
        copyText(IrisDriveStatus.shared.deviceNpub, statusMessage: "Device key copied")
    }

    private func copyText(_ value: String?, statusMessage: String) {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty
        else {
            NSSound.beep()
            return
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
        updateStatus(statusMessage)
    }

    @objc func openDriveLink() {
        guard let link = currentSnapshotLink(),
              let url = URL(string: link)
        else {
            NSSound.beep()
            return
        }
        NSWorkspace.shared.open(url)
    }

    @objc func openConfigFolder() {
        let paths = runtimePathsForMenu ?? runtimePaths()
        NSWorkspace.shared.open(paths.configDirectory)
    }

    @objc func startSync() {
        let paths = runtimePathsForMenu ?? runtimePaths()
        runtimePathsForMenu = paths
        userRequestedSyncStop = false
        startDaemon(idriveExecutableURL(), paths: paths)
    }

    @objc func stopSync() {
        userRequestedSyncStop = true
        daemonRestartWorkItem?.cancel()
        daemonRestartWorkItem = nil
        guard let daemon else {
            setDaemonRunning(false)
            return
        }
        terminateDaemonProcess(daemon)
        self.daemon = nil
        setDaemonRunning(false)
        updateStatus("Sync stopped")
    }

    private func terminateDaemonProcess(_ process: Process) {
        guard process.isRunning else { return }
        let pid = process.processIdentifier
        process.terminate()
        let deadline = Date().addingTimeInterval(2)
        while process.isRunning && Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
        if process.isRunning {
            let kill = Process()
            kill.executableURL = URL(fileURLWithPath: "/bin/kill")
            kill.arguments = ["-KILL", "\(pid)"]
            try? kill.run()
            kill.waitUntilExit()
        }
    }

    @objc func restartSync() {
        stopSync()
        startSync()
    }

    func addRelay(_ value: String) {
        mutateRelayConfig(arguments: ["relays", "add", value])
    }

    func updateRelay(_ oldValue: String, newValue: String) {
        mutateRelayConfig(arguments: ["relays", "update", oldValue, newValue])
    }

    func removeRelay(_ value: String) {
        mutateRelayConfig(arguments: ["relays", "remove", value])
    }

    func resetRelays() {
        mutateRelayConfig(arguments: ["relays", "reset"])
    }

    func addBackupTarget(_ value: String, label: String) {
        let target = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        var arguments = ["backups", "add", target]
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        if !label.isEmpty {
            arguments += ["--label", label]
        }
        mutateBackupConfig(arguments: arguments, success: "Backup added")
    }

    func syncBackups() {
        mutateBackupConfig(arguments: ["backups", "sync"], success: "Backups synced")
    }

    func createProfile(label: String) {
        let args = setupArguments(command: "init", label: label, extra: ["--force"])
        finishSetup(arguments: args)
    }

    func restoreProfile(secretKey: String, label: String) {
        let secret = secretKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !secret.isEmpty else {
            updateStatus("Secret key required")
            return
        }
        let args = setupArguments(command: "restore", label: label, extra: [secret])
        finishSetup(arguments: args)
    }

    func linkDevice(owner: String, label: String) {
        let owner = owner.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !owner.isEmpty else {
            updateStatus("Owner key required")
            return
        }
        let args = setupArguments(command: "link", label: label, extra: [owner])
        finishSetup(arguments: args)
    }

    func approveDevice(_ device: String, label: String) {
        let device = device.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !device.isEmpty else {
            updateStatus("Device key required")
            return
        }
        var arguments = ["approve", device]
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        if !label.isEmpty {
            arguments += ["--label", label]
        }

        let idrive = idriveExecutableURL()
        let paths = runtimePathsForMenu ?? runtimePaths()
        runtimePathsForMenu = paths
        updateStatus("Approving device")
        DispatchQueue.global(qos: .utility).async {
            do {
                _ = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                DispatchQueue.main.async {
                    self.updateStatus("Device approved")
                    self.refreshStatus()
                    if IrisDriveStatus.shared.daemonRunning {
                        self.restartSync()
                    } else {
                        self.startSync()
                    }
                }
            } catch {
                NSLog("Iris Drive device approval failed: \(error)")
                self.updateStatus("Approve failed")
                DispatchQueue.main.async {
                    NSSound.beep()
                }
            }
        }
    }

    @objc func quitApp() {
        NSApp.terminate(nil)
    }

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = statusIcon()
        item.button?.toolTip = irisDriveDisplayName

        let menu = NSMenu()
        let status = NSMenuItem(title: IrisDriveStatus.shared.message, action: nil, keyEquivalent: "")
        status.isEnabled = false
        menu.addItem(status)
        menu.addItem(.separator())
        let controlPanelItem = NSMenuItem(
            title: "Open Control Panel",
            action: #selector(showControlPanel),
            keyEquivalent: ""
        )
        controlPanelItem.target = self
        menu.addItem(controlPanelItem)

        let showDriveItem = NSMenuItem(
            title: "Show Drive Folder",
            action: #selector(showDriveFolder),
            keyEquivalent: ""
        )
        showDriveItem.target = self
        menu.addItem(showDriveItem)

        let copyItem = NSMenuItem(
            title: "Copy Snapshot Link",
            action: #selector(copyDriveLink),
            keyEquivalent: ""
        )
        copyItem.target = self
        copyItem.isEnabled = false
        menu.addItem(copyItem)

        let openLinkItem = NSMenuItem(
            title: "Open Snapshot Link",
            action: #selector(openDriveLink),
            keyEquivalent: ""
        )
        openLinkItem.target = self
        openLinkItem.isEnabled = false
        menu.addItem(openLinkItem)

        menu.addItem(.separator())
        let startItem = NSMenuItem(
            title: "Start Sync",
            action: #selector(startSync),
            keyEquivalent: ""
        )
        startItem.target = self
        startItem.isEnabled = false
        menu.addItem(startItem)

        let stopItem = NSMenuItem(
            title: "Stop Sync",
            action: #selector(stopSync),
            keyEquivalent: ""
        )
        stopItem.target = self
        menu.addItem(stopItem)

        let restartItem = NSMenuItem(
            title: "Restart Sync",
            action: #selector(restartSync),
            keyEquivalent: ""
        )
        restartItem.target = self
        menu.addItem(restartItem)

        let configItem = NSMenuItem(
            title: "Show Config Folder",
            action: #selector(openConfigFolder),
            keyEquivalent: ""
        )
        configItem.target = self
        menu.addItem(configItem)

        menu.addItem(.separator())
        let quitItem = NSMenuItem(
            title: "Quit",
            action: #selector(quitApp),
            keyEquivalent: "q"
        )
        quitItem.target = self
        menu.addItem(quitItem)

        item.menu = menu
        statusItem = item
        statusMenuItem = status
        copyLinkMenuItem = copyItem
        openLinkMenuItem = openLinkItem
        startSyncMenuItem = startItem
        stopSyncMenuItem = stopItem
        NSLog("Iris Drive menu bar item installed")
    }

    private func mutateRelayConfig(arguments: [String]) {
        guard let paths = runtimePathsForMenu else {
            NSSound.beep()
            return
        }
        let idrive = idriveExecutableURL()
        let shouldRestart = IrisDriveStatus.shared.daemonRunning
        DispatchQueue.global(qos: .utility).async {
            do {
                let data = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                self.applyRelaysData(data)
                if shouldRestart {
                    DispatchQueue.main.async {
                        self.restartSync()
                    }
                }
            } catch {
                NSLog("Iris Drive relay update failed: \(error)")
                DispatchQueue.main.async {
                    NSSound.beep()
                }
            }
        }
    }

    private func mutateBackupConfig(arguments: [String], success: String) {
        guard let paths = runtimePathsForMenu else {
            NSSound.beep()
            return
        }
        let idrive = idriveExecutableURL()
        updateStatus("Syncing backups")
        DispatchQueue.global(qos: .utility).async {
            do {
                _ = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                DispatchQueue.main.async {
                    self.updateStatus(success)
                    self.refreshStatus()
                }
            } catch {
                NSLog("Iris Drive backup update failed: \(error)")
                DispatchQueue.main.async {
                    self.updateStatus("Backup failed")
                    NSSound.beep()
                }
            }
        }
    }

    private func installWindowObserver() {
        guard windowObserver == nil else { return }
        windowObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeMainNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.observeWindows()
        }
    }

    private func observeWindows() {
        for window in NSApp.windows where window.title == irisDriveDisplayName {
            window.delegate = self
        }
    }

    private func mainWindow() -> NSWindow? {
        NSApp.windows.first(where: { $0.title == irisDriveDisplayName }) ?? NSApp.windows.first
    }

    private func statusIcon() -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { rect in
            NSColor.black.set()

            let ring = NSBezierPath(ovalIn: NSRect(x: 3.2, y: 3.2, width: 11.6, height: 11.6))
            ring.lineWidth = 2.4
            ring.stroke()

            NSBezierPath(ovalIn: NSRect(x: 8.0, y: 8.0, width: 2.0, height: 2.0)).fill()

            let reader = NSBezierPath()
            reader.move(to: NSPoint(x: 4.0, y: 4.0))
            reader.line(to: NSPoint(x: 7.9, y: 7.9))
            reader.lineWidth = 2.4
            reader.lineCapStyle = .round
            reader.stroke()

            return rect.width > 0
        }
        image.accessibilityDescription = irisDriveDisplayName
        image.isTemplate = true
        return image
    }

    private func bootstrapAndStartDaemon() {
        let idrive = idriveExecutableURL()
        if idrive == nil {
            NSLog("Iris Drive bundled idrive helper not found; falling back to PATH")
        }

        let paths = runtimePaths()
        runtimePathsForMenu = paths
        do {
            try FileManager.default.createDirectory(
                at: paths.configDirectory,
                withIntermediateDirectories: true
            )
            let status = try runIDrive(idrive, arguments: ["status"], paths: paths)
            applyStatusData(status)
            let initialized = statusJSON(from: status)["initialized"] as? Bool ?? false
            if !initialized {
                updateStatus("Setup needed")
                return
            }

            startDaemon(idrive, paths: paths)
        } catch {
            NSLog("Iris Drive daemon bootstrap failed: \(error)")
            updateStatus("Sync failed")
        }
    }

    private func setupArguments(command: String, label: String, extra: [String]) -> [String] {
        var arguments = [command] + extra
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        if !label.isEmpty {
            arguments += ["--label", label]
        }
        return arguments
    }

    private func finishSetup(arguments: [String]) {
        let idrive = idriveExecutableURL()
        let paths = runtimePaths()
        runtimePathsForMenu = paths
        updateStatus("Setting up")
        DispatchQueue.global(qos: .utility).async {
            do {
                try FileManager.default.createDirectory(
                    at: paths.configDirectory,
                    withIntermediateDirectories: true
                )
                _ = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                self.applyStatusData(try self.runIDrive(idrive, arguments: ["status"], paths: paths))
                DispatchQueue.main.async {
                    self.userRequestedSyncStop = false
                    self.startDaemon(idrive, paths: paths)
                }
            } catch {
                NSLog("Iris Drive setup failed: \(error)")
                self.updateStatus("Setup failed")
            }
        }
    }

    private func showMountedDriveFolder() {
        if fileProviderDomainState == .unavailable {
            NSLog("Iris Drive FileProvider domain unavailable")
            updateStatus("Drive mount unavailable")
            NSSound.beep()
            return
        }

        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveDisplayName
        )
        guard let manager = NSFileProviderManager(for: domain) else {
            NSLog("Iris Drive FileProvider manager unavailable")
            updateStatus("Drive mount unavailable")
            NSSound.beep()
            return
        }

        manager.getUserVisibleURL(for: .rootContainer) { [weak self] url, error in
            DispatchQueue.main.async {
                guard let self else { return }
                if let url {
                    self.openMountedDriveFolder(url)
                    return
                }

                if let error {
                    NSLog("Iris Drive mounted folder unavailable: \(error)")
                } else {
                    NSLog("Iris Drive mounted folder unavailable")
                }
                self.updateStatus("Drive mount unavailable")
                NSSound.beep()
            }
        }
    }

    private func openMountedDriveFolder(_ url: URL) {
        if NSWorkspace.shared.open(url) {
            NSLog("Iris Drive mounted drive folder opened: \(url.path)")
        } else {
            NSLog("Iris Drive failed to open mounted drive folder: \(url.path)")
            updateStatus("Drive mount unavailable")
            NSSound.beep()
        }
    }

    private func startDaemon(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        guard daemon == nil else { return }
        daemonRestartWorkItem?.cancel()
        daemonRestartWorkItem = nil

        let process = Process()
        configure(
            process,
            executable: idrive,
            arguments: ["daemon", "--no-working-dir", "--watch-interval", "0"],
            paths: paths
        )
        pipeLogs(from: process, label: "idrive")

        do {
            try process.run()
            daemon = process
            NSLog("Iris Drive sync daemon started")
            setDaemonRunning(true)
            updateStatus("Sync running")
            refreshStatus()
        } catch {
            NSLog("Iris Drive daemon failed to start: \(error)")
            updateStatus("Sync failed")
            setDaemonRunning(false)
            scheduleDaemonRestart(paths: paths)
        }
    }

    private func scheduleDaemonRestart(paths: IrisDriveRuntimePaths) {
        guard !userRequestedSyncStop else { return }
        daemonRestartWorkItem?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.daemon == nil, !self.userRequestedSyncStop else { return }
            self.updateStatus("Restarting sync")
            self.startDaemon(self.idriveExecutableURL(), paths: paths)
        }
        daemonRestartWorkItem = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: item)
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
        if arguments.first == "daemon" {
            environment["IRIS_DRIVE_PARENT_PID"] =
                "\(ProcessInfo.processInfo.processIdentifier)"
        }
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
                for line in text.split(whereSeparator: \.isNewline) {
                    self.handleDaemonLogLine(String(line))
                }
            }
        }

        process.terminationHandler = { [weak self, weak process] _ in
            DispatchQueue.main.async {
                guard let self, self.daemon === process else { return }
                self.daemon = nil
                self.setDaemonRunning(false)
                if self.userRequestedSyncStop {
                    self.updateStatus("Sync stopped")
                } else {
                    self.updateStatus("Restarting sync")
                    self.scheduleDaemonRestart(paths: self.runtimePathsForMenu ?? self.runtimePaths())
                }
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

    private func applyStatusData(_ data: Data) {
        applyStatusPayload(statusJSON(from: data))
    }

    private func applyStatusPayload(_ json: [String: Any]) {
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            status.initialized = json["initialized"] as? Bool ?? false
            status.configDirectory = json["config_dir"] as? String

            if let account = json["account"] as? [String: Any] {
                status.ownerNpub = account["owner_npub"] as? String
                status.deviceNpub = account["device_npub"] as? String
                status.hasOwnerSigningAuthority =
                    account["has_owner_signing_authority"] as? Bool ?? false
                status.authorizationState = account["authorization_state"] as? String
                status.rosterSize = Self.intValue(account["roster_size"]) ?? 0
            } else {
                status.ownerNpub = nil
                status.deviceNpub = nil
                status.hasOwnerSigningAuthority = false
                status.authorizationState = nil
                status.rosterSize = 0
            }

            if let drives = json["drives"] as? [[String: Any]],
               let primary = drives.first(where: { $0["drive_id"] as? String == "main" }) {
                status.driveName = primary["display_name"] as? String ?? "My Drive"
                status.workingDirectory = primary["working_dir"] as? String
                status.rootCID = primary["last_root_cid"] as? String
            }

            if let hashtree = json["hashtree"] as? [String: Any] {
                status.blocksDirectory = hashtree["blocks_dir"] as? String
                status.localBlockCount = Self.intValue(hashtree["local_block_count"]) ?? 0
                status.localBlockBytes = Self.int64Value(hashtree["local_block_bytes"]) ?? 0
                status.rootCID = hashtree["current_root_cid"] as? String ?? status.rootCID
                status.rootIsPrivate = hashtree["current_root_private"] as? Bool
                status.filesIrisURL =
                    (hashtree["drive_iris_to_url"] as? String)
                    ?? (hashtree["files_iris_to_url"] as? String)
                status.snapshotURL =
                    hashtree["snapshot_url"] as? String
                    ?? hashtree["permalink_url"] as? String
                    ?? status.snapshotURL
                status.fileCount = Self.intValue(hashtree["file_count"])
                status.topLevelEntries = Self.intValue(hashtree["top_level_entries"])
            }

            if let network = json["network"] as? [String: Any] {
                status.relays = network["relays"] as? [String] ?? []
                if let relayStatuses = network["relay_statuses"] as? [[String: Any]] {
                    status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
                } else {
                    status.relayStatuses = Self.mergeRelayStatuses(
                        relays: status.relays,
                        statuses: status.relayStatuses
                    )
                }
                status.blossomServers = network["blossom_servers"] as? [String] ?? []
                status.backupTargets =
                    (network["backup_targets"] as? [[String: Any]])?.map(IrisDriveBackupTarget.init)
                    ?? []
                status.fips = IrisDriveFipsStatus(
                    json: network["fips"] as? [String: Any] ?? [:]
                )
                status.authorizedDeviceCount =
                    Self.intValue(network["authorized_device_count"]) ?? 0
                status.publishedDeviceRoots =
                    Self.intValue(network["published_device_roots"]) ?? 0
            }

            if let peers = json["peers"] as? [[String: Any]] {
                status.peers = peers.map(IrisDrivePeerStatus.init)
            }

            self.updateLinkMenuState()
            NSLog("Iris Drive control panel updated")
        }
    }

    private func handleDaemonLogLine(_ line: String) {
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.first == "{",
              let data = trimmed.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return
        }
        handleDaemonPayload(json)
    }

    private func handleDaemonPayload(_ json: [String: Any]) {
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            if let event = json["event"] as? String {
                status.lastEvent = event
                switch event {
                case "subscribed":
                    self.updateStatus("Sync running")
                    status.relays = json["relays"] as? [String] ?? status.relays
                    if let relayStatuses = json["relay_statuses"] as? [[String: Any]] {
                        status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
                    } else {
                        status.relayStatuses = Self.mergeRelayStatuses(
                            relays: status.relays,
                            statuses: status.relayStatuses
                        )
                    }
                    status.workingDirectory =
                        json["working_dir"] as? String ?? status.workingDirectory
                case "relay_status":
                    if let url = json["url"] as? String,
                       let relayStatus = json["status"] as? String {
                        status.relayStatuses = Self.upsertRelayStatus(
                            IrisDriveRelayStatus(url: url, status: relayStatus),
                            into: status.relayStatuses,
                            relays: status.relays
                        )
                    }
                case "relay_statuses":
                    if let relayStatuses = json["relay_statuses"] as? [[String: Any]] {
                        status.relayStatuses = Self.mergeRelayStatuses(
                            relays: status.relays,
                            statuses: relayStatuses.map(IrisDriveRelayStatus.init)
                        )
                    }
                case "initial_import":
                    self.updateStatus("Imported drive")
                case "initial_publish":
                    self.updateStatus("Sync running")
                case "auto_published":
                    self.updateStatus("Published")
                case "app_keys":
                    self.updateStatus("Device roster updated")
                case "drive_root":
                    self.updateStatus("Peer root updated")
                case "blossom_downloaded":
                    self.updateStatus("Fetched blocks")
                case "shutdown":
                    self.updateStatus("Sync stopped")
                case "initial_publish_error", "auto_publish_error", "apply_error":
                    self.updateStatus("Sync needs attention")
                default:
                    break
                }
            }

            if let rootCID = json["root_cid"] as? String {
                status.rootCID = rootCID
            }
            if let link =
                (json["drive_iris_to_url"] as? String)
                ?? (json["files_iris_to_url"] as? String) {
                status.filesIrisURL = link
            }
            if let link = json["snapshot_url"] as? String ?? json["permalink_url"] as? String {
                status.snapshotURL = link
            }
            if let files = Self.intValue(json["file_count"]) {
                status.fileCount = files
            }
            if let entries = Self.intValue(json["top_level_entries"])
                ?? Self.intValue(json["entries"]) {
                status.topLevelEntries = entries
            }
            if let upload = json["blossom_upload"] as? [String: Any] {
                status.lastUpload = IrisDriveUploadStatus(json: upload)
            }

            self.updateLinkMenuState()
        }
        refreshStatus()
    }

    private func applyRelaysData(_ data: Data) {
        let relays = (try? JSONSerialization.jsonObject(with: data) as? [String]) ?? []
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            status.relays = relays
            status.relayStatuses = Self.mergeRelayStatuses(
                relays: relays,
                statuses: status.relayStatuses
            )
            NSLog("Iris Drive relays updated")
        }
    }

    private func updateStatus(_ message: String) {
        DispatchQueue.main.async {
            IrisDriveStatus.shared.message = message
            self.statusMenuItem?.title = message
        }
    }

    private func refreshStatus() {
        guard let paths = runtimePathsForMenu else { return }
        let idrive = idriveExecutableURL()
        DispatchQueue.global(qos: .utility).async {
            do {
                let data = try self.runIDrive(idrive, arguments: ["status"], paths: paths)
                self.applyStatusData(data)
            } catch {
                NSLog("Iris Drive status refresh failed: \(error)")
            }
        }
    }

    private func setDaemonRunning(_ running: Bool) {
        IrisDriveStatus.shared.daemonRunning = running
        startSyncMenuItem?.isEnabled = !running
        stopSyncMenuItem?.isEnabled = running
    }

    private func updateLinkMenuState() {
        let status = IrisDriveStatus.shared
        let hasLink = !(status.snapshotLinkURL ?? "").isEmpty || !(status.rootCID ?? "").isEmpty
        copyLinkMenuItem?.isEnabled = hasLink
        openLinkMenuItem?.isEnabled = hasLink
    }

    private func currentSnapshotLink() -> String? {
        if let link = IrisDriveStatus.shared.snapshotLinkURL, !link.isEmpty {
            return link
        }
        guard let paths = runtimePathsForMenu else {
            return nil
        }
        do {
            let data = try runIDrive(idriveExecutableURL(), arguments: ["status"], paths: paths)
            applyStatusData(data)
            return snapshotLink(from: data)
        } catch {
            NSLog("Iris Drive snapshot link refresh failed: \(error)")
            return nil
        }
    }

    private func snapshotLink(from data: Data) -> String? {
        let json = statusJSON(from: data)
        guard let hashtree = json["hashtree"] as? [String: Any] else {
            return nil
        }
        return hashtree["snapshot_url"] as? String
            ?? hashtree["permalink_url"] as? String
    }

    private static func mergeRelayStatuses(
        relays: [String],
        statuses: [IrisDriveRelayStatus]
    ) -> [IrisDriveRelayStatus] {
        let byURL = statuses.reduce(into: [String: String]()) { partial, relay in
            partial[normalizedRelayURL(relay.url)] = relay.status
        }
        return relays.map { relay in
            IrisDriveRelayStatus(
                url: relay,
                status: byURL[normalizedRelayURL(relay)] ?? "configured"
            )
        }
    }

    private static func upsertRelayStatus(
        _ relayStatus: IrisDriveRelayStatus,
        into statuses: [IrisDriveRelayStatus],
        relays: [String]
    ) -> [IrisDriveRelayStatus] {
        let normalized = normalizedRelayURL(relayStatus.url)
        var next = statuses.filter { normalizedRelayURL($0.url) != normalized }
        next.append(relayStatus)
        let knownRelays = relays.isEmpty ? next.map { normalizedRelayURL($0.url) } : relays
        return mergeRelayStatuses(relays: knownRelays, statuses: next)
    }

    private static func normalizedRelayURL(_ url: String) -> String {
        url.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }

    private static func intValue(_ value: Any?) -> Int? {
        if let value = value as? Int {
            return value
        }
        return (value as? NSNumber)?.intValue
    }

    private static func int64Value(_ value: Any?) -> Int64? {
        if let value = value as? Int64 {
            return value
        }
        return (value as? NSNumber)?.int64Value
    }
}

private struct IrisDriveRuntimePaths {
    let configDirectory: URL
    let workingDirectory: URL
}

private enum FileProviderDomainState {
    case unknown
    case registered
    case unavailable
}

private func registerFileProviderDomain(_ completion: @escaping (FileProviderDomainState) -> Void) {
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
            completion(.unavailable)
        } else {
            NSLog("Iris Drive FileProvider domain registered")
            completion(.registered)
        }
    }
}
