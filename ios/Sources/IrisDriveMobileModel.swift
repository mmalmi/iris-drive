import FileProvider
import Foundation
import SwiftUI
import UIKit

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var role: String
    var state: String
    var detail: String
    var isOnline: Bool
    var canRevoke: Bool
    var canAppointAdmin: Bool
    var canDemoteAdmin: Bool
}

struct IrisDriveDeviceLinkRequest: Identifiable, Equatable {
    var id: String { devicePubkey }
    var devicePubkey: String
    var label: String
    var requestedAt: UInt64
    var requestLink: String
}

struct IrisDriveBackup: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var state: String
    var detail: String
}

struct IrisDriveRoot: Identifiable, Equatable {
    var id: String { name }
    var name: String
    var status: String
    var path: String
}

enum IrisDriveSharedContainer {
    static let appGroupIdentifier = "group.to.iris.drive"

    static var baseDirectory: URL {
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent("Iris Drive", isDirectory: true)
        }
        let support = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return support.appendingPathComponent("Iris Drive", isDirectory: true)
    }
}

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveFileProviderDisplayName = "My Drive"
private let defaultRelay = "wss://relay.damus.io"
private let defaultRelays = [defaultRelay]
private let defaultBlossomServers = ["https://upload.iris.to"]
private let iosDebugStateFileName = "debug-state.json"

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
    @Published var syncOverCellular = false
    @Published var syncRunning = false
    @Published var fileProviderStatus = "Files provider not registered"
    @Published var approveDeviceKey = ""
    @Published var approveDeviceLabel = ""
    @Published var devices: [IrisDriveDevice] = []
    @Published var inboundDeviceLinkRequests: [IrisDriveDeviceLinkRequest] = []
    @Published var backups: [IrisDriveBackup] = []
    @Published var roots: [IrisDriveRoot] = []
    @Published var isDriveBrowserPresented = false
    @Published var driveBrowserInitialURL: URL?
    @Published var authorizationState = "Not linked"
    @Published var authorizedDeviceCount = 0
    @Published var fileCount = 0
    @Published var visibleFileBytes: UInt64 = 0

    private let defaults = UserDefaults.standard
    private let approvedDevicesKey = "approvedDevices"
    private let relaysKey = "relays"
    private let nativeCore: IrisDriveNativeCore
    private var lastState: NativeAppState?

    private struct StoredDevice: Codable, Equatable {
        var label: String
        var key: String
        var isAdmin: Bool

        enum CodingKeys: String, CodingKey {
            case label
            case key
            case isAdmin
        }

        init(label: String, key: String, isAdmin: Bool = false) {
            self.label = label
            self.key = key
            self.isAdmin = isAdmin
        }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            label = try container.decode(String.self, forKey: .label)
            key = try container.decode(String.self, forKey: .key)
            isAdmin = try container.decodeIfPresent(Bool.self, forKey: .isAdmin) ?? false
        }
    }

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
        isSetupComplete ? "checkmark.circle.fill" : "link.circle"
    }

    var statusTint: Color {
        isSetupComplete ? .green : .orange
    }

    var syncStateTitle: String {
        syncRunning ? "Sync on" : "Sync paused"
    }

    var snapshotLink: String {
        lastState?.ui.snapshotLink
            ?? "https://drive.iris.to/snapshot/\(ownerPublicKey.isEmpty ? "local" : ownerPublicKey)"
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
        lastState?.ui.account?.authorizationState == "authorized"
    }

    var isAwaitingApproval: Bool {
        lastState?.ui.account?.authorizationState == "awaiting_approval"
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
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDisplayName
        )
        NSFileProviderManager.add(domain) { [weak self] error in
            if error == nil {
                Task { @MainActor in
                    guard let self else { return }
                    self.fileProviderStatus = "Files provider registered"
                    self.rebuildDerivedState()
                    completion?(true)
                }
                return
            }

            NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
                let exists = domains.contains { $0.identifier == irisDriveDomainIdentifier }
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    self.fileProviderStatus = exists
                        ? "Files provider registered"
                        : "Files provider unavailable"
                    self.rebuildDerivedState()
                    completion?(exists)
                }
            }
        }
    }

    func openDriveFolder() {
        guard isSetupComplete else {
            fileProviderStatus = "Files provider not registered"
            rebuildDerivedState()
            return
        }
        fileProviderStatus = "Opening Files provider"
        rebuildDerivedState()
        ensureFileProviderDomain { [weak self] ready in
            guard let self else { return }
            guard ready else {
                self.fileProviderStatus = "Files provider unavailable"
                self.rebuildDerivedState()
                return
            }
            self.openRegisteredDriveFolder()
        }
    }

    private func openRegisteredDriveFolder() {
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDisplayName
        )
        guard let manager = NSFileProviderManager(for: domain) else {
            fileProviderStatus = "Files provider unavailable"
            rebuildDerivedState()
            return
        }
        manager.getUserVisibleURL(for: .rootContainer) { [weak self] url, _ in
            Task { @MainActor in
                guard let self else { return }
                self.driveBrowserInitialURL = url
                self.isDriveBrowserPresented = true
                self.fileProviderStatus = url == nil
                    ? "Files provider registered"
                    : "Files provider open"
                self.rebuildDerivedState()
            }
        }
    }

    private func removeFileProviderDomain() {
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDisplayName
        )
        NSFileProviderManager.remove(domain) { _ in }
    }

    func refresh() {
        applyStateJson(nativeCore.refreshJson())
        ensureFileProviderDomainIfProfileExists()
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
        dispatch([
            "type": "revoke_device",
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
        fileProviderStatus = "Files provider not registered"
        removeFileProviderDomain()
        persistLocalSettings()
    }

    func revokeDevice(label: String) {
        if let device = devices.first(where: { $0.label == label }) {
            revokeDevice(id: device.id)
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
        persistLocalSettings()
        applyStateJson(nativeCore.refreshJson())
    }

    func handle(url: URL) {
        if isLinkDevice(url) {
            ownerPublicKey = url.absoluteString
            linkDevice()
            ensureFileProviderDomainIfProfileExists()
            return
        }

        guard isDeviceLink(url) else {
            statusTitle = "Iris link opened"
            statusDetail = url.absoluteString
            persist()
            load()
            return
        }

        let device = queryValue("device", in: url)
        if hasOwnerAuthority, device != nil {
            approveDevice(request: url.absoluteString, label: "Linked device")
            return
        }

        statusTitle = hasOwnerAuthority ? "Invalid device invite" : "Open on an owner device"
        statusDetail = hasOwnerAuthority
            ? (device ?? url.absoluteString)
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
            relay = defaultRelay
            syncRunning = false
            statusTitle = "Ready"
            statusDetail = "Waiting for this device to be linked."
            return
        }

        ownerPublicKey = state.ui.account?.ownerPubkey ?? ""
        devicePublicKey = state.ui.account?.devicePubkey ?? "local-device"
        deviceLabel = state.ui.account?.deviceLabel.isEmpty == false
            ? state.ui.account?.deviceLabel ?? deviceLabel
            : deviceLabel
        syncRunning = state.ui.sync.running
        authorizationState = authorizationTitle(state.ui.account?.authorizationState)
        statusTitle = ownerPublicKey.isEmpty
            ? "Ready"
            : (isAwaitingApproval ? "Waiting for approval" : "Ready")
        statusDetail = state.error.isEmpty ? syncStateTitle : state.error
        relays = state.ui.relays.isEmpty ? defaultRelays : state.ui.relays
        relay = relays.first ?? defaultRelay
        devices = state.ui.devices.map { device in
            IrisDriveDevice(
                label: device.label.isEmpty ? "This device" : device.label,
                role: roleTitle(device.role),
                state: deviceStateTitle(device.state),
                detail: device.detail,
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
        authorizedDeviceCount = devices.count
        let stats = loadProviderStats()
        fileCount = stats.fileCount
        visibleFileBytes = stats.visibleFileBytes
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
        guard let data = try? JSONSerialization.data(withJSONObject: action),
              let actionJson = String(data: data, encoding: .utf8)
        else {
            statusTitle = "Native action failed"
            statusDetail = "Unable to encode action."
            return
        }
        applyStateJson(nativeCore.dispatchJson(actionJson))
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

    private func authorizationTitle(_ value: String?) -> String {
        switch value {
        case "authorized", "Authorized":
            "Linked"
        case "awaiting_approval", "Awaiting approval":
            "Awaiting approval"
        case "revoked", "Revoked":
            "Revoked"
        case "Admin":
            "Admin"
        default:
            ownerPublicKey.isEmpty ? "Not linked" : (value ?? "Linked")
        }
    }

    private func deviceStateTitle(_ value: String?) -> String {
        switch value {
        case "awaiting_approval", "Awaiting approval":
            "Awaiting approval"
        case "revoked", "Revoked":
            "Revoked"
        default:
            "Linked"
        }
    }

    private func roleTitle(_ value: String) -> String {
        switch value {
        case "admin":
            "Admin"
        case "member":
            "Member"
        default:
            value
        }
    }

    private func loadApprovedDevices() -> [StoredDevice] {
        guard let data = defaults.data(forKey: approvedDevicesKey),
              let devices = try? JSONDecoder().decode([StoredDevice].self, from: data)
        else {
            return []
        }
        return devices
    }

    private func saveApprovedDevices(_ devices: [StoredDevice]) {
        guard let data = try? JSONEncoder().encode(devices) else { return }
        defaults.set(data, forKey: approvedDevicesKey)
    }

    private func loadRelays() -> [String] {
        guard let data = defaults.data(forKey: relaysKey),
              let relays = try? JSONDecoder().decode([String].self, from: data),
              !relays.isEmpty
        else {
            return [relay].filter { !$0.isEmpty }
        }
        return relays
    }

    private func saveRelays(_ relays: [String]) {
        guard let data = try? JSONEncoder().encode(relays) else { return }
        defaults.set(data, forKey: relaysKey)
    }

    private struct ProviderState: Decodable {
        var entries: [ProviderEntry]
    }

    private struct ProviderEntry: Decodable {
        var path: String
        var kind: String
        var size: UInt64
    }

    private func loadProviderStats() -> (
        fileCount: Int,
        visibleFileBytes: UInt64
    ) {
        guard let data = IrisDriveNativeProvider
            .list(dataDir: IrisDriveSharedContainer.baseDirectory.path)
            .data(using: .utf8),
              let state = try? JSONDecoder().decode(ProviderState.self, from: data)
        else {
            return (0, 0)
        }
        let fileEntries = state.entries.filter { $0.kind != "directory" }
        let visibleFileBytes = fileEntries.reduce(UInt64(0)) { $0 + $1.size }
        return (fileEntries.count, visibleFileBytes)
    }

    private func isDeviceLink(_ url: URL) -> Bool {
        (url.scheme == "iris-drive" && url.host == "device-link")
            || (url.scheme == "iris-drive" && url.host == nil && url.path == "/device-link")
            || (url.scheme == "https" && url.host == "drive.iris.to" && url.path == "/device-link")
    }

    private func isLinkDevice(_ url: URL) -> Bool {
        (url.scheme == "iris-drive" && url.host == "link-device")
            || (url.scheme == "iris-drive" && url.host == "invite")
            || (url.scheme == "iris-drive" && url.host == nil && url.path == "/link-device")
            || (url.scheme == "iris-drive" && url.host == nil && url.path.starts(with: "/invite/"))
            || (url.scheme == "https" && url.host == "drive.iris.to" && url.path == "/link-device")
            || (url.scheme == "https" && url.host == "drive.iris.to" && url.path.starts(with: "/invite/"))
    }

    private func queryValue(_ name: String, in url: URL) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?
            .first { $0.name == name }?
            .value
    }

    private func deviceKey(from request: String) -> String {
        guard let url = URL(string: request),
              isDeviceLink(url),
              let device = queryValue("device", in: url)
        else {
            return request
        }
        return device
    }
}
