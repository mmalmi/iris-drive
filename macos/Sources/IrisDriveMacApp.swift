import AppKit
import Darwin
import FileProvider
import Security
import SwiftUI

let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"
let irisDriveFileProviderDomainDisplayName = "My Drive"
private let irisDriveControlPanelWindowID = "control-panel"
private let irisDriveFileProviderRuntimeFileName = "fileprovider-runtime.json"
private let irisDriveFileProviderPathIdentifierPrefix = "path:"
let irisDriveFileProviderRegistrationIdentityKey = "fileProviderRegistrationIdentity"
private let irisDriveShowControlPanelNotification =
    Notification.Name("to.iris.drive.showControlPanel")
private let irisDriveShowDriveFolderNotification =
    Notification.Name("to.iris.drive.showDriveFolder")
private let irisDriveE2ECreateProfileNotification =
    Notification.Name("to.iris.drive.e2eCreateProfile")
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
    private let screenshotFixtureMode = IrisDriveScreenshotFixtures.enabled
    private var daemon: Process?
    private var userRequestedSyncStop = false
    private var daemonRestartWorkItem: DispatchWorkItem?
    private var statusItem: NSStatusItem?
    private var statusMenuItem: NSMenuItem?
    private var copyLinkMenuItem: NSMenuItem?
    private var openLinkMenuItem: NSMenuItem?
    private var startSyncMenuItem: NSMenuItem?
    private var stopSyncMenuItem: NSMenuItem?
    private(set) var runtimePathsForMenu: IrisDriveRuntimePaths?
    private var fileProviderRegistrationInFlight = false
    private var fileProviderDomainState = FileProviderDomainState.unknown
    private var windowObserver: NSObjectProtocol?
    private var openControlPanelWindow: (() -> Void)?
    private var controlPanelWindow: NSWindow?
    private var peerStatusRefreshWorkItem: DispatchWorkItem?
    private var lastPeerStatusRefreshAt = Date.distantPast
    private var lastExternalFileProviderSignalKey: String?
    private var lastExternalFileProviderSignalAt = Date.distantPast
    private var fileProviderSignalWorkItem: DispatchWorkItem?
    private var pendingFileProviderDirectoryPaths: [String]?
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
    private lazy var desktopCore = IrisDriveDesktopCore(
        dataDir: runtimePaths().configDirectory.path,
        appVersion: appVersion
    )

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? "0.1.0"
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if screenshotFixtureMode {
            IrisDriveScreenshotFixtures.apply()
            installWindowObserver()
            observeWindows()
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        if handOffToExistingInstanceIfNeeded() {
            return
        }
        installSingleInstanceNotificationObserver()
        installE2ENotificationObserverIfEnabled()
        installStatusItem()
        installWindowObserver()
        observeWindows()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            NSLog("Iris Drive launching daemon bootstrap")
            self?.bootstrapAndStartDaemon()
        }
        irisDriveDebugLog(
            "Iris Drive FileProvider integration enabled=\(fileProviderIntegrationEnabled) " +
            "testing=\(currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode")) " +
            "team=\(currentProcessTeamIdentifier() ?? "nil")"
        )
        ensureFileProviderDomainIfProfileExists()
    }

    func ensureFileProviderDomain() {
        if screenshotFixtureMode {
            fileProviderDomainState = .unavailable
            return
        }
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
                let finish: (FileProviderDomainState) -> Void = { state in
                    DispatchQueue.main.async {
                        self.fileProviderRegistrationInFlight = false
                        self.fileProviderDomainState = state
                        if state == .registered {
                            self.scheduleFileProviderReimport(
                                reason: "domain registered",
                                key: "domain-registered"
                            )
                        } else if state == .disabled {
                            self.updateStatus("FileProvider unavailable")
                        }
                    }
                }
                let completion: (FileProviderDomainState) -> Void = { state in
                    if state == .disabled {
                        irisDriveDebugLog("Iris Drive FileProvider domain disabled; resetting")
                        resetFileProviderDomain(
                            reason: "domain disabled",
                            runtime: runtime,
                            finish
                        )
                        return
                    }
                    finish(state)
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

    func ensureFileProviderDomainIfProfileExists() {
        if screenshotFixtureMode {
            fileProviderDomainState = .unavailable
            return
        }
        let paths = runtimePathsForMenu ?? runtimePaths()
        guard localProfileExists(paths: paths) else {
            fileProviderDomainState = .unavailable
            removeFileProviderDomainRegistration(
                reason: "local profile missing",
                runtime: currentFileProviderRuntimeConfig()
            )
            return
        }
        guard IrisDriveStatus.shared.setupComplete else {
            fileProviderDomainState = .unavailable
            return
        }
        ensureFileProviderDomain()
    }

    private func ensureFileProviderDomainAfterStatusIfNeeded() {
        guard fileProviderIntegrationEnabled else { return }
        guard IrisDriveStatus.shared.setupComplete else { return }
        guard fileProviderDomainState != .registered
            || !fileProviderRegistrationIdentityIsCurrent()
        else {
            return
        }
        ensureFileProviderDomain()
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateStatus("Pausing sync")
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
        DistributedNotificationCenter.default().removeObserver(
            self,
            name: irisDriveE2ECreateProfileNotification,
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
              let url = userActivity.webpageURL
        else {
            return false
        }
        return handleLaunchURL(url)
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            _ = handleLaunchURL(url)
        }
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
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            self?.observeWindows()
            if let window = self?.mainWindow() {
                window.makeKeyAndOrderFront(nil)
            } else {
                self?.fallbackControlPanelWindow().makeKeyAndOrderFront(nil)
            }
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

    @objc private func handleE2ECreateProfileNotification(_ notification: Notification) {
        guard e2eNotificationsEnabled else {
            return
        }
        let username = notification.userInfo?["username"] as? String ?? ""
        let profilePhotoPath = notification.userInfo?["profilePhotoPath"] as? String ?? ""
        NSLog("Iris Drive e2e create profile requested")
        createProfile(username: username, profilePhotoPath: profilePhotoPath)
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

    private func handleLaunchURL(_ url: URL) -> Bool {
        let classification = IrisDriveDesktopCore.classifyLinkInput(url.absoluteString)
        if classification["kind"] as? String == "share_dialog" {
            openShareDialog(
                sourcePath: classification["share_source_path"] as? String ?? "",
                displayName: classification["share_display_name"] as? String ?? "",
                recipientNpubHint: classification["share_recipient_npub_hint"] as? String ?? "",
                recipientDisplayName: classification["share_recipient_display_name"] as? String ?? "",
                recipientProfileId: classification["share_recipient_profile_id"] as? String ?? ""
            )
            return true
        }
        guard isIrisWebURL(url) else {
            return false
        }
        handleIrisWebURL(url)
        return true
    }

    private func openShareDialog(
        sourcePath: String,
        displayName: String,
        recipientNpubHint: String = "",
        recipientDisplayName: String = "",
        recipientProfileId: String = ""
    ) {
        let sourcePath = sourcePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sourcePath.isEmpty else {
            showControlPanel()
            updateStatus("Share folder path required")
            return
        }
        IrisDriveStatus.shared.pendingShareDialog = IrisDriveShareDialogRequest(
            sourcePath: sourcePath,
            displayName: displayName.trimmingCharacters(in: .whitespacesAndNewlines),
            recipientNpubHint: recipientNpubHint.trimmingCharacters(in: .whitespacesAndNewlines),
            recipientDisplayName: recipientDisplayName.trimmingCharacters(in: .whitespacesAndNewlines),
            recipientProfileId: recipientProfileId.trimmingCharacters(in: .whitespacesAndNewlines)
        )
        showControlPanel()
        updateStatus("Share folder selected")
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

    private func installE2ENotificationObserverIfEnabled() {
        guard e2eNotificationsEnabled else {
            return
        }
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(handleE2ECreateProfileNotification),
            name: irisDriveE2ECreateProfileNotification,
            object: nil
        )
        NSLog("Iris Drive e2e notifications enabled")
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
        copyText(link, statusMessage: "drive.iris.to link copied")
        IrisDriveStatus.shared.copyStatus = "Copied"
        NSLog("Iris Drive drive.iris.to link copied")
    }

    @objc func copyAppKey() {
        copyText(IrisDriveStatus.shared.currentAppKeyNpub, statusMessage: "AppKey copied")
    }

    @objc func copyDeviceKey() {
        copyText(IrisDriveStatus.shared.deviceNpub, statusMessage: "AppKey copied")
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
        guard !IrisDriveStatus.shared.revoked else {
            updateStatus("AppKey removed")
            setDaemonRunning(false)
            return
        }
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
        updateStatus("Sync paused")
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

    func resetInvite() {
        dispatchNativeAction(
            ["type": "reset_invite"],
            progress: "Resetting invite",
            success: "Invite reset"
        )
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

    func createProfile(username: String, profilePhotoPath: String) {
        var extra = ["--force"]
        let username = username.trimmingCharacters(in: .whitespacesAndNewlines)
        if !username.isEmpty {
            extra += ["--username", username]
            let profilePhotoPath = profilePhotoPath.trimmingCharacters(in: .whitespacesAndNewlines)
            if !profilePhotoPath.isEmpty {
                extra += ["--profile-photo", profilePhotoPath]
            }
        }
        let args = setupArguments(command: "init", label: "", extra: extra)
        finishSetup(arguments: args)
    }

    func restoreProfile(recoverySecret: String) {
        let recoverySecret = recoverySecret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !recoverySecret.isEmpty else {
            updateStatus("Recovery phrase or secret key required")
            return
        }
        let args = setupArguments(command: "restore", label: "", extra: [recoverySecret, "--force"])
        finishSetup(arguments: args)
    }

    func exportRecoverySecret() -> [String: Any] {
        guard let paths = runtimePathsForMenu else {
            return ["error": "runtime paths are not ready"]
        }
        return IrisDriveDesktopCore.exportRecoverySecret(
            dataDir: paths.configDirectory.path
        )
    }

    func linkDevice(target: String) {
        let target = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else {
            updateStatus("IrisProfile invite or admin AppKey required")
            return
        }
        let args = setupArguments(command: "link", label: "", extra: [target, "--force"])
        finishSetup(arguments: args)
    }

    func classifyLinkInput(_ input: String, completion: @escaping (String, Bool) -> Void) {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            DispatchQueue.main.async {
                completion(trimmed, false)
            }
            return
        }

        DispatchQueue.global(qos: .utility).async {
            let isComplete = IrisDriveDesktopCore.validateLinkInput(trimmed)
            DispatchQueue.main.async {
                completion(trimmed, isComplete)
            }
        }
    }

    @objc func logout() {
        let paths = runtimePathsForMenu ?? runtimePaths()
        runtimePathsForMenu = paths
        stopSync()
        dispatchNativeAction(
            ["type": "logout"],
            progress: "Logging out",
            success: "Logged out"
        )
    }

    func approveDevice(_ device: String, label: String) {
        let device = device.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !device.isEmpty else {
            updateStatus("AppKey required")
            return
        }
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        dispatchNativeAction(
            ["type": "approve_device", "request": device, "label": label],
            progress: "Approving AppKey",
            success: "AppKey approved",
            restartSyncAfterSuccess: true
        )
    }

    func rejectDevice(_ request: String) {
        let request = request.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !request.isEmpty else {
            updateStatus("AppKey request required")
            return
        }
        dispatchNativeAction(
            ["type": "reject_device", "request": request],
            progress: "Rejecting AppKey",
            success: "AppKey request rejected"
        )
    }

    func appointAdmin(_ device: String) {
        setDeviceAdminRole(device, makeAdmin: true)
    }

    func demoteAdmin(_ device: String) {
        setDeviceAdminRole(device, makeAdmin: false)
    }

    func deleteDevice(_ device: String) {
        let device = device.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !device.isEmpty else {
            updateStatus("AppKey required")
            return
        }

        dispatchNativeAction(
            ["type": "revoke_device", "app_key_pubkey": device],
            progress: "Removing AppKey",
            success: "AppKey removed",
            restartSyncAfterSuccess: true
        )
    }

    private func setDeviceAdminRole(_ device: String, makeAdmin: Bool) {
        let device = device.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !device.isEmpty else {
            updateStatus("AppKey required")
            return
        }

        dispatchNativeAction(
            [
                "type": makeAdmin ? "appoint_admin" : "demote_admin",
                "app_key_pubkey": device,
            ],
            progress: makeAdmin ? "Making admin" : "Removing admin",
            success: makeAdmin ? "AppKey made admin" : "Admin removed",
            restartSyncAfterSuccess: true
        )
    }

    func createShare(sourcePath: String, displayName: String) {
        let sourcePath = sourcePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sourcePath.isEmpty else {
            updateStatus("Folder path required")
            return
        }
        dispatchNativeAction(
            [
                "type": "create_share",
                "source_path": sourcePath,
                "display_name": displayName,
            ],
            progress: "Creating share",
            success: "Share created"
        )
    }

    func acceptShareInvite(_ invite: String) {
        let invite = invite.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !invite.isEmpty else {
            updateStatus("Share invite required")
            return
        }
        dispatchNativeAction(
            ["type": "accept_share_invite", "invite": invite],
            progress: "Accepting share",
            success: "Share accepted"
        )
    }

    func inviteShareMember(
        shareId: String,
        profileId: String,
        appKey: String,
        role: String,
        representativeNpubHint: String,
        displayName: String,
        label: String
    ) {
        let profileId = profileId.trimmingCharacters(in: .whitespacesAndNewlines)
        let appKey = appKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !profileId.isEmpty, !appKey.isEmpty else {
            updateStatus("Member profile UUID and AppActor required")
            return
        }
        dispatchNativeAction(
            [
                "type": "invite_share_member",
                "share_id": shareId,
                "profile_id": profileId,
                "app_key": appKey,
                "role": role,
                "representative_npub_hint": representativeNpubHint,
                "display_name": displayName,
                "label": label,
            ],
            progress: "Creating share invite",
            success: "Share invite created"
        ) {
            if let invite = IrisDriveStatus.shared.lastShareInviteURL {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(invite, forType: .string)
            }
        }
    }

    func inviteShareMemberFromEvidence(
        shareId: String,
        evidenceJson: String,
        role: String,
        displayName: String
    ) {
        let evidenceJson = evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !evidenceJson.isEmpty else {
            updateStatus("Recipient evidence required")
            return
        }
        dispatchNativeAction(
            [
                "type": "invite_share_member_from_evidence",
                "share_id": shareId,
                "evidence_json": evidenceJson,
                "role": role,
                "display_name": displayName,
            ],
            progress: "Creating share invite",
            success: "Share invite created"
        ) {
            if let invite = IrisDriveStatus.shared.lastShareInviteURL {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(invite, forType: .string)
            }
        }
    }

    func revokeShareMember(shareId: String, profileId: String) {
        dispatchNativeAction(
            [
                "type": "revoke_share_member",
                "share_id": shareId,
                "profile_id": profileId,
                "reason": "",
            ],
            progress: "Revoking share member",
            success: "Share member revoked"
        )
    }

    func addShareShortcut(shareId: String, displayName: String) {
        let path = displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "Shared folder"
            : displayName
        dispatchNativeAction(
            [
                "type": "add_share_shortcut",
                "share_id": shareId,
                "path": path,
                "parent": "",
                "target_path": "",
            ],
            progress: "Adding shortcut",
            success: "Shortcut added"
        )
    }

    func repairShareWraps(shareId: String) {
        dispatchNativeAction(
            ["type": "repair_share_wraps", "share_id": shareId],
            progress: "Repairing share keys",
            success: "Share keys repaired"
        )
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
            title: "Copy drive.iris.to Link",
            action: #selector(copyDriveLink),
            keyEquivalent: ""
        )
        copyItem.target = self
        copyItem.isEnabled = false
        menu.addItem(copyItem)

        let openLinkItem = NSMenuItem(
            title: "View on drive.iris.to",
            action: #selector(openDriveLink),
            keyEquivalent: ""
        )
        openLinkItem.target = self
        openLinkItem.isEnabled = false
        menu.addItem(openLinkItem)

        menu.addItem(.separator())
        let startItem = NSMenuItem(
            title: "Resume Sync",
            action: #selector(startSync),
            keyEquivalent: ""
        )
        startItem.target = self
        startItem.isEnabled = false
        menu.addItem(startItem)

        let stopItem = NSMenuItem(
            title: "Pause Sync",
            action: #selector(stopSync),
            keyEquivalent: ""
        )
        stopItem.target = self
        menu.addItem(stopItem)

        let logoutItem = NSMenuItem(
            title: "Log Out",
            action: #selector(logout),
            keyEquivalent: ""
        )
        logoutItem.target = self
        menu.addItem(logoutItem)

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
        irisDriveDebugLog("Iris Drive menu bar item installed")
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
                _ = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                self.refreshStatus()
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

    private func fallbackControlPanelWindow() -> NSWindow {
        if let controlPanelWindow {
            return controlPanelWindow
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = irisDriveDisplayName
        window.identifier = NSUserInterfaceItemIdentifier(irisDriveControlPanelWindowID)
        window.delegate = self
        window.contentView = NSHostingView(
            rootView: IrisDriveControlPanel(status: IrisDriveStatus.shared, controller: self)
                .frame(minWidth: 780, minHeight: 520)
        )
        window.center()
        controlPanelWindow = window
        return window
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
            NSLog("Iris Drive bundled idrive helper not found; using PATH lookup")
        }

        let paths = runtimePaths()
        NSLog("Iris Drive runtime paths config=\(paths.configDirectory.path)")
        runtimePathsForMenu = paths
        do {
            try ensureDirectory(paths.configDirectory)
            if !localProfileExists(paths: paths) {
                irisDriveDebugLog("Iris Drive local profile not found at \(paths.configDirectory.path)")
                updateStatus("Setup needed")
                return
            }

            updateStatus("Turning sync on")
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
                let nativeState = try self.nativeStatePayload(from: self.desktopCore.refreshJson())
                self.applyNativeStatePayload(nativeState)
                let setupComplete = Self.nativeStateSetupComplete(nativeState)
                NSLog(setupComplete ? "Iris Drive setup succeeded" : "Iris Drive setup awaiting approval")
                if setupComplete {
                    self.prepareFileProviderRuntime(paths: paths, idrive: idrive)
                }
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
        let paths = runtimePathsForMenu ?? runtimePaths()
        let status = IrisDriveStatus.shared
        guard localProfileExists(paths: paths), status.setupComplete else {
            updateStatus(status.revoked ? "AppKey removed" : (status.awaitingApproval ? "Waiting for approval" : "Setup needed"))
            return
        }
        guard fileProviderIntegrationEnabled else {
            handleFileProviderOpenFailure("disabled for this signing mode")
            return
        }
        prepareFileProviderRuntime(paths: paths, idrive: idriveExecutableURL())
        let runtime = FileProviderRuntimeConfig(
            configDirectory: paths.configDirectory.path,
            idriveExecutable: idriveExecutableURL()?.path
        )
        ensureFileProviderDomainRegistered(runtime: runtime) { [weak self] state in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fileProviderDomainState = state
                if state == .disabled {
                    self.resetDisabledFileProviderDomainForOpen(runtime: runtime)
                    return
                }
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
            irisDriveDebugLog("Iris Drive mounted drive folder opened: \(url.path)")
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([url])
            irisDriveDebugLog("Iris Drive mounted drive folder revealed: \(url.path)")
        }
    }

    private func handleFileProviderOpenFailure(_ reason: String) {
        updateStatus("FileProvider unavailable")
        irisDriveDebugLog("Iris Drive FileProvider open failed: \(reason)")
        NSSound.beep()
    }

    private func resetDisabledFileProviderDomainForOpen(runtime: FileProviderRuntimeConfig) {
        updateStatus("Repairing FileProvider")
        NSLog("Iris Drive FileProvider domain disabled; resetting before open")
        resetFileProviderDomain(reason: "open requested while disabled", runtime: runtime) { [weak self] state in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fileProviderDomainState = state
                guard state == .registered else {
                    self.handleFileProviderOpenFailure("domain disabled")
                    return
                }
                self.openFileProviderDriveFolder()
            }
        }
    }

    private func startDaemon(_ idrive: URL?, paths: IrisDriveRuntimePaths) {
        guard daemon == nil else { return }
        guard !IrisDriveStatus.shared.revoked else {
            updateStatus("AppKey removed")
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

        if externalDaemonMode {
            NSLog("Iris Drive external daemon mode enabled; app will not spawn bundled idrive")
            setDaemonRunning(true)
            updateStatus("Sync on")
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
        guard !IrisDriveStatus.shared.revoked else {
            updateStatus("AppKey removed")
            setDaemonRunning(false)
            return
        }
        guard localProfileExists(paths: paths) else {
            updateStatus("Setup needed")
            setDaemonRunning(false)
            return
        }
        daemonRestartWorkItem?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self,
                  self.daemon == nil,
                  !self.userRequestedSyncStop,
                  !self.externalDaemonMode
            else { return }
            self.updateStatus("Sync starting")
            self.startDaemon(self.idriveExecutableURL(), paths: paths)
        }
        daemonRestartWorkItem = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: item)
    }

    func runIDrive(
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
                    self.updateStatus("Sync paused")
                } else {
                    self.updateStatus("Sync starting")
                    self.scheduleDaemonRestart(paths: self.runtimePathsForMenu ?? self.runtimePaths())
                }
            }
        }
    }

    func idriveExecutableURL() -> URL? {
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
            configDirectory: fileProviderApplicationSupportDirectory()
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

    private var e2eNotificationsEnabled: Bool {
        environmentFlag("IRIS_DRIVE_ENABLE_E2E_NOTIFICATIONS")
    }

    private var fileProviderIntegrationEnabled: Bool {
        guard !environmentFlag("IRIS_DRIVE_DISABLE_FILEPROVIDER") else {
            return false
        }
        return currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode")
            || currentProcessHasTeamIdentifier()
    }

    private func environmentFlag(_ name: String) -> Bool {
        IrisDriveEnvironment.flag(name)
    }

    private func fileProviderApplicationSupportDirectory() -> URL {
        IrisDriveAppGroup.applicationSupportDirectory(
            teamIdentifier: currentProcessTeamIdentifier()
        )
    }

    private func persistedFileProviderRuntime() -> FileProviderRuntimeConfig? {
        let url = fileProviderApplicationSupportDirectory()
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

    private func nativeStatePayload(from json: String) throws -> [String: Any] {
        guard let data = json.data(using: .utf8),
              let state = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            throw NSError(
                domain: "IrisDriveMac",
                code: 30,
                userInfo: [NSLocalizedDescriptionKey: "native app-core returned invalid JSON"]
            )
        }
        if let error = state["error"] as? String, !error.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw NSError(
                domain: "IrisDriveMac",
                code: 31,
                userInfo: [NSLocalizedDescriptionKey: error]
            )
        }
        return state
    }

    private static func nativeStateSetupComplete(_ state: [String: Any]) -> Bool {
        guard let ui = state["ui"] as? [String: Any] else {
            return false
        }
        return ui["setup_complete"] as? Bool ?? false
    }

    private func applyNativeStateJson(_ json: String) throws {
        applyNativeStatePayload(try nativeStatePayload(from: json))
    }

    private func applyNativeStatePayload(_ state: [String: Any]) {
        let ui = state["ui"] as? [String: Any] ?? [:]
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            let account = ui["profile"] as? [String: Any]
            let paths = ui["paths"] as? [String: Any] ?? [:]
            status.initialized = account != nil
            status.configDirectory =
                paths["data_dir"] as? String
                ?? self.runtimePathsForMenu?.configDirectory.path
            status.blocksDirectory = paths["blocks_dir"] as? String

            if let account {
                status.profileId = account["profile_id"] as? String
                status.currentAppKeyNpub = account["current_app_key_npub"] as? String
                status.deviceNpub = account["current_app_key_npub"] as? String
                status.canAdminProfile =
                    account["can_admin_profile"] as? Bool ?? false
                status.canExportRecoveryPhrase =
                    account["can_export_recovery_phrase"] as? Bool ?? false
                let invite = account["app_key_link_invite"] as? String ?? ""
                status.appKeyLinkInviteURL = invite.isEmpty ? nil : invite
                status.inboundAppKeyLinkRequests =
                    (account["inbound_app_key_link_requests"] as? [[String: Any]] ?? [])
                    .map(IrisDriveAppKeyLinkRequestStatus.init(json:))
            } else {
                status.profileId = nil
                status.currentAppKeyNpub = nil
                status.deviceNpub = nil
                status.appKeyLinkInviteURL = nil
                status.inboundAppKeyLinkRequests = []
                status.canAdminProfile = false
                status.canExportRecoveryPhrase = false
            }

            let roots = ui["roots"] as? [[String: Any]] ?? []
            if let primary = roots.first {
                status.driveName = primary["name"] as? String ?? "My Drive"
                status.workingDirectory = primary["local_path"] as? String
            } else {
                status.driveName = "My Drive"
                status.workingDirectory = nil
            }

            status.snapshotURL = ui["snapshot_link"] as? String
            status.filesIrisURL = ui["snapshot_link"] as? String
            status.setupState = ui["setup_state"] as? String ?? "not_configured"
            status.setupComplete = ui["setup_complete"] as? Bool ?? false
            status.awaitingApproval = ui["awaiting_approval"] as? Bool ?? false
            status.revoked = ui["revoked"] as? Bool ?? false
            status.setupLabel = ui["setup_label"] as? String ?? "Not linked"
            status.primaryStatus = ui["primary_status"] as? String ?? "not_setup"
            status.primaryStatusLabel = ui["primary_status_label"] as? String ?? "Ready"
            if let sync = ui["sync"] as? [String: Any] {
                status.syncStatus = sync["status"] as? String ?? status.syncStatus
                status.syncStatusLabel = sync["status_label"] as? String ?? status.syncStatusLabel
            }
            status.authorizedDeviceCount = Self.intValue(ui["authorized_device_count"]) ?? 0
            status.onlineDeviceCount = Self.intValue(ui["online_device_count"]) ?? 0
            status.fileCount = Self.intValue(ui["file_count"]) ?? 0
            status.visibleFileBytes = Self.int64Value(ui["visible_file_bytes"]) ?? 0
            status.relays = ui["relays"] as? [String] ?? []
            status.relayStatuses =
                (ui["relay_statuses"] as? [[String: Any]] ?? []).map(IrisDriveRelayStatus.init)
            status.backupTargets =
                (ui["backups"] as? [[String: Any]] ?? []).map(IrisDriveBackupTarget.init)
            status.shares =
                (ui["shares"] as? [[String: Any]] ?? []).map(IrisDriveShareStatus.init)
            let lastShareInvite = ui["last_share_invite"] as? String ?? ""
            status.lastShareInviteURL = lastShareInvite.isEmpty ? nil : lastShareInvite
            status.fips = IrisDriveFipsStatus(json: ui["fips"] as? [String: Any] ?? [:])
            status.peers =
                (ui["devices"] as? [[String: Any]] ?? []).map(IrisDrivePeerStatus.init)
            status.lastUpload = nil

            if status.setupComplete, let paths = self.runtimePathsForMenu {
                self.prepareFileProviderRuntime(paths: paths, idrive: self.idriveExecutableURL())
                self.ensureFileProviderDomainAfterStatusIfNeeded()
            }
            if status.revoked {
                self.userRequestedSyncStop = true
                self.daemonRestartWorkItem?.cancel()
                self.daemonRestartWorkItem = nil
                if self.daemon != nil {
                    self.stopSync()
                } else {
                    self.setDaemonRunning(false)
                }
                self.updateStatus("AppKey removed")
            }

            self.updateLinkMenuState()
            self.signalFileProviderDomainForProviderChangeIfNeeded(reason: "native state changed")
            irisDriveDebugLog("Iris Drive control panel updated from app-core")
        }
    }

    func dispatchNativeAction(
        _ action: [String: Any],
        progress: String,
        success: String,
        restartSyncAfterSuccess: Bool = false,
        completion: (() -> Void)? = nil
    ) {
        let paths = runtimePathsForMenu ?? runtimePaths()
        runtimePathsForMenu = paths
        updateStatus(progress)
        DispatchQueue.global(qos: .utility).async {
            do {
                let data = try JSONSerialization.data(withJSONObject: action)
                guard let actionJson = String(data: data, encoding: .utf8) else {
                    throw NSError(
                        domain: "IrisDriveMac",
                        code: 32,
                        userInfo: [NSLocalizedDescriptionKey: "native action JSON encoding failed"]
                    )
                }
                try self.applyNativeStateJson(self.desktopCore.dispatchJson(actionJson))
                DispatchQueue.main.async {
                    self.updateStatus(success)
                    if restartSyncAfterSuccess {
                        if IrisDriveStatus.shared.daemonRunning {
                            self.restartSync()
                        } else {
                            self.startSync()
                        }
                    }
                    completion?()
                }
            } catch {
                NSLog("Iris Drive native action failed: \(error)")
                self.updateStatus("\(success) failed")
                DispatchQueue.main.async {
                    NSSound.beep()
                    completion?()
                }
            }
        }
    }

    private func applyStatusData(_ data: Data) {
        applyStatusPayload(statusJSON(from: data))
    }

    private func applyStatusPayload(_ json: [String: Any]) {
        DispatchQueue.main.async {
            let status = IrisDriveStatus.shared
            status.initialized = json["initialized"] as? Bool ?? false
            status.configDirectory = json["config_dir"] as? String

            if let account = json["profile"] as? [String: Any] {
                status.currentAppKeyNpub = account["current_app_key_npub"] as? String
                status.deviceNpub = account["current_app_key_npub"] as? String
                status.canAdminProfile =
                    account["can_admin_profile"] as? Bool ?? false
                status.canExportRecoveryPhrase =
                    account["can_export_recovery_phrase"] as? Bool
                    ?? (status.currentAppKeyNpub == status.deviceNpub && status.currentAppKeyNpub != nil)
                status.appKeyLinkInviteURL =
                    account["app_key_link_invite"] as? String
                    ?? (account["app_key_link_invite"] as? [String: Any])?["url"] as? String
                status.inboundAppKeyLinkRequests =
                    (account["inbound_app_key_link_requests"] as? [[String: Any]] ?? [])
                    .map(IrisDriveAppKeyLinkRequestStatus.init(json:))
            } else {
                status.currentAppKeyNpub = nil
                status.deviceNpub = nil
                status.appKeyLinkInviteURL = nil
                status.inboundAppKeyLinkRequests = []
                status.canAdminProfile = false
                status.canExportRecoveryPhrase = false
            }

            if let drives = json["drives"] as? [[String: Any]],
               let primary = drives.first(where: { $0["drive_id"] as? String == "main" }) {
                status.driveName = primary["display_name"] as? String ?? "My Drive"
                status.rootCID = primary["last_root_cid"] as? String
            }

            if let hashtree = json["hashtree"] as? [String: Any] {
                status.blocksDirectory = hashtree["blocks_dir"] as? String
                status.rootCID = hashtree["current_root_cid"] as? String ?? status.rootCID
                status.rootIsPrivate = hashtree["current_root_private"] as? Bool
                status.filesIrisURL =
                    (hashtree["drive_iris_to_url"] as? String)
                    ?? (hashtree["files_iris_to_url"] as? String)
                status.snapshotURL =
                    hashtree["snapshot_url"] as? String
                    ?? hashtree["permalink_url"] as? String
                    ?? status.snapshotURL
            }

            if let network = json["network"] as? [String: Any] {
                status.relays = network["relays"] as? [String] ?? []
                if let relayStatuses = network["relay_statuses"] as? [[String: Any]] {
                    status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
                }
                status.blossomServers = network["blossom_servers"] as? [String] ?? []
                status.backupTargets =
                    (network["backup_targets"] as? [[String: Any]])?.map(IrisDriveBackupTarget.init)
                    ?? []
                status.fips = IrisDriveFipsStatus(
                    json: network["fips"] as? [String: Any] ?? [:]
                )
            }

            if let summary = json["summary"] as? [String: Any] {
                Self.applyStatusSummary(summary, to: status)
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
            status.lastUpload = nil

            if status.setupComplete, let paths = self.runtimePathsForMenu {
                self.prepareFileProviderRuntime(paths: paths, idrive: self.idriveExecutableURL())
                self.ensureFileProviderDomainAfterStatusIfNeeded()
            }
            if status.revoked {
                self.userRequestedSyncStop = true
                self.daemonRestartWorkItem?.cancel()
                self.daemonRestartWorkItem = nil
                if self.daemon != nil {
                    self.stopSync()
                } else {
                    self.setDaemonRunning(false)
                }
                self.updateStatus("AppKey removed")
            }

            self.updateLinkMenuState()
            self.signalFileProviderDomainForProviderChangeIfNeeded(reason: "status changed")
            irisDriveDebugLog("Iris Drive control panel updated")
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
                    self.updateStatus("Sync on")
                    status.relays = json["relays"] as? [String] ?? status.relays
                    if let relayStatuses = json["relay_statuses"] as? [[String: Any]] {
                        status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
                    }
                case "relay_statuses":
                    if let relayStatuses = json["relay_statuses"] as? [[String: Any]] {
                        status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
                    }
                case "initial_import":
                    self.updateStatus("Imported drive")
                case "initial_publish":
                    self.updateStatus("Sync on")
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
                    self.updateStatus("Sync paused")
                case "initial_publish_error", "auto_publish_error", "apply_error":
                    self.updateStatus("Sync failed")
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
            if let upload = json["blossom_upload"] as? [String: Any] {
                let uploadStatus = IrisDriveUploadStatus(json: upload)
                status.lastUpload = uploadStatus.isInProgress ? uploadStatus : nil
            }
            if let gateway = json["browser_gateway"] as? [String: Any],
               let enabled = gateway["enabled"] as? Bool {
                status.localNhashResolverEnabled = enabled
            }
            if let sync = json["sync"] as? [String: Any] {
                Self.applyDaemonSyncStatus(sync, to: status)
            }

            self.updateLinkMenuState()
        }
        refreshStatus()
    }

    func updateStatus(_ message: String) {
        DispatchQueue.main.async {
            IrisDriveStatus.shared.message = message
            self.statusMenuItem?.title = message
        }
    }

    private static func applyStatusSummary(_ summary: [String: Any], to status: IrisDriveStatus) {
        if let setupState = summary["setup_state"] as? String {
            status.setupState = setupState
        }
        status.setupComplete = summary["setup_complete"] as? Bool ?? false
        status.awaitingApproval = summary["awaiting_approval"] as? Bool ?? false
        status.revoked = summary["revoked"] as? Bool ?? false
        if let setupLabel = summary["setup_label"] as? String {
            status.setupLabel = setupLabel
        }
        if let primaryStatus = summary["primary_status"] as? String {
            status.primaryStatus = primaryStatus
        }
        if let primaryStatusLabel = summary["primary_status_label"] as? String {
            status.primaryStatusLabel = primaryStatusLabel
        }
        if let syncStatus = summary["sync_status"] as? String {
            status.syncStatus = syncStatus
        }
        if let syncStatusLabel = summary["sync_status_label"] as? String {
            status.syncStatusLabel = syncStatusLabel
        }
        status.authorizedDeviceCount =
            Self.intValue(summary["authorized_device_count"]) ?? 0
        status.onlineDeviceCount =
            Self.intValue(summary["online_device_count"]) ?? 0
        status.fileCount = Self.intValue(summary["file_count"]) ?? 0
        status.visibleFileBytes =
            Self.int64Value(summary["visible_file_bytes"]) ?? 0
    }

    private static func applyDaemonSyncStatus(_ sync: [String: Any], to status: IrisDriveStatus) {
        if let syncStatus = sync["status"] as? String {
            status.syncStatus = syncStatus
        }
        if let syncStatusLabel = sync["status_label"] as? String {
            status.syncStatusLabel = syncStatusLabel
        }
    }

    func refreshStatus() {
        guard let paths = runtimePathsForMenu else { return }
        if externalDaemonMode {
            refreshExternalDaemonStatus(paths: paths)
            return
        }
        DispatchQueue.global(qos: .utility).async {
            do {
                try self.applyNativeStateJson(self.desktopCore.refreshJson())
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
                status.relayStatuses = relayStatuses.map(IrisDriveRelayStatus.init)
            }
            if let gateway = json["browser_gateway"] as? [String: Any],
               let enabled = gateway["enabled"] as? Bool {
                status.localNhashResolverEnabled = enabled
            }
            if let sync = json["sync"] as? [String: Any] {
                Self.applyDaemonSyncStatus(sync, to: status)
            }
            if let summary = json["summary"] as? [String: Any] {
                Self.applyStatusSummary(summary, to: status)
            }

            if status.setupComplete, let paths = self.runtimePathsForMenu {
                self.prepareFileProviderRuntime(paths: paths, idrive: self.idriveExecutableURL())
                self.ensureFileProviderDomainAfterStatusIfNeeded()
            }
            status.fips = IrisDriveFipsStatus(json: json["fips"] as? [String: Any] ?? [:])
            self.updateLinkMenuState()
            self.signalFileProviderDomainForProviderChangeIfNeeded(reason: "external status changed")
            irisDriveDebugLog("Iris Drive control panel updated from external daemon status")
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
            directories.append(fileProviderApplicationSupportDirectory())
        }

        var seen = Set<String>()
        return directories.filter { directory in
            seen.insert(directory.standardizedFileURL.path).inserted
        }
    }

    private func signalFileProviderDomain(directoryPaths: [String]? = nil) {
        guard fileProviderIntegrationEnabled else { return }
        if !Thread.isMainThread {
            DispatchQueue.main.async { [weak self] in
                self?.signalFileProviderDomain(directoryPaths: directoryPaths)
            }
            return
        }
        if let directoryPaths {
            pendingFileProviderDirectoryPaths = directoryPaths
        }
        guard fileProviderSignalWorkItem == nil else { return }
        let workItem = DispatchWorkItem { [weak self] in
            guard let self else { return }
            let directoryPaths = self.pendingFileProviderDirectoryPaths
            self.pendingFileProviderDirectoryPaths = nil
            self.fileProviderSignalWorkItem = nil
            self.performFileProviderDomainSignal(directoryPaths: directoryPaths)
        }
        fileProviderSignalWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: workItem)
    }

    private func performFileProviderDomainSignal(directoryPaths: [String]?) {
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
        if let directoryPaths {
            signalFileProviderEnumerators(
                Self.fileProviderDirectorySignalIdentifiers(for: directoryPaths),
                manager: manager
            )
        } else {
            signalFileProviderDirectoryEnumerators(manager: manager)
        }
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
                let summary = try self.providerSignalSummary(idrive: idrive, paths: paths)
                let identifiers = Self.fileProviderDirectorySignalIdentifiers(
                    for: summary.directoryPaths
                )
                self.signalFileProviderEnumerators(identifiers, manager: manager)
            } catch {
                NSLog("Iris Drive FileProvider directory signal skipped: \(error)")
            }
        }
    }

    private func providerSignalSummary(
        idrive: URL?,
        paths: IrisDriveRuntimePaths
    ) throws -> ProviderSignalSummary {
        let data = try runIDrive(
            idrive,
            arguments: ["provider", "list"],
            paths: paths
        )
        return try JSONDecoder().decode(ProviderSignalSummary.self, from: data)
    }

    private static func fileProviderDirectorySignalIdentifiers(
        for directoryPaths: [String]
    ) -> [(NSFileProviderItemIdentifier, String)] {
        directoryPaths
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

    private struct ProviderSignalSummary: Decodable {
        let directoryPaths: [String]
        let changeKey: String

        enum CodingKeys: String, CodingKey {
            case directoryPaths = "directory_paths"
            case changeKey = "change_key"
        }
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

    private func signalFileProviderDomainForProviderChangeIfNeeded(reason: String) {
        guard fileProviderIntegrationEnabled else { return }
        guard let paths = runtimePathsForMenu else { return }
        let idrive = idriveExecutableURL()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let self else { return }
            do {
                let summary = try self.providerSignalSummary(idrive: idrive, paths: paths)
                DispatchQueue.main.async {
                    self.signalFileProviderDomainIfNeeded(summary: summary, reason: reason)
                }
            } catch {
                NSLog("Iris Drive FileProvider provider summary skipped: \(error)")
            }
        }
    }

    private func signalFileProviderDomainIfNeeded(
        summary: ProviderSignalSummary,
        reason: String
    ) {
        let key = summary.changeKey
        guard !key.isEmpty else { return }
        let now = Date()
        let changed = key != lastExternalFileProviderSignalKey
        guard changed || now.timeIntervalSince(lastExternalFileProviderSignalAt) >= 10 else {
            return
        }
        lastExternalFileProviderSignalKey = key
        lastExternalFileProviderSignalAt = now
        signalFileProviderDomain(directoryPaths: summary.directoryPaths)
        if changed {
            scheduleFileProviderReimport(reason: reason, key: key)
        }
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
        guard runtimePathsForMenu != nil else {
            return nil
        }
        do {
            let state = try nativeStatePayload(from: desktopCore.refreshJson())
            applyNativeStatePayload(state)
            let ui = state["ui"] as? [String: Any] ?? [:]
            return ui["snapshot_link"] as? String
        } catch {
            NSLog("Iris Drive snapshot link refresh failed: \(error)")
            return nil
        }
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

struct IrisDriveRuntimePaths {
    let configDirectory: URL
}

func irisDriveDebugLog(_ message: String) {
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
    if let override = ProcessInfo.processInfo.environment["IRIS_DRIVE_DEBUG_LOG_DIR"],
       !override.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        directories.append(URL(fileURLWithPath: override, isDirectory: true))
    }
    if let shared = IrisDriveAppGroup.containerURL(
        teamIdentifier: currentProcessTeamIdentifier()
    ) {
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

func currentProcessEntitlementValue(_ name: String) -> Any? {
    IrisDriveCodeSigning.currentEntitlementValue(name)
}

func currentProcessHasEntitlement(_ name: String) -> Bool {
    guard let value = currentProcessEntitlementValue(name) else {
        return false
    }
    return (value as? Bool) == true
}

private func currentProcessHasTeamIdentifier() -> Bool {
    currentProcessTeamIdentifier() != nil
}

private func currentProcessTeamIdentifier() -> String? {
    IrisDriveCodeSigning.currentTeamIdentifier()
}
