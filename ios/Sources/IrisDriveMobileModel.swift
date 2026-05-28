import FileProvider
import Foundation
import SwiftUI
import UIKit

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var state: String
    var detail: String
    var isOnline: Bool
    var canRevoke: Bool
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
    @Published var backups: [IrisDriveBackup] = []
    @Published var roots: [IrisDriveRoot] = []

    private let defaults = UserDefaults.standard
    private let approvedDevicesKey = "approvedDevices"
    private let relaysKey = "relays"

    private struct StoredDevice: Codable, Equatable {
        var label: String
        var key: String
    }

    init() {
        load()
    }

    var sharedContainerPath: String {
        IrisDriveSharedContainer.baseDirectory.path
    }

    var configPath: String {
        IrisDriveSharedContainer.baseDirectory
            .appendingPathComponent("config.toml", isDirectory: false)
            .path
    }

    var blocksPath: String {
        IrisDriveSharedContainer.baseDirectory
            .appendingPathComponent("blocks", isDirectory: true)
            .path
    }

    var statusSymbol: String {
        hasLocalProfile ? "checkmark.circle.fill" : "link.circle"
    }

    var statusTint: Color {
        hasLocalProfile ? .green : .orange
    }

    var syncStateTitle: String {
        syncRunning ? "Sync running" : "Sync paused"
    }

    var snapshotLink: String {
        "https://drive.iris.to/snapshot/\(ownerPublicKey.isEmpty ? "local" : ownerPublicKey)"
    }

    var deviceLinkRequest: String {
        guard hasLocalProfile else { return "" }
        return "iris-drive://device-link?owner=\(ownerPublicKey)&device=\(devicePublicKey)"
    }

    var hasLocalProfile: Bool {
        !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var hasOwnerAuthority: Bool {
        hasLocalProfile && defaults.bool(forKey: "hasOwnerAuthority")
    }

    func ensureFileProviderDomainIfProfileExists() {
        guard hasLocalProfile else {
            fileProviderStatus = "Files provider not registered"
            rebuildDerivedState()
            return
        }
        ensureFileProviderDomain()
    }

    func ensureFileProviderDomain() {
        guard hasLocalProfile else {
            fileProviderStatus = "Files provider not registered"
            rebuildDerivedState()
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
                    self?.fileProviderStatus = "Files provider ready"
                    self?.rebuildDerivedState()
                }
                return
            }

            NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
                let exists = domains.contains { $0.identifier == irisDriveDomainIdentifier }
                Task { @MainActor [weak self] in
                    self?.fileProviderStatus = exists
                        ? "Files provider ready"
                        : "Files provider unavailable"
                    self?.rebuildDerivedState()
                }
            }
        }
    }

    func openDriveFolder() {
        ensureFileProviderDomain()
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
                guard let url else {
                    self?.fileProviderStatus = "Files provider URL unavailable"
                    self?.rebuildDerivedState()
                    return
                }
                UIApplication.shared.open(url)
            }
        }
    }

    func refresh() {
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func createProfile(username: String = "", profilePhotoName: String = "") {
        ownerPublicKey = "local-owner"
        statusTitle = "Linked"
        statusDetail = syncStateTitle
        devicePublicKey = "device-\(UUID().uuidString.prefix(8))"
        profileUsername = username.trimmingCharacters(in: .whitespacesAndNewlines)
        self.profilePhotoName = profileUsername.isEmpty ? "" : profilePhotoName
        defaults.set(true, forKey: "hasOwnerAuthority")
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func restoreProfile() {
        guard !restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        ownerPublicKey = "restored-owner"
        statusTitle = "Restored"
        statusDetail = syncStateTitle
        devicePublicKey = "device-\(UUID().uuidString.prefix(8))"
        restoreSecret = ""
        profileUsername = ""
        profilePhotoName = ""
        defaults.set(true, forKey: "hasOwnerAuthority")
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func linkDevice() {
        guard !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        if devicePublicKey == "local-device" {
            devicePublicKey = "device-\(UUID().uuidString.prefix(8))"
        }
        statusTitle = "Link requested"
        statusDetail = "Approval is pending from an owner device."
        defaults.set(false, forKey: "hasOwnerAuthority")
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func approveDevice() {
        approveDevice(request: approveDeviceKey, label: approveDeviceLabel)
    }

    func approveDevice(request: String, label: String) {
        guard hasOwnerAuthority else {
            statusTitle = "Owner profile required"
            statusDetail = "Only an owner device can approve linked devices."
            return
        }
        let key = deviceKey(from: request).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        var stored = loadApprovedDevices()
        let device = StoredDevice(
            label: label.isEmpty ? "Linked device" : label,
            key: key
        )
        if let index = stored.firstIndex(where: { $0.key == key }) {
            stored[index] = device
        } else {
            stored.append(device)
        }
        saveApprovedDevices(stored)
        approveDeviceKey = ""
        approveDeviceLabel = ""
        statusTitle = "Device approved"
        statusDetail = syncStateTitle
        persist()
        load()
    }

    func revokeDevice(id: String) {
        let stored = loadApprovedDevices().filter { $0.key != id }
        saveApprovedDevices(stored)
        statusTitle = "Device revoked"
        statusDetail = syncStateTitle
        load()
    }

    func revokeDevice(label: String) {
        let stored = loadApprovedDevices().filter { $0.label != label }
        saveApprovedDevices(stored)
        statusTitle = "Device revoked"
        statusDetail = syncStateTitle
        load()
    }

    func startSync() {
        guard hasLocalProfile else { return }
        syncRunning = true
        statusTitle = "Sync running"
        statusDetail = "Foreground sync is active."
        persist()
        load()
    }

    func stopSync() {
        syncRunning = false
        statusTitle = "Sync paused"
        statusDetail = "Foreground sync is paused."
        persist()
        load()
    }

    func restartSync() {
        guard hasLocalProfile else { return }
        syncRunning = true
        statusTitle = "Sync restarted"
        statusDetail = "Foreground sync is active."
        persist()
        load()
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
            relays.append(candidate)
        }
        relay = relays.first ?? defaultRelay
        relayInput = ""
        persist()
    }

    func removeRelay(_ value: String) {
        relays.removeAll { $0 == value }
        if relays.isEmpty {
            relays = defaultRelays
        }
        relay = relays.first ?? defaultRelay
        persist()
    }

    func resetRelay() {
        resetRelays()
    }

    func resetRelays() {
        relays = defaultRelays
        relay = defaultRelay
        relayInput = ""
        persist()
    }

    func resetLocalState() {
        ownerPublicKey = ""
        devicePublicKey = "local-device"
        restoreSecret = ""
        syncRunning = false
        statusTitle = "Ready"
        statusDetail = "Waiting for this device to be linked."
        profileUsername = ""
        profilePhotoName = ""
        defaults.set(false, forKey: "hasOwnerAuthority")
        saveApprovedDevices([])
        persist()
        load()
    }

    func handle(url: URL) {
        guard isDeviceLink(url) else {
            statusTitle = "Iris link opened"
            statusDetail = url.absoluteString
            persist()
            load()
            return
        }

        let owner = queryValue("owner", in: url)
        let device = queryValue("device", in: url)
        if hasOwnerAuthority, device != nil {
            approveDevice(request: url.absoluteString, label: "Linked device")
            return
        }
        if let owner, !owner.isEmpty {
            ownerPublicKey = owner
            if devicePublicKey == "local-device" {
                devicePublicKey = "device-\(UUID().uuidString.prefix(8))"
            }
            statusTitle = "Link requested"
            statusDetail = "Approval is pending from an owner device."
            defaults.set(false, forKey: "hasOwnerAuthority")
            persist()
            load()
            ensureFileProviderDomainIfProfileExists()
            return
        }

        statusTitle = "Invalid device link"
        statusDetail = device ?? url.absoluteString
    }

    private func load() {
        deviceLabel = defaults.string(forKey: "deviceLabel") ?? UIDevice.current.name
        ownerPublicKey = defaults.string(forKey: "ownerPublicKey") ?? ownerPublicKey
        devicePublicKey = defaults.string(forKey: "devicePublicKey") ?? devicePublicKey
        statusTitle = defaults.string(forKey: "statusTitle") ?? statusTitle
        statusDetail = defaults.string(forKey: "statusDetail") ?? statusDetail
        profileUsername = defaults.string(forKey: "profileUsername") ?? profileUsername
        profilePhotoName = defaults.string(forKey: "profilePhotoName") ?? profilePhotoName
        relay = defaults.string(forKey: "relay") ?? relay
        relays = loadRelays()
        relayInput = ""
        syncOverCellular = defaults.bool(forKey: "syncOverCellular")
        syncRunning = defaults.bool(forKey: "syncRunning")
        rebuildDerivedState()
    }

    func persist() {
        defaults.set(deviceLabel, forKey: "deviceLabel")
        defaults.set(ownerPublicKey, forKey: "ownerPublicKey")
        defaults.set(devicePublicKey, forKey: "devicePublicKey")
        defaults.set(statusTitle, forKey: "statusTitle")
        defaults.set(statusDetail, forKey: "statusDetail")
        defaults.set(profileUsername, forKey: "profileUsername")
        defaults.set(profilePhotoName, forKey: "profilePhotoName")
        defaults.set(relay, forKey: "relay")
        defaults.set(syncOverCellular, forKey: "syncOverCellular")
        defaults.set(syncRunning, forKey: "syncRunning")
        saveRelays(relays)
    }

    private func rebuildDerivedState() {
        let authorizationState: String
        if ownerPublicKey.isEmpty {
            authorizationState = "Not linked"
        } else if hasOwnerAuthority {
            authorizationState = "Authorized"
        } else {
            authorizationState = "Awaiting approval"
        }

        let currentDevice = IrisDriveDevice(
            label: deviceLabel,
            state: authorizationState,
            detail: ownerPublicKey.isEmpty ? "No owner profile on this device." : devicePublicKey,
            isOnline: hasLocalProfile && syncRunning,
            canRevoke: false
        )
        let approved = loadApprovedDevices().map { device in
            IrisDriveDevice(
                label: device.label,
                state: "Authorized",
                detail: device.key,
                isOnline: syncRunning,
                canRevoke: true
            )
        }
        devices = [currentDevice] + approved
        backups = defaultBlossomServers.map { server in
            IrisDriveBackup(
                label: "Blossom fallback",
                state: ownerPublicKey.isEmpty ? "Paused" : "Ready",
                detail: server
            )
        }
        roots = hasLocalProfile
            ? [
                IrisDriveRoot(
                    name: driveName,
                    status: fileProviderStatus,
                    path: sharedContainerPath
                ),
            ]
            : []
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

    private func isDeviceLink(_ url: URL) -> Bool {
        (url.scheme == "iris-drive" && url.host == "device-link")
            || (url.scheme == "https" && url.host == "drive.iris.to" && url.path == "/device-link")
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
