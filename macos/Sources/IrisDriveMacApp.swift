import AppKit
import FileProvider
import SwiftUI

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"
private let irisDriveControlPanelWindowID = "control-panel"
private let irisDriveAppGroupIdentifier = "group.to.iris.drive"

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
    }

    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        showControlPanel()
        return false
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

    @objc func setCloseToMenuBarOnClose(_ enabled: Bool) {
        UserDefaults.standard.set(enabled, forKey: IrisDriveStatus.closeToMenuBarOnCloseKey)
        IrisDriveStatus.shared.closeToMenuBarOnClose = enabled
        NSLog("Iris Drive menu bar on close set to \(enabled)")
    }

    @objc func showDriveFolder() {
        let paths = runtimePathsForMenu ?? runtimePaths()
        showMountedDriveFolder(fallbackURL: paths.workingDirectory)
    }

    @objc func copyDriveLink() {
        guard let link = IrisDriveStatus.shared.filesIrisURL, !link.isEmpty else {
            NSSound.beep()
            return
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(link, forType: .string)
        IrisDriveStatus.shared.copyStatus = "Copied"
        NSLog("Iris Drive private link copied")
    }

    @objc func openDriveLink() {
        guard let link = IrisDriveStatus.shared.filesIrisURL,
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
        startDaemon(idriveExecutableURL(), paths: paths)
    }

    @objc func stopSync() {
        guard let daemon else {
            setDaemonRunning(false)
            return
        }
        daemon.terminate()
        self.daemon = nil
        setDaemonRunning(false)
        updateStatus("Sync stopped")
    }

    @objc func restartSync() {
        stopSync()
        startSync()
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
            title: "Copy Private Link",
            action: #selector(copyDriveLink),
            keyEquivalent: ""
        )
        copyItem.target = self
        copyItem.isEnabled = false
        menu.addItem(copyItem)

        let openLinkItem = NSMenuItem(
            title: "Open Private Link",
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
        let image = NSImage(
            systemSymbolName: "externaldrive.fill",
            accessibilityDescription: irisDriveDisplayName
        ) ?? NSImage(size: NSSize(width: 18, height: 18))
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
            try FileManager.default.createDirectory(
                at: paths.workingDirectory,
                withIntermediateDirectories: true
            )

            let status = try runIDrive(idrive, arguments: ["status"], paths: paths)
            applyStatusData(status)
            let initialized = statusJSON(from: status)["initialized"] as? Bool ?? false
            if !initialized {
                _ = try runIDrive(idrive, arguments: ["init"], paths: paths)
            }

            let latestStatus = initialized
                ? status
                : try runIDrive(idrive, arguments: ["status"], paths: paths)
            applyStatusData(latestStatus)
            if primaryDriveRootCID(from: latestStatus) == nil {
                _ = try runIDrive(
                    idrive,
                    arguments: ["import", paths.workingDirectory.path],
                    paths: paths
                )
                applyStatusData(try runIDrive(idrive, arguments: ["status"], paths: paths))
            }

            startDaemon(idrive, paths: paths)
        } catch {
            NSLog("Iris Drive daemon bootstrap failed: \(error)")
            updateStatus("Sync failed")
        }
    }

    private func showMountedDriveFolder(fallbackURL: URL) {
        if fileProviderDomainState == .unavailable {
            NSLog("Iris Drive FileProvider domain unavailable; opening backing drive folder")
            openDriveFolder(fallbackURL, source: "backing")
            return
        }

        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveDisplayName
        )
        guard let manager = NSFileProviderManager(for: domain) else {
            NSLog("Iris Drive FileProvider manager unavailable; opening backing drive folder")
            openDriveFolder(fallbackURL, source: "backing")
            return
        }

        manager.getUserVisibleURL(for: .rootContainer) { [weak self] url, error in
            DispatchQueue.main.async {
                guard let self else { return }
                if let url {
                    self.openDriveFolder(url, source: "mounted")
                    return
                }

                if let error {
                    NSLog("Iris Drive mounted folder unavailable; opening backing drive folder: \(error)")
                } else {
                    NSLog("Iris Drive mounted folder unavailable; opening backing drive folder")
                }
                self.openDriveFolder(fallbackURL, source: "backing")
            }
        }
    }

    private func openDriveFolder(_ url: URL, source: String) {
        if source == "backing" {
            do {
                try FileManager.default.createDirectory(
                    at: url,
                    withIntermediateDirectories: true
                )
            } catch {
                NSLog("Iris Drive failed to create backing drive folder: \(error)")
                return
            }
        }

        let didStartSecurityScope = url.startAccessingSecurityScopedResource()
        defer {
            if didStartSecurityScope {
                url.stopAccessingSecurityScopedResource()
            }
        }

        if NSWorkspace.shared.open(url) {
            NSLog("Iris Drive drive folder opened (\(source)): \(url.path)")
        } else {
            NSLog("Iris Drive failed to open \(source) drive folder: \(url.path)")
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
            setDaemonRunning(true)
            updateStatus("Sync running")
            refreshStatus()
        } catch {
            NSLog("Iris Drive daemon failed to start: \(error)")
            updateStatus("Sync failed")
            setDaemonRunning(false)
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
                self.updateStatus("Sync stopped")
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
                status.authorizationState = account["authorization_state"] as? String
                status.rosterSize = Self.intValue(account["roster_size"]) ?? 0
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
                status.filesIrisURL = hashtree["files_iris_to_url"] as? String
                status.topLevelEntries = Self.intValue(hashtree["top_level_entries"])
            }

            if let network = json["network"] as? [String: Any] {
                status.relays = network["relays"] as? [String] ?? []
                status.blossomServers = network["blossom_servers"] as? [String] ?? []
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
                    status.workingDirectory =
                        json["working_dir"] as? String ?? status.workingDirectory
                case "initial_import":
                    self.updateStatus("Imported drive")
                case "initial_publish":
                    self.updateStatus("Sync running")
                case "auto_published":
                    self.updateStatus("Synced")
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
            if let link = json["files_iris_to_url"] as? String {
                status.filesIrisURL = link
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
        let hasLink = !(IrisDriveStatus.shared.filesIrisURL ?? "").isEmpty
        copyLinkMenuItem?.isEnabled = hasLink
        openLinkMenuItem?.isEnabled = hasLink
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
