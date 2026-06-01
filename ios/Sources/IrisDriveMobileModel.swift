import FileProvider
import Foundation
import SwiftUI
import UIKit

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveFileProviderDisplayName = "Iris Drive"
private let defaultRelay = "wss://relay.damus.io"
private let defaultRelays = [defaultRelay]
private let defaultRelayStatuses = defaultRelays.map {
    IrisDriveRelayStatus(url: $0, status: "configured", statusLabel: "saved", health: "configured")
}
private let defaultBlossomServers = ["https://upload.iris.to"]
private let iosDebugStateFileName = "debug-state.json"
private let fileProviderPathIdentifierPrefix = "path:"
private let foregroundSyncIntervalNanoseconds: UInt64 = 5_000_000_000
private let nativeBackgroundStackSize = 8 * 1024 * 1024
#if DEBUG
private let fileProviderDebugRegistrationVersion = 2
private let fileProviderDebugRegistrationVersionKey = "fileProviderDebugRegistrationVersion"
#endif

@MainActor
final class IrisDriveMobileModel: ObservableObject {
    @Published var driveName = "My Drive"
    @Published var statusTitle = "Ready"
    @Published var statusDetail = "Waiting for this device to be linked."
    @Published var deviceLabel = UIDevice.current.name
    @Published var ownerPublicKey = ""
    @Published var devicePublicKey = "local-device"
    @Published var restoreSecret = ""
    @Published var profileUsername = ""
    @Published var profilePhotoName = ""
    @Published var relay = defaultRelay
    @Published var relayInput = ""
    @Published var relays = defaultRelays
    @Published var relayStatuses = defaultRelayStatuses
    @Published var syncOverCellular = false
    @Published var syncRunning = false
    @Published var fileProviderStatus = "Files provider not registered"
    @Published var approveDeviceKey = ""
    @Published var approveDeviceLabel = ""
    @Published var devices: [IrisDriveDevice] = []
    @Published var inboundDeviceLinkRequests: [IrisDriveDeviceLinkRequest] = []
    @Published var backups: [IrisDriveBackup] = []
    @Published var roots: [IrisDriveRoot] = []
    @Published var fileProviderError = ""
    @Published var authorizationState = "Not linked"
    @Published var authorizedDeviceCount = 0
    @Published var onlineDeviceCount = 0
    @Published var fileCount = 0
    @Published var visibleFileBytes: UInt64 = 0

    private let defaults = UserDefaults.standard
    private let nativeCore: IrisDriveNativeCore
    private var lastState: NativeAppState?
    private var fileProviderOpenAttempt = 0
    private var currentProviderSignalKey = ""
    private var lastProviderSignalKey = ""
    private var currentProviderDirectoryPaths: [String] = []
    private var foregroundSyncTask: Task<Void, Never>?

    init() {
        nativeCore = IrisDriveNativeCore(dataDir: IrisDriveSharedContainer.baseDirectory.path, appVersion: "ios")
        load()
    }

    var sharedContainerPath: String {
        lastState?.ui.paths.dataDir ?? IrisDriveSharedContainer.baseDirectory.path
    }

    var configPath: String {
        lastState?.ui.paths.configPath
            ?? IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent("config.toml", isDirectory: false)
                .path
    }

    var blocksPath: String {
        lastState?.ui.paths.blocksDir
            ?? IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent("blocks", isDirectory: true)
                .path
    }

    var statusSymbol: String {
        if isSetupComplete {
            return "checkmark.circle.fill"
        }
        return isRevoked ? "exclamationmark.circle.fill" : "link.circle"
    }

    var statusTint: Color {
        if !fileProviderError.isEmpty {
            return .red
        }
        if isRevoked {
            return .red
        }
        return isSetupComplete ? .green : .orange
    }

    var syncStateTitle: String {
        syncRunning ? "Sync on" : "Sync paused"
    }

    var snapshotLink: String {
        lastState?.ui.snapshotLink ?? ""
    }

    var deviceLinkRequest: String {
        lastState?.ui.account?.deviceLinkRequest ?? ""
    }

    var deviceLinkInvite: String {
        lastState?.ui.account?.deviceLinkInvite ?? ""
    }

    var hasLocalProfile: Bool {
        !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var isSetupComplete: Bool {
        lastState?.ui.setupState == "authorized"
    }

    var isAwaitingApproval: Bool {
        lastState?.ui.setupState == "awaiting_approval"
    }

    var isRevoked: Bool {
        lastState?.ui.setupState == "revoked"
    }

    var hasOwnerAuthority: Bool {
        lastState?.ui.account?.hasOwnerSigningAuthority ?? false
    }

    func ensureFileProviderDomainIfProfileExists() {
        guard isSetupComplete else {
            fileProviderStatus = "Files provider not registered"
            rebuildDerivedState()
            return
        }
        ensureFileProviderDomain()
    }

    func ensureFileProviderDomain(completion: ((Bool) -> Void)? = nil) {
        guard isSetupComplete else {
            fileProviderStatus = "Files provider not registered"
            rebuildDerivedState()
            completion?(false)
            return
        }
        fileProviderStatus = "Registering Files provider"
        rebuildDerivedState()
        let domain = irisDriveFileProviderDomain()
        NSFileProviderManager.add(domain) { [weak self] error in
            if error == nil {
                Task { @MainActor in
                    guard let self else { return }
                    self.markFileProviderRegistrationCurrent()
                    self.fileProviderStatus = "Files provider registered"
                    self.rebuildDerivedState()
                    self.signalFileProviderIfNeeded()
                    completion?(true)
                }
                return
            }

            NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
                let existingDomain = domains.first { $0.identifier == irisDriveDomainIdentifier }
                let exists = existingDomain != nil
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    #if DEBUG
                    if let existingDomain,
                       self.shouldRepairFileProviderDebugRegistration(existingDomain) {
                        self.repairFileProviderDebugRegistration(
                            existingDomain: existingDomain,
                            completion: completion
                        )
                        return
                    }
                    if exists {
                        self.markFileProviderRegistrationCurrent()
                    }
                    #endif
                    self.fileProviderStatus = exists
                        ? "Files provider registered"
                        : "Files provider unavailable"
                    self.rebuildDerivedState()
                    if exists {
                        self.signalFileProviderIfNeeded()
                    }
                    completion?(exists)
                }
            }
        }
    }

    func openDriveFolder() {
        guard isSetupComplete else {
            showFileProviderError("Link this device before opening Iris Drive in Files.")
            return
        }
        fileProviderError = ""
        fileProviderStatus = "Opening Files provider"
        rebuildDerivedState()
        fileProviderOpenAttempt += 1
        let attempt = fileProviderOpenAttempt
        scheduleOpenInFilesTimeout(for: attempt)
        ensureFileProviderDomain { [weak self] ready in
            guard let self else { return }
            guard ready else {
                self.showFileProviderError("Files could not register Iris Drive.")
                return
            }
            self.openRegisteredDriveFolder(attempt: attempt)
        }
    }

    private func scheduleOpenInFilesTimeout(for attempt: Int) {
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            await MainActor.run { [weak self] in
                guard let self,
                      self.isSetupComplete,
                      self.fileProviderOpenAttempt == attempt,
                      self.fileProviderError.isEmpty,
                      UIApplication.shared.applicationState == .active
                else {
                    return
                }
                self.showFileProviderError("Files did not open Iris Drive.")
            }
        }
    }

    private func openRegisteredDriveFolder(attempt: Int) {
        let domain = irisDriveFileProviderDomain()
        guard let manager = NSFileProviderManager(for: domain) else {
            showFileProviderError("Files provider manager is unavailable.")
            return
        }
        manager.getUserVisibleURL(for: .rootContainer) { [weak self] url, error in
            Task { @MainActor in
                guard let self else { return }
                guard self.fileProviderOpenAttempt == attempt else { return }
                guard let url else {
                    if let error {
                        NSLog("Iris Drive Files provider URL unavailable: \(error)")
                    }
                    self.showFileProviderError("Files could not locate Iris Drive.")
                    return
                }
                guard let filesURL = self.filesAppURL(for: url) else {
                    NSLog("Iris Drive Files provider URL unsupported: \(url.absoluteString)")
                    self.showFileProviderError("Files could not locate Iris Drive.")
                    return
                }
                UIApplication.shared.open(filesURL, options: [:]) { [weak self] opened in
                    Task { @MainActor in
                        guard let self else { return }
                        guard self.fileProviderOpenAttempt == attempt else { return }
                        if opened {
                            self.fileProviderOpenAttempt += 1
                            self.fileProviderError = ""
                            self.fileProviderStatus = "Files provider open"
                            self.rebuildDerivedState()
                        } else {
                            NSLog(
                                "Iris Drive Files provider open failed: " +
                                    "\(filesURL.absoluteString) from \(url.absoluteString)"
                            )
                            self.showFileProviderError("Files refused to open Iris Drive.")
                        }
                    }
                }
            }
        }
    }

    private func filesAppURL(for userVisibleURL: URL) -> URL? {
        guard userVisibleURL.isFileURL else {
            return userVisibleURL
        }
        var components = URLComponents(url: userVisibleURL, resolvingAgainstBaseURL: false)
        components?.scheme = "shareddocuments"
        return components?.url
    }

    private func showFileProviderError(_ message: String) {
        fileProviderOpenAttempt += 1
        fileProviderError = message
        fileProviderStatus = message
        rebuildDerivedState()
    }

    private func removeFileProviderDomain() {
        let domain = irisDriveFileProviderDomain()
        NSFileProviderManager.remove(domain) { _ in }
        #if DEBUG
        defaults.removeObject(forKey: fileProviderDebugRegistrationVersionKey)
        #endif
    }

    private func irisDriveFileProviderDomain() -> NSFileProviderDomain {
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDisplayName
        )
        #if DEBUG
        domain.testingModes = [.alwaysEnabled]
        #endif
        return domain
    }

    private func markFileProviderRegistrationCurrent() {
        #if DEBUG
        defaults.set(fileProviderDebugRegistrationVersion, forKey: fileProviderDebugRegistrationVersionKey)
        #endif
    }

    #if DEBUG
    private func shouldRepairFileProviderDebugRegistration(_ domain: NSFileProviderDomain) -> Bool {
        if defaults.integer(forKey: fileProviderDebugRegistrationVersionKey)
            < fileProviderDebugRegistrationVersion {
            return true
        }
        return !domain.testingModes.contains(.alwaysEnabled)
    }

    private func repairFileProviderDebugRegistration(
        existingDomain: NSFileProviderDomain,
        completion: ((Bool) -> Void)?
    ) {
        let freshDomain = irisDriveFileProviderDomain()
        fileProviderStatus = "Repairing Files provider"
        rebuildDerivedState()
        NSLog("Iris Drive repairing stale FileProvider domain registration")
        NSFileProviderManager.remove(existingDomain) { [weak self] removeError in
            if let removeError {
                NSLog("Iris Drive FileProvider domain removal before repair failed: \(removeError)")
            }
            NSFileProviderManager.add(freshDomain) { [weak self] addError in
                Task { @MainActor in
                    guard let self else { return }
                    if let addError {
                        NSLog("Iris Drive FileProvider domain repair failed: \(addError)")
                        self.fileProviderStatus = "Files provider unavailable"
                        self.rebuildDerivedState()
                        completion?(false)
                        return
                    }
                    self.markFileProviderRegistrationCurrent()
                    self.fileProviderStatus = "Files provider registered"
                    self.rebuildDerivedState()
                    self.signalFileProviderIfNeeded()
                    completion?(true)
                }
            }
        }
    }
    #endif

    func refresh() {
        applyStateJson(nativeCore.refreshJson())
        ensureFileProviderDomainIfProfileExists()
    }

    func startForegroundSyncLoop() {
        guard foregroundSyncTask == nil else { return }
        foregroundSyncTask = Task { @MainActor [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                await self.syncOnceIfRunning()
                do {
                    try await Task.sleep(nanoseconds: foregroundSyncIntervalNanoseconds)
                } catch {
                    return
                }
            }
        }
    }

    func stopForegroundSyncLoop() {
        foregroundSyncTask?.cancel()
        foregroundSyncTask = nil
    }

    private func syncOnceIfRunning() async {
        guard isSetupComplete else {
            await refreshInBackground()
            return
        }
        if syncRunning {
            await dispatchInBackground(["type": "restart_sync"])
        } else {
            await refreshInBackground()
        }
    }

    func createProfile(username: String = "", profilePhotoName: String = "") {
        profileUsername = username.trimmingCharacters(in: .whitespacesAndNewlines)
        self.profilePhotoName = profileUsername.isEmpty ? "" : profilePhotoName
        dispatch([
            "type": "create_profile",
            "device_label": deviceLabel,
        ])
        persistLocalSettings()
        ensureFileProviderDomainIfProfileExists()
    }

    func restoreProfile() {
        guard !restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let secret = restoreSecret
        restoreSecret = ""
        profileUsername = ""
        profilePhotoName = ""
        dispatch([
            "type": "restore_profile",
            "secret": secret,
            "device_label": deviceLabel,
        ])
        persistLocalSettings()
        ensureFileProviderDomainIfProfileExists()
    }

    func linkDevice() {
        let owner = ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !owner.isEmpty else {
            return
        }
        ownerPublicKey = owner
        dispatch([
            "type": "link_device",
            "owner_pubkey": owner,
            "device_label": deviceLabel,
        ])
        ensureFileProviderDomainIfProfileExists()
    }

    func relinkDevice() {
        linkDevice()
    }

    func approveDevice() {
        approveDevice(request: approveDeviceKey, label: approveDeviceLabel)
    }

    func approveDevice(request: String, label: String) {
        dispatch([
            "type": "approve_device",
            "request": request,
            "label": label,
        ])
        approveDeviceKey = ""
        approveDeviceLabel = ""
    }

    func resetInvite() {
        dispatch(["type": "reset_invite"])
    }

    func revokeDevice(id: String) {
        deleteDevice(id: id)
    }

    func deleteDevice(id: String) {
        dispatch([
            "type": "delete_device",
            "device_pubkey": id,
        ])
    }

    func appointAdmin(id: String) {
        dispatch([
            "type": "appoint_admin",
            "device_pubkey": id,
        ])
    }

    func demoteAdmin(id: String) {
        dispatch([
            "type": "demote_admin",
            "device_pubkey": id,
        ])
    }

    func logout() {
        stopSync()
        dispatch(["type": "logout"])
        restoreSecret = ""
        approveDeviceKey = ""
        approveDeviceLabel = ""
        profileUsername = ""
        profilePhotoName = ""
        fileProviderError = ""
        fileProviderStatus = "Files provider not registered"
        removeFileProviderDomain()
        persistLocalSettings()
    }

    func revokeDevice(label: String) {
        if let device = devices.first(where: { $0.label == label }) {
            deleteDevice(id: device.id)
        }
    }

    func startSync() {
        guard isSetupComplete else { return }
        dispatch(["type": "start_sync"])
    }

    func stopSync() {
        dispatch(["type": "stop_sync"])
    }

    func restartSync() {
        guard isSetupComplete else { return }
        dispatch(["type": "restart_sync"])
    }

    func copyOwnerKey() {
        UIPasteboard.general.string = ownerPublicKey
    }

    func copyDeviceKey() {
        UIPasteboard.general.string = devicePublicKey
    }

    func copyLinkRequest() {
        UIPasteboard.general.string = deviceLinkRequest
    }

    func copyLinkInvite() {
        UIPasteboard.general.string = deviceLinkInvite
    }

    func qrMatrix(for value: String) -> QrMatrix {
        nativeCore.qrMatrix(text: value)
    }

    func copySnapshotLink() {
        guard !snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        UIPasteboard.general.string = snapshotLink
    }

    func openSnapshotLink() {
        guard let url = URL(string: snapshotLink) else { return }
        UIApplication.shared.open(url)
    }

    func addRelay(_ value: String? = nil) {
        let candidate = (value ?? relayInput).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !candidate.isEmpty else { return }
        if !relays.contains(candidate) {
            dispatch([
                "type": "add_relay",
                "url": candidate,
            ])
        }
        relayInput = ""
        persistLocalSettings()
    }

    func removeRelay(_ value: String) {
        dispatch([
            "type": "remove_relay",
            "url": value,
        ])
    }

    func resetRelay() {
        resetRelays()
    }

    func resetRelays() {
        dispatch(["type": "reset_relays"])
        relayInput = ""
        persistLocalSettings()
    }

    func resetLocalState() {
        try? FileManager.default.removeItem(at: IrisDriveSharedContainer.baseDirectory)
        lastState = nil
        restoreSecret = ""
        syncRunning = false
        statusTitle = "Ready"
        statusDetail = "Waiting for this device to be linked."
        profileUsername = ""
        profilePhotoName = ""
        #if DEBUG
        defaults.removeObject(forKey: fileProviderDebugRegistrationVersionKey)
        #endif
        persistLocalSettings()
        applyStateJson(nativeCore.refreshJson())
    }

    func handle(url: URL) {
        let linkInput = IrisDriveNativeLinkInput.classify(url.absoluteString)
        if linkInput.kind == "invite" {
            ownerPublicKey = url.absoluteString
            linkDevice()
            ensureFileProviderDomainIfProfileExists()
            return
        }

        guard linkInput.kind == "device_approval" else {
            statusTitle = "Iris link opened"
            statusDetail = url.absoluteString
            persist()
            load()
            return
        }

        if hasOwnerAuthority, linkInput.isComplete {
            approveDevice(request: url.absoluteString, label: "Linked device")
            return
        }

        statusTitle = hasOwnerAuthority ? "Invalid device invite" : "Open on an owner device"
        statusDetail = hasOwnerAuthority
            ? (linkInput.error.isEmpty ? url.absoluteString : linkInput.error)
            : "Open this request on an owner device, or scan an invite link to join."
    }

    func handleDebugLaunchEnvironment() {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        guard environment["IRIS_DRIVE_DEBUG_ACTION"] == "link-device",
              let owner = environment["IRIS_DRIVE_DEBUG_OWNER"],
              !owner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return
        }
        ownerPublicKey = owner
        linkDevice()
        #endif
    }

    private func load() {
        removeObsoletePrototypeDefaults()
        applyStateJson(nativeCore.refreshJson())
        deviceLabel = defaults.string(forKey: "deviceLabel") ?? UIDevice.current.name
        profileUsername = defaults.string(forKey: "profileUsername") ?? profileUsername
        profilePhotoName = defaults.string(forKey: "profilePhotoName") ?? profilePhotoName
        relayInput = ""
        syncOverCellular = defaults.bool(forKey: "syncOverCellular")
    }

    func persist() {
        persistLocalSettings()
    }

    private func persistLocalSettings() {
        defaults.set(deviceLabel, forKey: "deviceLabel")
        defaults.set(profileUsername, forKey: "profileUsername")
        defaults.set(profilePhotoName, forKey: "profilePhotoName")
        defaults.set(syncOverCellular, forKey: "syncOverCellular")
    }

    private func removeObsoletePrototypeDefaults() {
        [
            "approvedDevices",
            "devicePublicKey",
            "hasOwnerAuthority",
            "ownerPublicKey",
            "relay",
            "relays",
            "statusDetail",
            "statusTitle",
            "syncRunning",
        ].forEach(defaults.removeObject)
    }

    private func rebuildDerivedState() {
        guard let state = lastState else {
            ownerPublicKey = ""
            devicePublicKey = "local-device"
            authorizationState = "Not linked"
            devices = []
            inboundDeviceLinkRequests = []
            roots = []
            backups = []
            relays = defaultRelays
            relayStatuses = defaultRelayStatuses
            relay = defaultRelay
            syncRunning = false
            statusTitle = "Ready"
            statusDetail = "Waiting for this device to be linked."
            currentProviderSignalKey = ""
            lastProviderSignalKey = ""
            currentProviderDirectoryPaths = []
            onlineDeviceCount = 0
            return
        }

        ownerPublicKey = state.ui.account?.ownerPubkey ?? ""
        devicePublicKey = state.ui.account?.devicePubkey ?? "local-device"
        deviceLabel = state.ui.account?.deviceLabel.isEmpty == false
            ? state.ui.account?.deviceLabel ?? deviceLabel
            : deviceLabel
        syncRunning = state.ui.sync.running
        authorizationState = state.ui.setupLabel
        statusTitle = state.ui.primaryStatusLabel
        statusDetail = state.error.isEmpty ? syncStateTitle : state.error
        if !fileProviderError.isEmpty {
            statusTitle = "Open in Files failed"
            statusDetail = fileProviderError
        }
        relays = state.ui.relays.isEmpty ? defaultRelays : state.ui.relays
        relayStatuses = state.ui.relayStatuses.isEmpty
            ? relays.map {
                IrisDriveRelayStatus(
                    url: $0,
                    status: "configured",
                    statusLabel: "saved",
                    health: "configured"
                )
            }
            : state.ui.relayStatuses.map(IrisDriveRelayStatus.init)
        relay = relays.first ?? defaultRelay
        devices = state.ui.devices.map { device in
            return IrisDriveDevice(
                label: device.displayLabel,
                role: device.roleLabel,
                state: device.stateLabel,
                connectionState: device.connectionState,
                connectionLabel: device.connectionLabel,
                detail: device.detail,
                isCurrentDevice: device.isCurrentDevice,
                isOnline: device.isOnline,
                canRevoke: device.canRevoke,
                canAppointAdmin: device.canAppointAdmin,
                canDemoteAdmin: device.canDemoteAdmin
            )
        }
        inboundDeviceLinkRequests = state.ui.account?.inboundDeviceLinkRequests.map { request in
            IrisDriveDeviceLinkRequest(
                devicePubkey: request.devicePubkey,
                label: request.label,
                requestedAt: request.requestedAt,
                requestLink: request.requestLink
            )
        } ?? []
        authorizedDeviceCount = Int(state.ui.authorizedDeviceCount)
        onlineDeviceCount = Int(state.ui.onlineDeviceCount)
        fileCount = Int(state.ui.fileCount)
        visibleFileBytes = state.ui.visibleFileBytes
        let signal = loadProviderSignalSummary()
        currentProviderSignalKey = signal.changeKey
        currentProviderDirectoryPaths = signal.directoryPaths
        signalFileProviderIfNeeded()
        backups = state.ui.backups.map { backup in
            IrisDriveBackup(
                label: backup.label,
                state: backup.state,
                detail: backup.detail
            )
        }
        roots = state.ui.roots.map { root in
            IrisDriveRoot(name: root.name, status: root.status, path: root.localPath)
        }
    }

    private func dispatch(_ action: [String: Any]) {
        guard let actionJson = encodeNativeAction(action) else {
            statusTitle = "Native action failed"
            statusDetail = "Unable to encode action."
            return
        }
        applyStateJson(nativeCore.dispatchJson(actionJson))
    }

    private func dispatchInBackground(_ action: [String: Any]) async {
        guard let actionJson = encodeNativeAction(action) else {
            statusTitle = "Native action failed"
            statusDetail = "Unable to encode action."
            return
        }
        let json = await runNativeInBackground { nativeCore in
            nativeCore.dispatchJson(actionJson)
        }
        guard !Task.isCancelled else { return }
        applyStateJson(json)
    }

    private func refreshInBackground() async {
        let json = await runNativeInBackground { nativeCore in
            nativeCore.refreshJson()
        }
        guard !Task.isCancelled else { return }
        applyStateJson(json)
        ensureFileProviderDomainIfProfileExists()
    }

    private func runNativeInBackground(
        _ operation: @escaping @Sendable (IrisDriveNativeCore) -> String
    ) async -> String {
        let nativeCore = nativeCore
        return await withCheckedContinuation { continuation in
            let thread = Thread {
                continuation.resume(returning: operation(nativeCore))
            }
            thread.name = "IrisDriveNativeCore"
            thread.qualityOfService = .utility
            thread.stackSize = nativeBackgroundStackSize
            thread.start()
        }
    }

    private func encodeNativeAction(_ action: [String: Any]) -> String? {
        guard let data = try? JSONSerialization.data(withJSONObject: action) else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    private func applyStateJson(_ json: String) {
        guard let data = json.data(using: .utf8),
              let state = try? JSONDecoder().decode(NativeAppState.self, from: data)
        else {
            statusTitle = "Native state failed"
            statusDetail = json
            writeDebugState(json)
            return
        }
        lastState = state
        rebuildDerivedState()
        writeDebugState(json)
    }

    private func writeDebugState(_ json: String) {
        #if DEBUG
        let url = IrisDriveSharedContainer.baseDirectory
            .appendingPathComponent(iosDebugStateFileName, isDirectory: false)
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try? json.write(to: url, atomically: true, encoding: .utf8)
        #endif
    }

    private struct ProviderState: Decodable {
        var anchor: String?
        var directoryPaths: [String]
        var changeKey: String

        enum CodingKeys: String, CodingKey {
            case anchor
            case directoryPaths = "directory_paths"
            case changeKey = "change_key"
        }
    }

    private func loadProviderSignalSummary() -> (
        changeKey: String,
        directoryPaths: [String]
    ) {
        guard let data = IrisDriveNativeProvider
            .list(dataDir: IrisDriveSharedContainer.baseDirectory.path)
            .data(using: .utf8),
              let state = try? JSONDecoder().decode(ProviderState.self, from: data)
        else {
            return ("", [])
        }
        return (state.changeKey.isEmpty ? state.anchor ?? "" : state.changeKey, state.directoryPaths)
    }

    private func signalFileProviderIfNeeded() {
        guard isSetupComplete, !currentProviderSignalKey.isEmpty else { return }
        guard currentProviderSignalKey != lastProviderSignalKey else { return }
        let domain = irisDriveFileProviderDomain()
        guard let manager = NSFileProviderManager(for: domain) else { return }
        lastProviderSignalKey = currentProviderSignalKey
        var identifiers: [NSFileProviderItemIdentifier] = [.rootContainer, .workingSet]
        identifiers.append(contentsOf: currentProviderDirectoryPaths.map(fileProviderIdentifier))
        for identifier in identifiers {
            manager.signalEnumerator(for: identifier) { error in
                if let error {
                    NSLog("Iris Drive Files provider signal failed for \(identifier.rawValue): \(error)")
                }
            }
        }
    }

    private func fileProviderIdentifier(for path: String) -> NSFileProviderItemIdentifier {
        guard !path.isEmpty else { return .rootContainer }
        return NSFileProviderItemIdentifier(
            "\(fileProviderPathIdentifierPrefix)\(Data(path.utf8).base64EncodedString())"
        )
    }

    private func deviceKey(from request: String) -> String {
        let linkInput = IrisDriveNativeLinkInput.classify(request)
        guard linkInput.kind == "device_approval", !linkInput.devicePubkey.isEmpty else {
            return request
        }
        return linkInput.devicePubkey
    }
}
