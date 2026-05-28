import AppKit
import Darwin
import FileProvider
import Security
import SwiftUI

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"
private let irisDriveFileProviderDomainDisplayName = "My Drive"
private let irisDriveControlPanelWindowID = "control-panel"
private let irisDriveFileProviderRuntimeFileName = "fileprovider-runtime.json"
private let irisDriveFileProviderPathIdentifierPrefix = "path:"
private let irisDriveAppGroupName = "to.iris.drive"
private let irisDriveLegacyAppGroupIdentifier = "group.to.iris.drive"
private let irisDriveShowControlPanelNotification =
    Notification.Name("to.iris.drive.showControlPanel")
private let irisDriveShowDriveFolderNotification =
    Notification.Name("to.iris.drive.showDriveFolder")
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
    private var fileProviderRegistrationInFlight = false
    private var fileProviderDomainState = FileProviderDomainState.unknown
    private var windowObserver: NSObjectProtocol?
    private var openControlPanelWindow: (() -> Void)?
    private var peerStatusRefreshWorkItem: DispatchWorkItem?
    private var lastPeerStatusRefreshAt = Date.distantPast
    private var lastExternalFileProviderSignalKey: String?
    private var lastExternalFileProviderSignalAt = Date.distantPast
    private var fileProviderSignalWorkItem: DispatchWorkItem?
    private var fileProviderRepairInFlight = false
    private var fileProviderReimportInFlight = false
    private var lastFileProviderRepairAt = Date.distantPast
    private var lastFileProviderReimportAt = Date.distantPast
    private var lastFileProviderReimportKey: String?
    private var startupFileProviderDomainResetDone = false
    private var statusRefreshTimer: Timer?
    private var externalStatusFileSource: DispatchSourceFileSystemObject?
    private var externalStatusDirectorySource: DispatchSourceFileSystemObject?
    private var externalStatusFileDescriptor: CInt = -1
    private var externalStatusDirectoryDescriptor: CInt = -1
    private var externalStatusRefreshWorkItem: DispatchWorkItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        if handOffToExistingInstanceIfNeeded() {
            return
        }
        installSingleInstanceNotificationObserver()
        installStatusItem()
        installWindowObserver()
        observeWindows()
        updateStatus("Starting sync")
        DispatchQueue.global(qos: .utility).async { [weak self] in
            NSLog("Iris Drive launching daemon bootstrap")
            self?.bootstrapAndStartDaemon()
        }
        irisDriveDebugLog(
            "Iris Drive FileProvider integration enabled=\(fileProviderIntegrationEnabled) " +
            "testing=\(currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode")) " +
            "team=\(currentProcessEntitlementValue("com.apple.developer.team-identifier") ?? "nil")"
        )
        ensureFileProviderDomain()
    }

    func ensureFileProviderDomain() {
        if fileProviderIntegrationEnabled {
            guard !fileProviderRegistrationInFlight else {
                return
            }
            fileProviderRegistrationInFlight = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                guard let self else { return }
                let runtime = self.currentFileProviderRuntimeConfig()
                let resetDomain =
                    self.resetFileProviderDomainOnStart && !self.startupFileProviderDomainResetDone
                self.startupFileProviderDomainResetDone = true
                let completion: (FileProviderDomainState) -> Void = { state in
                    DispatchQueue.main.async {
                        self.fileProviderRegistrationInFlight = false
                        self.fileProviderDomainState = state
                        if state == .registered {
                            self.scheduleFileProviderReimport(
                                reason: "domain registered",
                                key: "domain-registered"
                            )
                        }
                    }
                }
                if resetDomain {
                    resetFileProviderDomain(
                        reason: "startup reset requested",
                        runtime: runtime,
                        completion
                    )
                } else {
                    ensureFileProviderDomainRegistered(runtime: runtime, completion)
                }
            }
        } else {
            irisDriveDebugLog("Iris Drive FileProvider disabled for this signing mode")
            fileProviderDomainState = .unavailable
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateStatus("Stopping sync")
        stopSync()
        statusRefreshTimer?.invalidate()
        statusRefreshTimer = nil
        stopExternalDaemonStatusWatcher()
        if let windowObserver {
            NotificationCenter.default.removeObserver(windowObserver)
        }
        DistributedNotificationCenter.default().removeObserver(
            self,
            name: irisDriveShowControlPanelNotification,
            object: nil
        )
        DistributedNotificationCenter.default().removeObserver(
            self,
            name: irisDriveShowDriveFolderNotification,
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

    @objc private func handleShowDriveFolderNotification(_ notification: Notification) {
        showDriveFolder()
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
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(handleShowDriveFolderNotification),
            name: irisDriveShowDriveFolderNotification,
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
        statusRefreshTimer?.invalidate()
        statusRefreshTimer = nil
        stopExternalDaemonStatusWatcher()
        if externalDaemonMode {
            setDaemonRunning(true)
            updateStatus("Sync managed externally")
            return
        }
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

    func checkBackups() {
        mutateBackupConfig(arguments: ["backups", "check"], success: "Backups checked")
    }

    func setLocalNhashResolver(_ enabled: Bool) {
        guard let paths = runtimePathsForMenu else {
            NSSound.beep()
            return
        }
        let idrive = idriveExecutableURL()
        let shouldRestart = IrisDriveStatus.shared.daemonRunning && !externalDaemonMode
        IrisDriveStatus.shared.localNhashResolverEnabled = enabled
        updateStatus(enabled ? "Local resolver enabled" : "Local resolver disabled")
        DispatchQueue.global(qos: .utility).async {
            do {
                _ = try self.runIDrive(
                    idrive,
                    arguments: ["nhash-resolver", enabled ? "enable" : "disable"],
                    paths: paths
                )
                DispatchQueue.main.async {
                    if shouldRestart {
                        self.restartSync()
                    } else {
                        self.refreshStatus()
                    }
                }
            } catch {
                NSLog("Iris Drive local resolver update failed: \(error)")
                DispatchQueue.main.async {
                    self.refreshStatus()
                    NSSound.beep()
                }
            }
        }
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
            title: "Open Drive Folder",
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
        NSLog("Iris Drive runtime paths config=\(paths.configDirectory.path)")
        runtimePathsForMenu = paths
        do {
            try ensureDirectory(paths.configDirectory)
            if !localProfileExists(paths: paths) {
                NSLog("Iris Drive local profile not found at \(paths.configDirectory.path)")
                updateStatus("Setup needed")
                return
            }

            prepareFileProviderRuntime(paths: paths, idrive: idrive)
            startDaemon(idrive, paths: paths)
        } catch {
            NSLog("Iris Drive daemon bootstrap failed: \(error)")
            updateStatus("Sync failed")
        }
    }

    private func localProfileExists(paths: IrisDriveRuntimePaths) -> Bool {
        FileManager.default.fileExists(
            atPath: paths.configDirectory.appendingPathComponent("key").path
        )
    }

    private func ensureDirectory(_ url: URL) throws {
        var isDirectory: ObjCBool = false
        if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) {
            if isDirectory.boolValue {
                return
            }
            throw NSError(
                domain: "IrisDriveMac",
                code: 20,
                userInfo: [NSLocalizedDescriptionKey: "\(url.path) is not a directory"]
            )
        }
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
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
                self.prepareFileProviderRuntime(paths: paths, idrive: idrive)
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
        guard fileProviderIntegrationEnabled else {
            handleFileProviderOpenFailure("disabled for this signing mode")
            return
        }
        let paths = runtimePathsForMenu ?? runtimePaths()
        prepareFileProviderRuntime(paths: paths, idrive: idriveExecutableURL())
        let runtime = FileProviderRuntimeConfig(
            configDirectory: paths.configDirectory.path,
            idriveExecutable: idriveExecutableURL()?.path
        )
        ensureFileProviderDomainRegistered(runtime: runtime) { [weak self] state in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fileProviderDomainState = state
                guard state == .registered else {
                    self.handleFileProviderOpenFailure("domain unavailable")
                    return
                }
                self.openFileProviderDriveFolder()
            }
        }
    }

    private func openFileProviderDriveFolder() {
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDomainDisplayName
        )
        guard let manager = NSFileProviderManager(for: domain) else {
            NSLog("Iris Drive FileProvider manager unavailable")
            handleFileProviderOpenFailure("manager unavailable")
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
                self.handleFileProviderOpenFailure("user-visible URL unavailable")
            }
        }
    }

    private func openMountedDriveFolder(_ url: URL) {
        if NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: url.path) {
            NSLog("Iris Drive mounted drive folder opened: \(url.path)")
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([url])
            NSLog("Iris Drive mounted drive folder revealed: \(url.path)")
        }
    }

    private func handleFileProviderOpenFailure(_ reason: String) {
        updateStatus("FileProvider unavailable")
        NSLog("Iris Drive FileProvider open failed: \(reason)")
        NSSound.beep()
    }

    private func startDaemon(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        guard daemon == nil else { return }
        daemonRestartWorkItem?.cancel()
        daemonRestartWorkItem = nil

        if externalDaemonMode {
            NSLog("Iris Drive external daemon mode enabled; app will not spawn bundled idrive")
            setDaemonRunning(true)
            updateStatus("Sync running")
            startStatusRefreshTimer(interval: 10.0)
            startExternalDaemonStatusWatcher(paths: paths)
            refreshStatus()
            return
        }

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
            NSLog("Iris Drive sync daemon started")
            setDaemonRunning(true)
            updateStatus("Sync running")
            prepareFileProviderRuntime(paths: paths, idrive: idrive)
            startStatusRefreshTimer(interval: 5.0)
            refreshStatus()
        } catch {
            NSLog("Iris Drive daemon failed to start: \(error)")
            updateStatus("Sync failed")
            setDaemonRunning(false)
            scheduleDaemonRestart(paths: paths)
        }
    }

    private func startStatusRefreshTimer(interval: TimeInterval) {
        DispatchQueue.main.async {
            self.statusRefreshTimer?.invalidate()
            let timer = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
                self?.refreshStatus()
            }
            timer.tolerance = min(interval / 2, 1.0)
            self.statusRefreshTimer = timer
            RunLoop.main.add(timer, forMode: .common)
        }
    }

    private func startExternalDaemonStatusWatcher(paths: IrisDriveRuntimePaths) {
        DispatchQueue.main.async {
            self.stopExternalDaemonStatusWatcher()
            self.startExternalDaemonStatusDirectoryWatcher(paths: paths)
            self.startExternalDaemonStatusFileWatcher(paths: paths)
        }
    }

    private func stopExternalDaemonStatusWatcher() {
        externalStatusRefreshWorkItem?.cancel()
        externalStatusRefreshWorkItem = nil
        externalStatusFileSource?.cancel()
        externalStatusFileSource = nil
        externalStatusDirectorySource?.cancel()
        externalStatusDirectorySource = nil
    }

    private func startExternalDaemonStatusDirectoryWatcher(paths: IrisDriveRuntimePaths) {
        let directory = paths.configDirectory
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            NSLog("Iris Drive daemon status directory watch unavailable: \(directory.path)")
            return
        }
        externalStatusDirectoryDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            self.startExternalDaemonStatusFileWatcher(paths: paths)
            self.scheduleExternalDaemonStatusRefresh(paths: paths)
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.externalStatusDirectoryDescriptor == descriptor {
                self.externalStatusDirectoryDescriptor = -1
            }
        }
        externalStatusDirectorySource = source
        source.resume()
    }

    private func startExternalDaemonStatusFileWatcher(paths: IrisDriveRuntimePaths) {
        externalStatusFileSource?.cancel()
        externalStatusFileSource = nil
        let statusURL = paths.configDirectory.appendingPathComponent("daemon-status.json")
        let descriptor = open(statusURL.path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        externalStatusFileDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            let data = source.data
            if data.contains(.delete) || data.contains(.rename) {
                self.startExternalDaemonStatusFileWatcher(paths: paths)
            }
            self.scheduleExternalDaemonStatusRefresh(paths: paths)
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.externalStatusFileDescriptor == descriptor {
                self.externalStatusFileDescriptor = -1
            }
        }
        externalStatusFileSource = source
        source.resume()
        NSLog("Iris Drive watching daemon status file: \(statusURL.path)")
    }

    private func scheduleExternalDaemonStatusRefresh(paths: IrisDriveRuntimePaths) {
        externalStatusRefreshWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.refreshExternalDaemonStatusFile(paths: paths)
        }
        externalStatusRefreshWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: workItem)
    }

    private func scheduleDaemonRestart(paths: IrisDriveRuntimePaths) {
        guard !userRequestedSyncStop, !externalDaemonMode else { return }
        daemonRestartWorkItem?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self,
                  self.daemon == nil,
                  !self.userRequestedSyncStop,
                  !self.externalDaemonMode
            else { return }
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

        var output = Data()
        var errorOutput = Data()
        let outputGroup = DispatchGroup()
        outputGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            output = stdout.fileHandleForReading.readDataToEndOfFile()
            outputGroup.leave()
        }
        outputGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            errorOutput = stderr.fileHandleForReading.readDataToEndOfFile()
            outputGroup.leave()
        }

        try process.run()
        let deadline = Date().addingTimeInterval(15)
        while process.isRunning && Date() < deadline {
            Thread.sleep(forTimeInterval: 0.05)
        }
        if process.isRunning {
            terminateDaemonProcess(process)
            outputGroup.wait()
            throw NSError(
                domain: "IrisDriveMac",
                code: 124,
                userInfo: [NSLocalizedDescriptionKey: "idrive command timed out"]
            )
        }
        process.waitUntilExit()
        outputGroup.wait()

        if process.terminationStatus != 0 {
            let errorText = String(
                data: errorOutput,
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
            process.arguments = ["--config-dir", paths.configDirectory.path] + arguments
        } else {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = ["idrive", "--config-dir", paths.configDirectory.path] + arguments
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
        if let override = ProcessInfo.processInfo.environment["IRIS_DRIVE_APP_BASE_DIR"],
           !override.isEmpty {
            return IrisDriveRuntimePaths(
                configDirectory: URL(fileURLWithPath: override, isDirectory: true)
                    .appendingPathComponent("Config", isDirectory: true)
            )
        }

        if let runtime = persistedFileProviderRuntime(),
           !runtime.configDirectory.isEmpty {
            return IrisDriveRuntimePaths(
                configDirectory: URL(fileURLWithPath: runtime.configDirectory, isDirectory: true)
            )
        }

        return IrisDriveRuntimePaths(
            configDirectory: fileProviderApplicationSupportFallbackDirectory()
                .appendingPathComponent("Config", isDirectory: true)
        )
    }

    private func currentFileProviderRuntimeConfig() -> FileProviderRuntimeConfig {
        let paths = runtimePathsForMenu ?? runtimePaths()
        return FileProviderRuntimeConfig(
            configDirectory: paths.configDirectory.path,
            idriveExecutable: idriveExecutableURL()?.path
        )
    }

    private var externalDaemonMode: Bool {
        environmentFlag("IRIS_DRIVE_EXTERNAL_DAEMON")
    }

    private var externalFileProviderRuntimeMode: Bool {
        environmentFlag("IRIS_DRIVE_FILEPROVIDER_RUNTIME_EXTERNAL")
    }

    private var resetFileProviderDomainOnStart: Bool {
        environmentFlag("IRIS_DRIVE_FILEPROVIDER_RESET_ON_START")
    }

    private var fileProviderReimportEnabled: Bool {
        environmentFlag("IRIS_DRIVE_ENABLE_FILEPROVIDER_REIMPORT")
    }

    private var fileProviderIntegrationEnabled: Bool {
        guard !environmentFlag("IRIS_DRIVE_DISABLE_FILEPROVIDER") else {
            return false
        }
        return currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode")
            || currentProcessHasTeamIdentifier()
    }

    private func environmentFlag(_ name: String) -> Bool {
        guard let value = ProcessInfo.processInfo.environment[name]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        else {
            return false
        }
        return ["1", "true", "yes", "on"].contains(value)
    }

    private func fileProviderApplicationSupportFallbackDirectory() -> URL {
        if let shared = irisDriveAppGroupContainerURL() {
            return shared.appendingPathComponent("Iris Drive", isDirectory: true)
        }
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return base.appendingPathComponent("Iris Drive", isDirectory: true)
    }

    private func persistedFileProviderRuntime() -> FileProviderRuntimeConfig? {
        let url = fileProviderApplicationSupportFallbackDirectory()
            .appendingPathComponent(irisDriveFileProviderRuntimeFileName)
        guard let data = try? Data(contentsOf: url) else {
            return nil
        }
        do {
            return try JSONDecoder().decode(FileProviderRuntimeConfig.self, from: data)
        } catch {
            NSLog("Iris Drive runtime config decode failed at \(url.path): \(error)")
            return nil
        }
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
                status.rootCID = primary["last_root_cid"] as? String
            }

            if let hashtree = json["hashtree"] as? [String: Any] {
                status.blocksDirectory = hashtree["blocks_dir"] as? String
                status.localBlockCount = Self.intValue(hashtree["local_block_count"]) ?? 0
                status.localBlockBytes = Self.int64Value(hashtree["local_block_bytes"]) ?? 0
                status.visibleFileBytes = Self.int64Value(hashtree["visible_file_bytes"])
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

            if let settings = json["settings"] as? [String: Any] {
                status.localNhashResolverEnabled =
                    settings["local_nhash_resolver_enabled"] as? Bool ?? true
            } else if let hashtree = json["hashtree"] as? [String: Any],
                      let gateway = hashtree["local_gateway"] as? [String: Any] {
                status.localNhashResolverEnabled = gateway["enabled"] as? Bool ?? true
            } else {
                status.localNhashResolverEnabled = true
            }

            if let peers = json["peers"] as? [[String: Any]] {
                status.peers = peers.map(IrisDrivePeerStatus.init)
            }

            self.updateLinkMenuState()
            self.signalFileProviderDomainForExternalStatusIfNeeded(
                key: Self.fileProviderSignalKey(json)
            )
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
                    self.signalFileProviderDomain()
                case "blossom_downloaded":
                    self.updateStatus("Fetched blocks")
                    self.signalFileProviderDomain()
                case "fips_downloaded",
                     "mounted_root",
                     "mount_refreshed",
                     "mount_refresh_skipped":
                    self.signalFileProviderDomain()
                case "shutdown":
                    self.updateStatus("Sync stopped")
                case "initial_publish_error", "auto_publish_error", "apply_error":
                    self.updateStatus("Sync needs attention")
                default:
                    break
                }
            }

            if json["fips_block_sync"] != nil {
                self.schedulePeerStatusRefresh()
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
            if let gateway = json["browser_gateway"] as? [String: Any],
               let enabled = gateway["enabled"] as? Bool {
                status.localNhashResolverEnabled = enabled
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
        if externalDaemonMode {
            refreshExternalDaemonStatus(paths: paths)
            return
        }
        let idrive = idriveExecutableURL()
        DispatchQueue.global(qos: .utility).async {
            do {
                self.applyStatusData(try self.runIDrive(idrive, arguments: ["status"], paths: paths))
            } catch {
                NSLog("Iris Drive status refresh failed: \(error)")
            }
        }
    }

    private func refreshExternalDaemonStatus(paths: IrisDriveRuntimePaths) {
        DispatchQueue.global(qos: .utility).async {
            let statusURL = paths.configDirectory.appendingPathComponent("daemon-status.json")
            do {
                self.applyStatusData(try self.runIDrive(self.idriveExecutableURL(), arguments: ["status"], paths: paths))
            } catch {
                do {
                    let data = try Data(contentsOf: statusURL)
                    guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                    else {
                        return
                    }
                    self.applyExternalDaemonStatusPayload(json)
                } catch {
                    NSLog("Iris Drive external daemon status refresh failed: \(error)")
                }
            }
        }
    }

    private func refreshExternalDaemonStatusFile(paths: IrisDriveRuntimePaths) {
        DispatchQueue.global(qos: .utility).async {
            let statusURL = paths.configDirectory.appendingPathComponent("daemon-status.json")
            do {
                let data = try Data(contentsOf: statusURL)
                guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                else {
                    return
                }
                self.applyExternalDaemonStatusPayload(json)
            } catch {
                NSLog("Iris Drive external daemon status file refresh failed: \(error)")
            }
        }
    }

    private func applyExternalDaemonStatusPayload(_ json: [String: Any]) {
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            status.initialized = true
            status.configDirectory = self.runtimePathsForMenu?.configDirectory.path
            if let event = json["event"] as? String {
                status.lastEvent = event
            }
            if let relays = json["relays"] as? [String] {
                status.relays = relays
            }
            if let relayStatuses = json["relay_statuses"] as? [[String: Any]] {
                status.relayStatuses = Self.mergeRelayStatuses(
                    relays: status.relays,
                    statuses: relayStatuses.map(IrisDriveRelayStatus.init)
                )
            }
            if let gateway = json["browser_gateway"] as? [String: Any],
               let enabled = gateway["enabled"] as? Bool {
                status.localNhashResolverEnabled = enabled
            }

            let running = json["running"] as? Bool ?? false
            let fresh = json["fresh"] as? Bool ?? false
            let fips = json["fips_block_sync"] as? [String: Any]
            let connectedPeers = fips?["connected_peers"] as? [String] ?? []
            let authorizedPeers = fips?["authorized_peers"] as? [String] ?? []
            let connectedSet = Set(connectedPeers)
            let authorizedConnected = authorizedPeers.filter { connectedSet.contains($0) }.count
            status.fips = IrisDriveFipsStatus(
                enabled: fips != nil,
                running: running,
                fresh: fresh,
                endpointNpub: fips?["endpoint_npub"] as? String,
                discoveryScope: fips?["discovery_scope"] as? String,
                rosterPeerCount: authorizedPeers.count,
                rosterConnectedPeerCount: authorizedConnected,
                connectedPeerCount: connectedPeers.count,
                otherPeerCount: max(0, connectedPeers.count - authorizedConnected),
                error: json["fips_block_sync_error"] as? String
            )
            self.updateLinkMenuState()
            self.signalFileProviderDomainForExternalStatusIfNeeded(
                key: Self.externalFileProviderSignalKey(json)
            )
            NSLog("Iris Drive control panel updated from external daemon status")
        }
    }

    private func prepareFileProviderRuntime(paths: IrisDriveRuntimePaths, idrive: URL?) {
        do {
            try ensureDirectory(paths.configDirectory)
            if externalFileProviderRuntimeMode {
                NSLog("Iris Drive FileProvider runtime managed externally")
                return
            }
            try writeFileProviderRuntime(paths: paths, idrive: idrive)
            NSLog("Iris Drive FileProvider runtime prepared")
        } catch {
            NSLog("Iris Drive FileProvider runtime preparation failed: \(error)")
        }
    }

    private func writeFileProviderRuntime(paths: IrisDriveRuntimePaths, idrive: URL?) throws {
        let runtime = FileProviderRuntimeConfig(
            configDirectory: paths.configDirectory.path,
            idriveExecutable: idrive?.path
        )
        let data = try JSONEncoder().encode(runtime)

        for directory in fileProviderRuntimeDirectories(paths: paths) {
            try ensureDirectory(directory)
            let url = directory.appendingPathComponent(irisDriveFileProviderRuntimeFileName)
            try data.write(to: url)
        }
    }

    private func fileProviderRuntimeDirectories(paths: IrisDriveRuntimePaths) -> [URL] {
        var directories = [URL]()
        directories.append(paths.configDirectory.deletingLastPathComponent())
        if ProcessInfo.processInfo.environment["IRIS_DRIVE_APP_BASE_DIR"] == nil {
            directories.append(fileProviderApplicationSupportFallbackDirectory())
        }

        var seen = Set<String>()
        return directories.filter { directory in
            seen.insert(directory.standardizedFileURL.path).inserted
        }
    }

    private func signalFileProviderDomain() {
        guard fileProviderIntegrationEnabled else { return }
        if !Thread.isMainThread {
            DispatchQueue.main.async { [weak self] in
                self?.signalFileProviderDomain()
            }
            return
        }
        guard fileProviderSignalWorkItem == nil else { return }
        let workItem = DispatchWorkItem { [weak self] in
            self?.fileProviderSignalWorkItem = nil
            self?.performFileProviderDomainSignal()
        }
        fileProviderSignalWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: workItem)
    }

    private func performFileProviderDomainSignal() {
        guard fileProviderIntegrationEnabled else { return }
        let domain = irisDriveFileProviderDomain()
        guard let manager = NSFileProviderManager(for: domain) else { return }
        signalFileProviderEnumerators(
            [
                (.workingSet, "working set"),
                (.rootContainer, "root"),
            ],
            manager: manager
        )
        signalFileProviderDirectoryEnumerators(manager: manager)
    }

    private func signalFileProviderEnumerators(
        _ identifiers: [(NSFileProviderItemIdentifier, String)],
        manager: NSFileProviderManager
    ) {
        for (identifier, label) in identifiers {
            manager.signalEnumerator(for: identifier) { error in
                if let error {
                    NSLog("Iris Drive FileProvider signal \(label) failed: \(error)")
                    DispatchQueue.main.async {
                        self.repairFileProviderDomainAfterSignalFailure(error)
                    }
                } else {
                    NSLog("Iris Drive FileProvider signal \(label) ok")
                }
            }
        }
    }

    private func signalFileProviderDirectoryEnumerators(manager: NSFileProviderManager) {
        guard let paths = runtimePathsForMenu else { return }
        let idrive = idriveExecutableURL()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let self else { return }
            do {
                let data = try self.runIDrive(
                    idrive,
                    arguments: ["provider", "list"],
                    paths: paths
                )
                let identifiers = try Self.fileProviderDirectorySignalIdentifiers(from: data)
                self.signalFileProviderEnumerators(identifiers, manager: manager)
            } catch {
                NSLog("Iris Drive FileProvider directory signal skipped: \(error)")
            }
        }
    }

    private static func fileProviderDirectorySignalIdentifiers(
        from data: Data
    ) throws -> [(NSFileProviderItemIdentifier, String)] {
        let list = try JSONDecoder().decode(FileProviderProviderList.self, from: data)
        var paths = Set<String>()
        for entry in list.entries {
            var parent = fileProviderParentPath(for: entry.path)
            while !parent.isEmpty {
                paths.insert(parent)
                parent = fileProviderParentPath(for: parent)
            }
            if entry.kind == "directory" {
                paths.insert(entry.path)
            }
        }
        return paths
            .sorted()
            .map { (fileProviderIdentifier(for: $0), "directory \($0)") }
    }

    private static func fileProviderIdentifier(for path: String) -> NSFileProviderItemIdentifier {
        if path.isEmpty {
            return .rootContainer
        }
        let encoded = Data(path.utf8).base64EncodedString()
        return NSFileProviderItemIdentifier(
            "\(irisDriveFileProviderPathIdentifierPrefix)\(encoded)"
        )
    }

    private static func fileProviderParentPath(for path: String) -> String {
        let parts = path.split(separator: "/").map(String.init)
        guard parts.count > 1 else { return "" }
        return parts.dropLast().joined(separator: "/")
    }

    private struct FileProviderProviderList: Decodable {
        let entries: [FileProviderProviderEntry]
    }

    private struct FileProviderProviderEntry: Decodable {
        let path: String
        let kind: String
    }

    private func repairFileProviderDomainAfterSignalFailure(_ error: Error) {
        guard shouldRepairFileProviderDomain(after: error) else { return }
        guard !fileProviderRepairInFlight else { return }

        let now = Date()
        guard now.timeIntervalSince(lastFileProviderRepairAt) >= 30 else {
            return
        }

        fileProviderRepairInFlight = true
        lastFileProviderRepairAt = now
        let runtime = currentFileProviderRuntimeConfig()
        resetFileProviderDomain(
            reason: "signal failure: \((error as NSError).domain) \((error as NSError).code)",
            runtime: runtime
        ) { [weak self] state in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fileProviderRepairInFlight = false
                self.fileProviderDomainState = state
                if state == .registered {
                    self.signalFileProviderDomain()
                }
            }
        }
    }

    private func scheduleFileProviderReimport(reason: String, key: String) {
        guard fileProviderIntegrationEnabled && fileProviderReimportEnabled else { return }
        guard !fileProviderReimportInFlight else { return }
        let now = Date()
        guard key != lastFileProviderReimportKey
            || now.timeIntervalSince(lastFileProviderReimportAt) >= 10
        else {
            return
        }

        fileProviderReimportInFlight = true
        lastFileProviderReimportAt = now
        lastFileProviderReimportKey = key
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            guard let self else { return }
            let domain = irisDriveFileProviderDomain(runtime: self.currentFileProviderRuntimeConfig())
            guard let manager = NSFileProviderManager(for: domain) else {
                self.fileProviderReimportInFlight = false
                NSLog("Iris Drive FileProvider reimport skipped; manager unavailable")
                return
            }
            manager.reimportItems(below: .rootContainer) { error in
                DispatchQueue.main.async {
                    self.fileProviderReimportInFlight = false
                    if let error {
                        NSLog("Iris Drive FileProvider reimport failed (\(reason)): \(error)")
                    } else {
                        NSLog("Iris Drive FileProvider reimport requested (\(reason))")
                    }
                }
            }
        }
    }

    private func signalFileProviderDomainForExternalStatusIfNeeded(key: String) {
        guard fileProviderIntegrationEnabled else { return }
        let now = Date()
        let changed = key != lastExternalFileProviderSignalKey
        guard changed || now.timeIntervalSince(lastExternalFileProviderSignalAt) >= 10 else {
            return
        }
        lastExternalFileProviderSignalKey = key
        lastExternalFileProviderSignalAt = now
        signalFileProviderDomain()
        if changed {
            scheduleFileProviderReimport(reason: "external status changed", key: key)
        }
    }

    private static func externalFileProviderSignalKey(_ json: [String: Any]) -> String {
        var parts = [String]()
        parts.append(json["root_cid"] as? String ?? "")
        parts.append(json["root_key"] as? String ?? "")
        if let lastBlockSync = json["last_block_sync"] as? [String: Any] {
            parts.append(lastBlockSync["root_cid"] as? String ?? "")
            parts.append(lastBlockSync["transport"] as? String ?? "")
            parts.append("\(Self.intValue(lastBlockSync["fetched"]) ?? 0)")
            parts.append("\(Self.intValue(lastBlockSync["already_local"]) ?? 0)")
            parts.append("\(Self.intValue(lastBlockSync["total_hashes"]) ?? 0)")
        }
        if let blockSyncByRoot = json["block_sync_by_root"] as? [String: Any] {
            for root in blockSyncByRoot.keys.sorted() {
                parts.append(root)
                guard let sync = blockSyncByRoot[root] as? [String: Any] else { continue }
                parts.append(sync["transport"] as? String ?? "")
                parts.append("\(Self.intValue(sync["fetched"]) ?? 0)")
                parts.append("\(Self.intValue(sync["already_local"]) ?? 0)")
                parts.append("\(Self.intValue(sync["total_hashes"]) ?? 0)")
            }
        }
        return parts.joined(separator: "|")
    }

    private static func fileProviderSignalKey(_ json: [String: Any]) -> String {
        var parts = [externalFileProviderSignalKey(json)]
        if let hashtree = json["hashtree"] as? [String: Any] {
            parts.append(hashtree["current_root_cid"] as? String ?? "")
            parts.append("\(Self.intValue(hashtree["file_count"]) ?? 0)")
            parts.append("\(Self.intValue(hashtree["top_level_entries"]) ?? 0)")
        }
        if let drives = json["drives"] as? [[String: Any]] {
            for drive in drives {
                parts.append([
                    drive["drive_id"] as? String ?? "",
                    drive["last_root_cid"] as? String ?? "",
                    "\(Self.intValue(drive["device_root_count"]) ?? 0)",
                ].joined(separator: ":"))
            }
        }
        if let peers = json["peers"] as? [[String: Any]] {
            for peer in peers {
                parts.append([
                    peer["device_npub"] as? String ?? peer["device_pubkey"] as? String ?? "",
                    peer["root_cid"] as? String ?? "",
                    peer["sync_state"] as? String ?? "",
                    "\(peer["root_available"] as? Bool ?? false)",
                    "\(peer["fips_online"] as? Bool ?? false)",
                ].joined(separator: ":"))
                if let lastBlockSync = peer["last_block_sync"] as? [String: Any] {
                    parts.append([
                        peer["device_npub"] as? String ?? peer["device_pubkey"] as? String ?? "",
                        "blocks",
                        lastBlockSync["root_cid"] as? String ?? "",
                        lastBlockSync["transport"] as? String ?? "",
                        "\(Self.intValue(lastBlockSync["fetched"]) ?? 0)",
                        "\(Self.intValue(lastBlockSync["already_local"]) ?? 0)",
                        "\(Self.intValue(lastBlockSync["total_hashes"]) ?? 0)",
                    ].joined(separator: ":"))
                }
            }
        }
        return parts.sorted().joined(separator: "|")
    }

    private func schedulePeerStatusRefresh() {
        guard peerStatusRefreshWorkItem == nil else { return }
        let elapsed = Date().timeIntervalSince(lastPeerStatusRefreshAt)
        let delay = max(0, 5 - elapsed)
        let workItem = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.peerStatusRefreshWorkItem = nil
            self.lastPeerStatusRefreshAt = Date()
            self.refreshStatus()
        }
        peerStatusRefreshWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: workItem)
    }

    private func setDaemonRunning(_ running: Bool) {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in
                self?.setDaemonRunning(running)
            }
            return
        }
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
}

private struct FileProviderRuntimeConfig: Codable {
    let configDirectory: String
    let idriveExecutable: String?

    var domainUserInfo: [String: String] {
        var userInfo = ["config_dir": configDirectory]
        if let idriveExecutable, !idriveExecutable.isEmpty {
            userInfo["idrive_executable"] = idriveExecutable
        }
        return userInfo
    }

    enum CodingKeys: String, CodingKey {
        case configDirectory = "config_dir"
        case idriveExecutable = "idrive_executable"
    }
}

private enum FileProviderDomainState {
    case unknown
    case registered
    case unavailable
}

private func irisDriveDebugLog(_ message: String) {
    NSLog("%@", message)
    if let data = (message + "\n").data(using: .utf8) {
        for directory in irisDriveDebugLogDirectories() {
            do {
                try FileManager.default.createDirectory(
                    at: directory,
                    withIntermediateDirectories: true
                )
                let url = directory.appendingPathComponent("macos-app-debug.log")
                if FileManager.default.fileExists(atPath: url.path),
                   let handle = try? FileHandle(forWritingTo: url) {
                    defer { try? handle.close() }
                    _ = try? handle.seekToEnd()
                    try? handle.write(contentsOf: data)
                } else {
                    try data.write(to: url)
                }
                break
            } catch {
                continue
            }
        }
    }
}

private func irisDriveDebugLogDirectories() -> [URL] {
    var directories = [URL]()
    if let shared = irisDriveAppGroupContainerURL() {
        directories.append(
            shared
                .appendingPathComponent("Iris Drive", isDirectory: true)
                .appendingPathComponent("Logs", isDirectory: true)
        )
    }
    if let support = FileManager.default.urls(
        for: .applicationSupportDirectory,
        in: .userDomainMask
    ).first {
        directories.append(
            support
                .appendingPathComponent("Iris Drive", isDirectory: true)
                .appendingPathComponent("Logs", isDirectory: true)
        )
    }
    directories.append(
        FileManager.default.temporaryDirectory
            .appendingPathComponent("Iris Drive", isDirectory: true)
            .appendingPathComponent("Logs", isDirectory: true)
    )
    return directories
}

private func irisDriveAppGroupContainerURL() -> URL? {
    for identifier in irisDriveAppGroupIdentifiers() {
        if let url = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: identifier
        ) {
            return url
        }
    }
    return nil
}

private func irisDriveAppGroupIdentifiers() -> [String] {
    var identifiers = [String]()
    if let teamIdentifier = currentProcessTeamIdentifier() {
        identifiers.append("\(teamIdentifier).\(irisDriveAppGroupName)")
    }
    identifiers.append(irisDriveLegacyAppGroupIdentifier)

    var seen = Set<String>()
    return identifiers.filter { seen.insert($0).inserted }
}

private func ensureFileProviderDomainRegistered(
    attempt: Int = 1,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    addFileProviderDomain(attempt: attempt, runtime: runtime, completion)
}

private func irisDriveFileProviderDomain(
    runtime: FileProviderRuntimeConfig? = nil
) -> NSFileProviderDomain {
    let domain = NSFileProviderDomain(
        identifier: irisDriveDomainIdentifier,
        displayName: irisDriveFileProviderDomainDisplayName
    )
    if let runtime, #available(macOS 15.0, *) {
        domain.userInfo = runtime.domainUserInfo
    }
    #if DEBUG
    if currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode") {
        domain.testingModes = [.alwaysEnabled]
    }
    #endif
    return domain
}

private func resetFileProviderDomain(
    reason: String,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    let domain = irisDriveFileProviderDomain(runtime: runtime)
    irisDriveDebugLog("Iris Drive FileProvider domain reset: \(reason)")
    NSFileProviderManager.remove(domain) { error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider domain remove during reset failed: \(error)")
        } else {
            irisDriveDebugLog("Iris Drive FileProvider domain removed during reset")
        }
        ensureFileProviderDomainRegistered(runtime: runtime, completion)
    }
}

private func addFileProviderDomain(
    attempt: Int,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    irisDriveDebugLog(
        "Iris Drive FileProvider registration attempt \(attempt) " +
        "config=\(runtime.configDirectory) idrive=\(runtime.idriveExecutable ?? "nil")"
    )
    let domain = irisDriveFileProviderDomain(runtime: runtime)

    NSFileProviderManager.add(domain) { error in
        if let error {
            fileProviderDomainExists { exists in
                if exists {
                    irisDriveDebugLog(
                        "Iris Drive FileProvider domain registered after add error: \(error)"
                    )
                    completion(.registered)
                    return
                }

                if attempt < 5 {
                    let delay = Double(attempt)
                    irisDriveDebugLog(
                        "Iris Drive FileProvider registration attempt \(attempt) failed; retrying in \(delay)s: \(error)"
                    )
                    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + delay) {
                        ensureFileProviderDomainRegistered(
                            attempt: attempt + 1,
                            runtime: runtime,
                            completion
                        )
                    }
                    return
                }

                irisDriveDebugLog("Iris Drive FileProvider registration failed: \(error)")
                completion(.unavailable)
            }
        } else {
            irisDriveDebugLog("Iris Drive FileProvider domain registered")
            completion(.registered)
        }
    }
}

private func fileProviderDomainExists(_ completion: @escaping (Bool) -> Void) {
    NSFileProviderManager.getDomainsWithCompletionHandler { domains, error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider domain query failed: \(error)")
        }
        completion(domains.contains { $0.identifier == irisDriveDomainIdentifier })
    }
}

private func shouldRepairFileProviderDomain(after error: Error) -> Bool {
    let nsError = error as NSError
    if nsError.domain == NSCocoaErrorDomain && nsError.code == NSFileReadNoPermissionError {
        return true
    }
    if nsError.domain == NSFileProviderErrorDomain && [-2001, -2014].contains(nsError.code) {
        return true
    }
    return false
}

private func currentProcessEntitlementValue(_ name: String) -> Any? {
    guard let task = SecTaskCreateFromSelf(nil),
          let value = SecTaskCopyValueForEntitlement(task, name as CFString, nil)
    else {
        return nil
    }
    return value
}

private func currentProcessHasEntitlement(_ name: String) -> Bool {
    guard let value = currentProcessEntitlementValue(name) else {
        return false
    }
    return (value as? Bool) == true
}

private func currentProcessHasTeamIdentifier() -> Bool {
    currentProcessTeamIdentifier() != nil
}

private func currentProcessTeamIdentifier() -> String? {
    guard let value = currentProcessEntitlementValue("com.apple.developer.team-identifier")
            as? String
    else {
        return nil
    }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}
