import BackgroundTasks
import Darwin
@preconcurrency import FileProvider
import Foundation
import SwiftUI
import UIKit

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("primary-v1")
private let irisDriveLegacyDomainIdentifiers = [
    NSFileProviderDomainIdentifier("main"),
]
private let irisDriveFileProviderDisplayName = "Iris Drive"
private let defaultRelay = "wss://relay.damus.io"
private let defaultRelays = [defaultRelay]
private let defaultBlossomServers = ["https://upload.iris.to"]
private let iosDebugStateFileName = "debug-state.json"
private let configMutationAuditDefaultsKey = "configMutationAuditV1"
private let configMutationAuditMaxEvents = 20
private let fileProviderPathIdentifierPrefix = "path:"
private let fileProviderRegistrationIdentityKey = "fileProviderRegistrationIdentity"
private let fileProviderRegistrationVersion = 5
private let fileProviderRegistrationVersionKey = "fileProviderRegistrationVersion"
private let fileProviderAddRetryLimit = 8
private let fileProviderVisibleURLRetryLimit = 12
private let fileProviderRemovalWaitPolls = 450
private let providerRootSignalFileName = "provider-root.changed"
private let nativeFipsStatusFileName = "native-fips-status.json"
private let irisWebPublisherProfileNameCacheKey = "irisWebPublisherProfileNameCacheV1"
private let irisWebPublisherProfileNameFreshAge: TimeInterval = 24 * 60 * 60
private let irisWebPublisherProfileNameMaxCacheAge: TimeInterval = 30 * 24 * 60 * 60
private let irisWebPublisherProfileNameMissCooldown: TimeInterval = 5 * 60
private let irisWebPublisherProfileFetchTimeout: TimeInterval = 4
private let appleCalendarSyncCheckInterval: TimeInterval = 60
#if DEBUG
private let fileProviderDebugRegistrationVersion = 2
private let fileProviderDebugRegistrationVersionKey = "fileProviderDebugRegistrationVersion"
#endif

private struct IrisWebPublisherProfileNameCacheEntry: Codable {
    var name: String
    var fetchedAt: TimeInterval
}

private struct ConfigIdentitySnapshot: Codable {
    var hasProfile: Bool
    var setupState: String
    var profileId: String
    var currentAppKeyNpub: String
    var currentAppKeyLabel: String
}

private struct ConfigMutationAuditEvent: Codable {
    var timestamp: String
    var action: String
    var debugAction: String
    var before: ConfigIdentitySnapshot
    var after: ConfigIdentitySnapshot
    var error: String
}

struct IrisWebRoute: Identifiable {
    let id = UUID()
    let url: URL
}

@MainActor
final class IrisDriveMobileModel: ObservableObject {
    @Published var driveName = "My Drive"
    @Published var statusTitle = "Ready"
    @Published var statusDetail = "Waiting for this device to be linked."
    @Published var stateLoaded = false
    @Published var deviceLabel = UIDevice.current.name
    @Published var profileLinkTarget = ""
    @Published var currentAppKeyNpub = ""
    @Published var devicePublicKey = "local-device"
    @Published var restoreSecret = ""
    @Published var profileUsername = ""
    @Published var profilePhotoName = ""
    @Published var relay = defaultRelay
    @Published var relayInput = ""
    @Published var backupTargetInput = ""
    @Published var backupTargetLabelInput = ""
    @Published var blossomEndpointInput = ""
    @Published var shareSourceInput = ""
    @Published var shareInviteInput = ""
    @Published var shareRecipientNpubHint = ""
    @Published var shareRecipientDisplayName = ""
    @Published var shareRecipientProfileId = ""
    @Published var shareDialogRequestId: UInt64 = 0
    @Published var relays = defaultRelays
    @Published var relayStatuses: [IrisDriveRelayStatus] = []
    @Published var syncOverCellular = false
    @Published var syncRunning = false
    @Published var fileProviderStatus = "Files provider not registered"
    @Published var approveDeviceKey = ""
    @Published var devices: [IrisDriveDevice] = []
    @Published var inboundAppKeyLinkRequests: [IrisDriveAppKeyLinkRequest] = []
    @Published var backups: [IrisDriveBackup] = []
    @Published var checkingBackupTargets: Set<String> = []
    @Published var backupCheckCompleted = 0
    @Published var backupCheckTotal = 0
    @Published var shares: [IrisDriveShare] = []
    @Published var roots: [IrisDriveRoot] = []
    @Published var copyFeedback = ""
    @Published var fileProviderError = ""
    @Published var authorizationState = "Not linked"
    @Published var pendingContentLink: PendingContentLink?
    @Published var authorizedDeviceCount = 0
    @Published var onlineDeviceCount = 0
    @Published var fileCount = 0
    @Published var visibleFileBytes: UInt64 = 0
    @Published var localNhashResolverEnabled = true
    @Published var sitesPortalUrl = ""
    @Published var webRoute: IrisWebRoute?
    @Published var isOpeningIrisApps = false
    @Published var appleCalendarSyncEnabled = false
    @Published var appleCalendarSyncStatus = "Off"
    @Published private var irisWebPublisherProfileNameCache: [String: IrisWebPublisherProfileNameCacheEntry] = [:]

    private let defaults = UserDefaults.standard
    private let nativeCore: IrisDriveNativeCore
    private let appleCalendarSync = IrisDriveAppleCalendarSync.shared
    private let nativeCoreQueue = DispatchQueue(label: "fi.siriusbusiness.drive.native-core", qos: .utility)
    private var lastAppliedStateJson = ""
    private var lastState: NativeAppState?
    private var fileProviderOpenAttempt = 0
    private var currentProviderSignalKey = ""
    private var lastProviderSignalKey = ""
    private var providerSignalInFlightKey = ""
    private var providerSignalRetryTask: Task<Void, Never>?
    private var currentProviderDirectoryPaths: [String] = []
    private var foregroundSyncTask: Task<Void, Never>?
    var copyFeedbackTask: Task<Void, Never>?
    private var fileProviderDomainRemovalInFlight = false
    private var stateGeneration: UInt64 = 0
    private var nativeFipsStatusDirectorySource: DispatchSourceFileSystemObject?
    private var nativeFipsStatusFileSource: DispatchSourceFileSystemObject?
    private var nativeFipsStatusDirectoryDescriptor: CInt = -1
    private var nativeFipsStatusFileDescriptor: CInt = -1
    private var nativeFipsStatusRefreshTask: Task<Void, Never>?
    var lastNativeFipsStatusFingerprint = ""
    var lastNativeFipsStatusRefreshAt = Date.distantPast
    private var providerRootSignalDirectorySource: DispatchSourceFileSystemObject?
    private var providerRootSignalFileSource: DispatchSourceFileSystemObject?
    private var providerRootSignalDirectoryDescriptor: CInt = -1
    private var providerRootSignalFileDescriptor: CInt = -1
    private var providerRootSignalRefreshTask: Task<Void, Never>?
    private var appleCalendarSyncInFlight = false
    private var lastAppleCalendarSyncCheck = Date.distantPast
    private var lastForegroundDriveSyncStartedAt = Date.distantPast
    private var irisWebPublisherProfileNameTasks: [String: Task<Void, Never>] = [:]
    private var irisWebPublisherProfileNameMisses: [String: Date] = [:]

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
        lastState?.ui.sync.statusLabel ?? "Ready"
    }

    var snapshotLink: String {
        lastState?.ui.snapshotLink ?? ""
    }

    var lastShareInvite: String {
        lastState?.ui.lastShareInvite ?? ""
    }

    var lastShareRecipientEvidence: String {
        lastState?.ui.lastShareRecipientEvidence ?? ""
    }

    var localProfileId: String {
        lastState?.ui.profile?.profileId ?? ""
    }

    var appKeyLinkRequest: String {
        lastState?.ui.profile?.appKeyLinkRequest ?? ""
    }

    var appKeyLinkInvite: String {
        lastState?.ui.profile?.appKeyLinkInvite ?? ""
    }

    var hasLocalProfile: Bool {
        !currentAppKeyNpub.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var setupErrorMessage: String {
        guard !isSetupComplete else { return "" }
        if let error = lastState?.error.trimmingCharacters(in: .whitespacesAndNewlines),
           !error.isEmpty {
            return error
        }
        if statusTitle == "Native state failed" {
            return statusDetail.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return ""
    }

    var isSetupComplete: Bool {
        lastState?.ui.setupComplete ?? false
    }

    var isAwaitingApproval: Bool {
        lastState?.ui.awaitingApproval ?? false
    }

    var isRevoked: Bool {
        lastState?.ui.revoked ?? false
    }

    private var shouldRunDriveBackgroundSync: Bool {
        syncRunning && !isRevoked && (isSetupComplete || isAwaitingApproval)
    }

    private var shouldRunDriveForegroundRefresh: Bool {
        syncRunning && !isRevoked && (isSetupComplete || isAwaitingApproval)
    }

    private var shouldRunAppleCalendarSync: Bool {
        appleCalendarSync.isActive && isSetupComplete && !isRevoked
    }

    private var shouldScheduleBackgroundSync: Bool {
        shouldRunDriveBackgroundSync || shouldRunAppleCalendarSync
    }

    private var shouldRunForegroundWork: Bool {
        shouldRunDriveForegroundRefresh || shouldRunAppleCalendarSync
    }

    var canAdminProfile: Bool {
        lastState?.ui.profile?.canAdminProfile ?? false
    }

    var canExportRecoveryPhrase: Bool {
        lastState?.ui.profile?.canExportRecoveryPhrase ?? false
    }

    func ensureFileProviderDomainIfProfileExists() {
        guard stateLoaded else { fileProviderStatus = "Files provider not registered"; rebuildDerivedState(); return }
        guard isSetupComplete else {
            removeFileProviderDomain()
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
        if fileProviderDomainRemovalInFlight {
            waitForFileProviderRemovalThenEnsure(completion: completion)
            return
        }
        let domain = irisDriveFileProviderDomain()
        NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                let legacyDomains = domains.filter { irisDriveLegacyDomainIdentifiers.contains($0.identifier) }
                if !legacyDomains.isEmpty {
                    self.removeLegacyFileProviderDomains(legacyDomains, completion: completion)
                    return
                }
                if let existingDomain = domains.first(where: { $0.identifier == irisDriveDomainIdentifier }) {
                    if self.shouldRepairFileProviderRegistration(existingDomain) {
                        self.repairFileProviderRegistration(
                            existingDomain: existingDomain,
                            completion: completion
                        )
                    } else {
                        self.markFileProviderRegistrationCurrent()
                        self.fileProviderStatus = "Files provider registered"
                        self.rebuildDerivedState()
                        self.signalFileProviderIfNeeded()
                        completion?(true)
                    }
                    return
                }
                self.addFileProviderDomain(domain, completion: completion)
            }
        }
    }

    private func addFileProviderDomain(
        _ domain: NSFileProviderDomain,
        completion: ((Bool) -> Void)?,
        attempt: Int = 0
    ) {
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
            if let error {
                NSLog("Iris Drive FileProvider domain add failed attempt \(attempt + 1): \(error)")
            }

            NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
                let existingDomain = domains.first { $0.identifier == irisDriveDomainIdentifier }
                let exists = existingDomain != nil
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if let existingDomain,
                       self.shouldRepairFileProviderRegistration(existingDomain) {
                        self.repairFileProviderRegistration(
                            existingDomain: existingDomain,
                            completion: completion
                        )
                        return
                    }
                    if exists {
                        self.markFileProviderRegistrationCurrent()
                    }
                    if !exists, attempt < fileProviderAddRetryLimit {
                        self.fileProviderStatus = "Registering Files provider"
                        self.rebuildDerivedState()
                        Task { @MainActor [weak self] in
                            try? await Task.sleep(nanoseconds: 350_000_000)
                            guard let self else {
                                completion?(false)
                                return
                            }
                            self.addFileProviderDomain(
                                self.irisDriveFileProviderDomain(),
                                completion: completion,
                                attempt: attempt + 1
                            )
                        }
                        return
                    }
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

    func openDriveFolder(path: String) {
        guard isSetupComplete else {
            showFileProviderError("Link this device before opening Iris Drive in Files.")
            return
        }
        let normalized = IrisDriveNativeProvider.normalizePath(path: path).path
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
            self.openRegisteredDriveFolder(attempt: attempt, path: normalized)
        }
    }

    private func scheduleOpenInFilesTimeout(for attempt: Int) {
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 60_000_000_000)
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

    private func openRegisteredDriveFolder(attempt: Int, path: String = "", visibleURLAttempt: Int = 0) {
        let domain = irisDriveFileProviderDomain()
        guard let manager = NSFileProviderManager(for: domain) else {
            showFileProviderError("Files provider manager is unavailable.")
            return
        }
        let identifier = fileProviderIdentifier(for: path)
        manager.getUserVisibleURL(for: identifier) { [weak self] url, error in
            Task { @MainActor in
                guard let self else { return }
                guard self.fileProviderOpenAttempt == attempt else { return }
                guard let url else {
                    if self.retryOpenRegisteredDriveFolder(attempt: attempt, path: path, identifier: identifier, visibleURLAttempt: visibleURLAttempt, error: error) {
                        return
                    }
                    self.openFilesRootFallback(attempt: attempt)
                    return
                }
                guard let filesURL = self.filesAppURL(for: url) else {
                    NSLog("Iris Drive Files provider URL unsupported: \(url.absoluteString)")
                    self.showFileProviderError("Files could not locate Iris Drive.")
                    return
                }
                self.scheduleFilesRootFallbackIfStillActive(for: attempt)
                UIApplication.shared.open(filesURL, options: [:]) { [weak self] opened in
                    Task { @MainActor in
                        guard let self else { return }
                        guard self.fileProviderOpenAttempt == attempt else { return }
                        if opened {
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

    private func retryOpenRegisteredDriveFolder(
        attempt: Int,
        path: String,
        identifier: NSFileProviderItemIdentifier,
        visibleURLAttempt: Int,
        error: Error?
    ) -> Bool {
        if let error {
            NSLog("Iris Drive Files provider URL unavailable attempt \(visibleURLAttempt + 1): \(error)")
        }
        guard visibleURLAttempt < fileProviderVisibleURLRetryLimit else { return false }
        fileProviderStatus = "Opening Files provider"
        rebuildDerivedState()
        if let manager = NSFileProviderManager(for: irisDriveFileProviderDomain()) {
            if #available(iOS 16.0, *) {
                manager.reimportItems(below: .rootContainer) { error in
                    if let error { NSLog("Iris Drive Files provider open reimport failed: \(error)") }
                }
            }
            let signalIdentifiers = identifier == .rootContainer ? [identifier] : [identifier, .rootContainer]
            for signalIdentifier in signalIdentifiers {
                manager.signalEnumerator(for: signalIdentifier) { error in
                    if let error {
                        NSLog("Iris Drive Files provider open signal failed for \(signalIdentifier.rawValue): \(error)")
                    }
                }
            }
        }
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard let self, self.fileProviderOpenAttempt == attempt else { return }
            self.openRegisteredDriveFolder(attempt: attempt, path: path, visibleURLAttempt: visibleURLAttempt + 1)
        }
        return true
    }

    private func filesAppURL(for userVisibleURL: URL) -> URL? {
        guard userVisibleURL.isFileURL else {
            return userVisibleURL
        }
        var components = URLComponents(url: userVisibleURL, resolvingAgainstBaseURL: false)
        components?.scheme = "shareddocuments"
        return components?.url
    }

    private func scheduleFilesRootFallbackIfStillActive(for attempt: Int) {
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            await MainActor.run { [weak self] in
                guard let self,
                      self.fileProviderOpenAttempt == attempt,
                      self.fileProviderError.isEmpty
                else {
                    return
                }
                guard UIApplication.shared.applicationState == .active else {
                    self.fileProviderOpenAttempt += 1
                    return
                }
                self.openFilesRootFallback(attempt: attempt)
            }
        }
    }

    private func openFilesRootFallback(attempt: Int) {
        guard let filesRoot = URL(string: "shareddocuments://") else {
            showFileProviderError("Files could not locate Iris Drive.")
            return
        }
        UIApplication.shared.open(filesRoot, options: [:]) { [weak self] opened in
            Task { @MainActor in
                guard let self, self.fileProviderOpenAttempt == attempt else { return }
                if opened {
                    self.fileProviderOpenAttempt += 1
                    self.fileProviderError = ""
                    self.fileProviderStatus = "Files provider open"
                    self.rebuildDerivedState()
                } else {
                    self.showFileProviderError("Files refused to open Iris Drive.")
                }
            }
        }
    }

    private func showFileProviderError(_ message: String) {
        fileProviderOpenAttempt += 1
        fileProviderError = message
        fileProviderStatus = message
        rebuildDerivedState()
    }

    private func removeFileProviderDomain() {
        let domains = [irisDriveFileProviderDomain()]
            + irisDriveLegacyDomainIdentifiers.map(legacyFileProviderDomain)
        fileProviderDomainRemovalInFlight = true
        let group = DispatchGroup()
        for domain in domains {
            group.enter()
            NSFileProviderManager.remove(domain) { _ in
                group.leave()
            }
        }
        group.notify(queue: .main) { [weak self] in
            Task { @MainActor in
                self?.fileProviderDomainRemovalInFlight = false
            }
        }
        defaults.removeObject(forKey: fileProviderRegistrationIdentityKey)
        defaults.removeObject(forKey: fileProviderRegistrationVersionKey)
        #if DEBUG
        defaults.removeObject(forKey: fileProviderDebugRegistrationVersionKey)
        #endif
    }

    private func removeLegacyFileProviderDomains(
        _ domains: [NSFileProviderDomain],
        completion: ((Bool) -> Void)?
    ) {
        fileProviderStatus = "Repairing Files provider"
        rebuildDerivedState()
        fileProviderDomainRemovalInFlight = true
        defaults.removeObject(forKey: fileProviderRegistrationIdentityKey)
        defaults.removeObject(forKey: fileProviderRegistrationVersionKey)
        #if DEBUG
        defaults.removeObject(forKey: fileProviderDebugRegistrationVersionKey)
        #endif
        let group = DispatchGroup()
        for domain in domains {
            group.enter()
            NSFileProviderManager.remove(domain) { error in
                if let error {
                    NSLog("Iris Drive legacy FileProvider domain removal failed: \(error)")
                }
                group.leave()
            }
        }
        group.notify(queue: .main) { [weak self] in
            guard let self else {
                completion?(false)
                return
            }
            self.fileProviderDomainRemovalInFlight = false
            self.ensureFileProviderDomain(completion: completion)
        }
    }

    private func waitForFileProviderRemovalThenEnsure(completion: ((Bool) -> Void)?) {
        Task { @MainActor [weak self] in
            for _ in 0..<fileProviderRemovalWaitPolls where self?.fileProviderDomainRemovalInFlight == true {
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
            guard let self else {
                completion?(false)
                return
            }
            guard !self.fileProviderDomainRemovalInFlight else {
                self.fileProviderStatus = "Files provider unavailable"
                self.rebuildDerivedState()
                completion?(false)
                return
            }
            self.fileProviderDomainRemovalInFlight = false
            self.ensureFileProviderDomain(completion: completion)
        }
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

    private func legacyFileProviderDomain(
        identifier: NSFileProviderDomainIdentifier
    ) -> NSFileProviderDomain {
        NSFileProviderDomain(identifier: identifier, displayName: irisDriveFileProviderDisplayName)
    }

    private func markFileProviderRegistrationCurrent() {
        let identity = fileProviderRegistrationIdentity
        if identity.isEmpty {
            defaults.removeObject(forKey: fileProviderRegistrationIdentityKey)
        } else {
            defaults.set(identity, forKey: fileProviderRegistrationIdentityKey)
        }
        defaults.set(fileProviderRegistrationVersion, forKey: fileProviderRegistrationVersionKey)
        #if DEBUG
        defaults.set(fileProviderDebugRegistrationVersion, forKey: fileProviderDebugRegistrationVersionKey)
        #endif
    }

    private var fileProviderRegistrationIdentity: String {
        guard isSetupComplete,
              let account = lastState?.ui.profile,
              !account.currentAppKeyNpub.isEmpty,
              !account.devicePubkey.isEmpty
        else {
            return ""
        }
        return "\(account.currentAppKeyNpub):\(account.devicePubkey)"
    }

    private func shouldRepairFileProviderRegistration(_ domain: NSFileProviderDomain) -> Bool {
        let currentIdentity = fileProviderRegistrationIdentity
        if currentIdentity.isEmpty {
            return true
        }
        if defaults.string(forKey: fileProviderRegistrationIdentityKey) != currentIdentity {
            return true
        }
        if defaults.integer(forKey: fileProviderRegistrationVersionKey) < fileProviderRegistrationVersion {
            return true
        }
        #if DEBUG
        return shouldRepairFileProviderDebugRegistration(domain)
        #else
        return false
        #endif
    }

    private func repairFileProviderRegistration(
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
                    self.lastProviderSignalKey = ""
                    self.providerSignalInFlightKey = ""
                    self.providerSignalRetryTask?.cancel()
                    self.providerSignalRetryTask = nil
                    self.fileProviderStatus = "Files provider registered"
                    self.rebuildDerivedState()
                    self.signalFileProviderIfNeeded()
                    completion?(true)
                }
            }
        }
    }

    #if DEBUG
    private func shouldRepairFileProviderDebugRegistration(_ domain: NSFileProviderDomain) -> Bool {
        if defaults.integer(forKey: fileProviderDebugRegistrationVersionKey)
            < fileProviderDebugRegistrationVersion {
            return true
        }
        return !domain.testingModes.contains(.alwaysEnabled)
    }
    #endif

    func refresh() {
        stateGeneration &+= 1
        applyStateJson(runNative { $0.refreshJson() })
        ensureFileProviderDomainIfProfileExists()
        maybeRunAppleCalendarSync()
    }

    func refreshProfileStatusInBackground(scheduleBackgroundSync: Bool = true) async {
        await dispatchInBackground(["type": "refresh_profile"])
        ensureFileProviderDomainIfProfileExists()
        if scheduleBackgroundSync {
            scheduleBackgroundSyncIfNeeded()
        }
    }

    func refreshAfterStartup() { Task { await refreshInBackground() } }

    func scheduleBackgroundSyncIfNeeded() {
        guard shouldScheduleBackgroundSync else { return cancelBackgroundSync() }
        let request = BGAppRefreshTaskRequest(identifier: IrisDriveBackgroundSyncTask.identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: IrisDriveBackgroundSyncTask.identifier)
        do { try BGTaskScheduler.shared.submit(request) }
        catch { NSLog("Iris Drive background sync scheduling failed: \(error)") }
    }

    func cancelBackgroundSync() { BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: IrisDriveBackgroundSyncTask.identifier) }

    func performBackgroundSyncTask() async {
        await syncOnceIfRunning()
        scheduleBackgroundSyncIfNeeded()
    }

    func reconcileForegroundWork(isActive: Bool) {
        if isActive {
            startForegroundSyncLoop()
        } else {
            stopForegroundSyncLoop()
            scheduleBackgroundSyncIfNeeded()
        }
    }

    func startForegroundSyncLoop() {
        guard UIApplication.shared.applicationState == .active else {
            stopForegroundSyncLoop()
            return
        }
        guard shouldRunForegroundWork else {
            stopForegroundSyncLoop()
            return
        }
        startNativeFipsStatusWatcher()
        startProviderRootSignalWatcher()
        guard foregroundSyncTask == nil else { return }
        foregroundSyncTask = Task { @MainActor [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                await self.syncOnceIfRunning()
                do {
                    try await Task.sleep(nanoseconds: self.foregroundSyncDelayNanoseconds)
                } catch {
                    return
                }
            }
        }
    }

    func stopForegroundSyncLoop() {
        foregroundSyncTask?.cancel()
        foregroundSyncTask = nil
        stopNativeFipsStatusWatcher()
        stopProviderRootSignalWatcher()
    }

    private func startNativeFipsStatusWatcher() {
        stopNativeFipsStatusWatcher()
        let directory = IrisDriveSharedContainer.baseDirectory
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        startNativeFipsStatusDirectoryWatcher(directory: directory)
        startNativeFipsStatusFileWatcher(directory: directory)
    }

    private func stopNativeFipsStatusWatcher() {
        nativeFipsStatusRefreshTask?.cancel()
        nativeFipsStatusRefreshTask = nil
        nativeFipsStatusFileSource?.cancel()
        nativeFipsStatusFileSource = nil
        nativeFipsStatusDirectorySource?.cancel()
        nativeFipsStatusDirectorySource = nil
    }

    private func startNativeFipsStatusDirectoryWatcher(directory: URL) {
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            NSLog("Iris Drive native FIPS status directory watch unavailable: \(directory.path)")
            return
        }
        nativeFipsStatusDirectoryDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            guard self.nativeFipsStatusFileSource == nil else { return }
            if self.startNativeFipsStatusFileWatcher(directory: directory) {
                self.scheduleNativeFipsStatusRefresh()
            }
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.nativeFipsStatusDirectoryDescriptor == descriptor {
                self.nativeFipsStatusDirectoryDescriptor = -1
            }
        }
        nativeFipsStatusDirectorySource = source
        source.resume()
    }

    @discardableResult
    private func startNativeFipsStatusFileWatcher(directory: URL) -> Bool {
        nativeFipsStatusFileSource?.cancel()
        nativeFipsStatusFileSource = nil
        let statusURL = directory.appendingPathComponent(nativeFipsStatusFileName, isDirectory: false)
        let descriptor = open(statusURL.path, O_EVTONLY)
        guard descriptor >= 0 else { return false }
        nativeFipsStatusFileDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            let data = source.data
            if data.contains(.delete) || data.contains(.rename) {
                self.startNativeFipsStatusFileWatcher(directory: directory)
            }
            self.scheduleNativeFipsStatusRefresh()
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.nativeFipsStatusFileDescriptor == descriptor {
                self.nativeFipsStatusFileDescriptor = -1
            }
        }
        nativeFipsStatusFileSource = source
        source.resume()
        return true
    }

    private func scheduleNativeFipsStatusRefresh() {
        nativeFipsStatusRefreshTask?.cancel()
        nativeFipsStatusRefreshTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 50_000_000)
            guard let self else { return }
            let statusURL = IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent(nativeFipsStatusFileName, isDirectory: false)
            if !self.nativeFipsStatusRefreshIsDue(statusURL: statusURL) {
                return
            }
            NSLog("Iris Drive native FIPS status file changed")
            await self.refreshInBackground()
        }
    }

    private func startProviderRootSignalWatcher() {
        stopProviderRootSignalWatcher()
        let directory = IrisDriveSharedContainer.baseDirectory
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        startProviderRootSignalDirectoryWatcher(directory: directory)
        startProviderRootSignalFileWatcher(directory: directory)
        let signalURL = directory.appendingPathComponent(providerRootSignalFileName, isDirectory: false)
        if FileManager.default.fileExists(atPath: signalURL.path) {
            scheduleProviderRootSignalRefresh()
        }
    }

    private func stopProviderRootSignalWatcher() {
        providerRootSignalRefreshTask?.cancel()
        providerRootSignalRefreshTask = nil
        providerRootSignalFileSource?.cancel()
        providerRootSignalFileSource = nil
        providerRootSignalDirectorySource?.cancel()
        providerRootSignalDirectorySource = nil
    }

    private func startProviderRootSignalDirectoryWatcher(directory: URL) {
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            NSLog("Iris Drive provider root signal directory watch unavailable: \(directory.path)")
            return
        }
        providerRootSignalDirectoryDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            guard self.providerRootSignalFileSource == nil else { return }
            if self.startProviderRootSignalFileWatcher(directory: directory) {
                self.scheduleProviderRootSignalRefresh()
            }
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.providerRootSignalDirectoryDescriptor == descriptor {
                self.providerRootSignalDirectoryDescriptor = -1
            }
        }
        providerRootSignalDirectorySource = source
        source.resume()
    }

    @discardableResult
    private func startProviderRootSignalFileWatcher(directory: URL) -> Bool {
        providerRootSignalFileSource?.cancel()
        providerRootSignalFileSource = nil
        let signalURL = directory.appendingPathComponent(providerRootSignalFileName, isDirectory: false)
        let descriptor = open(signalURL.path, O_EVTONLY)
        guard descriptor >= 0 else { return false }
        providerRootSignalFileDescriptor = descriptor
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            let data = source.data
            if data.contains(.delete) || data.contains(.rename) {
                self.startProviderRootSignalFileWatcher(directory: directory)
            }
            self.scheduleProviderRootSignalRefresh()
        }
        source.setCancelHandler { [weak self, descriptor] in
            close(descriptor)
            guard let self else { return }
            if self.providerRootSignalFileDescriptor == descriptor {
                self.providerRootSignalFileDescriptor = -1
            }
        }
        providerRootSignalFileSource = source
        source.resume()
        return true
    }

    private func scheduleProviderRootSignalRefresh() {
        providerRootSignalRefreshTask?.cancel()
        providerRootSignalRefreshTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 50_000_000)
            guard let self else { return }
            NSLog("Iris Drive provider root signal changed")
            await self.refreshInBackground()
        }
    }

    private func syncOnceIfRunning() async {
        if !isRevoked, isSetupComplete {
            if syncRunning, foregroundDriveSyncIsDue() {
                await dispatchInBackground(["type": "restart_sync"])
            } else {
                await refreshInBackground()
                return
            }
        } else if !isRevoked, isAwaitingApproval {
            await refreshProfileStatusInBackground(scheduleBackgroundSync: false)
        }
        await runAppleCalendarSyncIfEnabled()
    }

    private func foregroundDriveSyncIsDue(now: Date = Date()) -> Bool {
        let elapsed = now.timeIntervalSince(lastForegroundDriveSyncStartedAt)
        guard elapsed < 0 || elapsed >= foregroundDriveSyncMinimumIntervalSeconds else {
            return false
        }
        lastForegroundDriveSyncStartedAt = now
        return true
    }

    private var foregroundSyncDelayNanoseconds: UInt64 {
        if isAwaitingApproval && !isSetupComplete {
            return awaitingApprovalForegroundSyncIntervalNanoseconds
        }
        return foregroundSyncIntervalNanoseconds
    }

    func createProfile(username: String = "", profilePhotoName: String = "") {
        let before = configIdentitySnapshot()
        statusTitle = "Creating profile"
        statusDetail = "Preparing this device."
        profileUsername = username.trimmingCharacters(in: .whitespacesAndNewlines)
        self.profilePhotoName = profileUsername.isEmpty ? "" : profilePhotoName
        dispatch([
            "type": "create_profile",
            "app_key_label": deviceLabel,
        ])
        persistLocalSettings()
        ensureFileProviderDomainIfProfileExists()
        scheduleBackgroundSyncIfNeeded()
        recordConfigMutation(action: "create_profile", before: before)
    }

    func restoreProfile() {
        let recoverySecret = restoreSecret
        restoreSecret = ""
        restoreProfile(recoverySecret: recoverySecret)
    }

    func restoreProfile(recoverySecret: String) {
        guard !recoverySecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let before = configIdentitySnapshot()
        profileUsername = ""
        profilePhotoName = ""
        dispatch([
            "type": "restore_profile",
            "recovery_secret": recoverySecret,
            "app_key_label": deviceLabel,
        ])
        persistLocalSettings()
        ensureFileProviderDomainIfProfileExists()
        scheduleBackgroundSyncIfNeeded()
        recordConfigMutation(action: "restore_profile", before: before)
    }

    func exportRecoverySecret() -> NativeRecoverySecretExport {
        IrisDriveNativeCore.exportRecoverySecret(dataDir: sharedContainerPath)
    }

    func linkDevice() {
        let target = profileLinkTarget.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else {
            return
        }
        profileLinkTarget = target
        let before = configIdentitySnapshot()
        Task { @MainActor [weak self] in
            guard let self else { return }
            await self.dispatchInBackground(
                [
                    "type": "link_device",
                    "link_target": target,
                    "app_key_label": self.deviceLabel,
                ],
                invalidatePendingState: true
            )
            self.ensureFileProviderDomainIfProfileExists()
            self.scheduleBackgroundSyncIfNeeded()
            self.recordConfigMutation(action: "link_device", before: before)
        }
    }

    func relinkDevice() {
        linkDevice()
    }

    func startJoinRequest() {
        let before = configIdentitySnapshot()
        Task { @MainActor [weak self] in
            guard let self else { return }
            await self.dispatchInBackground(
                [
                    "type": "start_join_request",
                    "app_key_label": self.deviceLabel,
                ],
                invalidatePendingState: true
            )
            self.ensureFileProviderDomainIfProfileExists()
            self.scheduleBackgroundSyncIfNeeded()
            self.recordConfigMutation(action: "start_join_request", before: before)
        }
    }

    func approveDevice() {
        approveDevice(request: approveDeviceKey, label: "")
    }

    func approveDevice(request: String, label: String) {
        dispatch([
            "type": "approve_device",
            "request": request,
            "label": label,
        ])
        approveDeviceKey = ""
    }

    func rejectDevice(request: String) {
        dispatch([
            "type": "reject_device",
            "request": request,
        ])
    }

    func resetInvite() {
        dispatch(["type": "reset_invite"])
    }

    func generateRecoveryKey() -> NativeGeneratedRecoveryKey {
        IrisDriveNativeCore.generateRecoveryKey()
    }

    func recoveryPubkey(forPhrase phrase: String) -> NativeGeneratedRecoveryKey {
        IrisDriveNativeCore.recoveryPubkey(forPhrase: phrase)
    }

    func addRecoveryKey(pubkey: String) {
        let pubkey = pubkey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !pubkey.isEmpty else { return }
        dispatch([
            "type": "add_recovery_device",
            "recovery_pubkey": pubkey,
        ])
    }

    func revokeDevice(id: String) {
        deleteDevice(id: id)
    }

    func deleteDevice(id: String) {
        dispatch([
            "type": "delete_device",
            "app_key_pubkey": id,
        ])
    }

    func appointAdmin(id: String) {
        dispatch([
            "type": "appoint_admin",
            "app_key_pubkey": id,
        ])
    }

    func demoteAdmin(id: String) {
        dispatch([
            "type": "demote_admin",
            "app_key_pubkey": id,
        ])
    }

    func renameDevice(id: String, label: String) {
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !label.isEmpty else { return }
        dispatch([
            "type": "rename_device",
            "app_key_pubkey": id,
            "label": label,
        ])
    }

    func logout() {
        let before = configIdentitySnapshot()
        cancelBackgroundSync()
        appleCalendarSync.setEnabled(false)
        refreshAppleCalendarSyncState()
        clearLocalProfileUiForLogout()
        Task { @MainActor [weak self] in
            guard let self else { return }
            await self.dispatchInBackground(["type": "logout"], invalidatePendingState: true)
            self.recordConfigMutation(action: "logout", before: before)
        }
    }

    private func clearLocalProfileUiForLogout() {
        stateGeneration &+= 1
        lastState = nil
        stateLoaded = true
        restoreSecret = ""
        approveDeviceKey = ""
        profileUsername = ""
        profilePhotoName = ""
        fileProviderError = ""
        fileProviderStatus = "Files provider not registered"
        rebuildDerivedState()
        removeFileProviderDomain()
        stopForegroundSyncLoop()
        persistLocalSettings()
    }

    func revokeDevice(label: String) {
        if let device = devices.first(where: { $0.label == label }) {
            deleteDevice(id: device.id)
        }
    }

    func startSync() {
        guard isSetupComplete else { return }
        lastForegroundDriveSyncStartedAt = Date()
        dispatch(["type": "start_sync"])
        scheduleBackgroundSyncIfNeeded()
        reconcileForegroundWorkIfAppActive()
    }

    func stopSync() {
        dispatch(["type": "stop_sync"])
        scheduleBackgroundSyncIfNeeded()
        reconcileForegroundWorkIfAppActive()
    }

    func restartSync() {
        guard isSetupComplete else { return }
        lastForegroundDriveSyncStartedAt = Date()
        dispatch(["type": "restart_sync"])
        scheduleBackgroundSyncIfNeeded()
        reconcileForegroundWorkIfAppActive()
    }

    func setAppleCalendarSyncEnabled(_ enabled: Bool) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            if !enabled {
                self.appleCalendarSync.setEnabled(false)
                self.refreshAppleCalendarSyncState()
                self.scheduleBackgroundSyncIfNeeded()
                self.reconcileForegroundWorkIfAppActive()
                return
            }

            do {
                let granted = try await self.appleCalendarSync.requestFullAccess()
                guard granted else {
                    self.appleCalendarSync.setEnabled(false)
                    self.refreshAppleCalendarSyncState()
                    self.appleCalendarSyncStatus = "Calendar access denied"
                    self.scheduleBackgroundSyncIfNeeded()
                    self.reconcileForegroundWorkIfAppActive()
                    return
                }
                self.appleCalendarSync.setEnabled(true)
                self.refreshAppleCalendarSyncState()
                self.scheduleBackgroundSyncIfNeeded()
                self.reconcileForegroundWorkIfAppActive()
                await self.runAppleCalendarSyncIfEnabled(force: true)
            } catch {
                self.appleCalendarSync.setEnabled(false)
                self.refreshAppleCalendarSyncState()
                self.appleCalendarSyncStatus = error.localizedDescription.isEmpty
                    ? "Apple Calendar access failed"
                    : error.localizedDescription
                self.scheduleBackgroundSyncIfNeeded()
                self.reconcileForegroundWorkIfAppActive()
            }
        }
    }

    private func refreshAppleCalendarSyncState() {
        appleCalendarSyncEnabled = appleCalendarSync.isActive
        appleCalendarSyncStatus = appleCalendarSync.accessStatusLabel
    }

    private func maybeRunAppleCalendarSync(force: Bool = false) {
        Task { @MainActor [weak self] in
            await self?.runAppleCalendarSyncIfEnabled(force: force)
        }
    }

    private func runAppleCalendarSyncIfEnabled(force: Bool = false) async {
        refreshAppleCalendarSyncState()
        guard shouldRunAppleCalendarSync else { return }
        guard !appleCalendarSyncInFlight else { return }
        if !force,
           Date().timeIntervalSince(lastAppleCalendarSyncCheck) < appleCalendarSyncCheckInterval {
            return
        }
        appleCalendarSyncInFlight = true
        lastAppleCalendarSyncCheck = Date()
        let result = appleCalendarSync.syncIfEnabled(
            dataDir: sharedContainerPath,
            isSetupComplete: isSetupComplete,
            force: force
        )
        appleCalendarSyncInFlight = false
        applyAppleCalendarSyncResult(result)
        scheduleBackgroundSyncIfNeeded()
    }

    private func applyAppleCalendarSyncResult(_ result: AppleCalendarSyncResult) {
        if !result.error.isEmpty {
            appleCalendarSyncStatus = result.error
            return
        }
        if result.synced, result.eventsDeleted > 0 {
            appleCalendarSyncStatus = "\(result.eventsSynced) events synced, \(result.eventsDeleted) removed"
        } else if result.synced {
            appleCalendarSyncStatus = "\(result.eventsSynced) events synced"
        } else if result.unchanged {
            appleCalendarSyncStatus = "Up to date"
        } else {
            refreshAppleCalendarSyncState()
        }
    }

    func qrMatrix(for value: String) -> QrMatrix {
        runNative { $0.qrMatrix(text: value) }
    }

    func qrMatrixInBackground(for value: String) async -> QrMatrix {
        let nativeCore = nativeCore
        let nativeCoreQueue = nativeCoreQueue
        return await withCheckedContinuation { continuation in
            nativeCoreQueue.async {
                continuation.resume(returning: nativeCore.qrMatrix(text: value))
            }
        }
    }

    func openSnapshotLink() {
        openIrisLink(snapshotLink)
    }

    func openIrisApps() {
        openIrisBrowserWhenReady(sitesPortalUrl)
    }

    func openIrisLink(_ value: String) {
        let candidate = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !candidate.isEmpty else { return }
        let linkInput = IrisDriveNativeLinkInput.classify(candidate)
        switch linkInput.kind {
        case "iris_web", "nhash_file", "mutable_file":
            if linkInput.isValid {
                openIrisBrowserWhenReady(linkInput.localOpenUrl)
            } else {
                statusTitle = "Iris link failed"
                statusDetail = linkInput.error.isEmpty ? candidate : linkInput.error
            }
        default:
            guard let url = URL(string: candidate) else { return }
            UIApplication.shared.open(url)
        }
    }

    func openIrisBrowser(_ value: String) {
        let candidate = localGatewayURL(value).trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: candidate) else { return }
        webRoute = IrisWebRoute(url: url)
    }

    func browserAddressURL(_ value: String) -> String {
        var candidate = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !candidate.isEmpty else { return value }
        if URLComponents(string: candidate)?.scheme == nil,
           candidate.contains(".") {
            candidate = "https://\(candidate)"
        }
        let linkInput = IrisDriveNativeLinkInput.classify(candidate)
        if ["iris_web", "nhash_file", "mutable_file"].contains(linkInput.kind),
           linkInput.isValid,
           !linkInput.localOpenUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return localGatewayURL(linkInput.localOpenUrl)
        }
        return localGatewayURL(candidate)
    }

    func irisWebPublisherDisplayName(forNpub npub: String) -> String? {
        let normalized = npub.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return nil }
        if normalized == currentAppKeyNpub.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            let username = profileUsername.trimmingCharacters(in: .whitespacesAndNewlines)
            if !username.isEmpty {
                return username
            }
        }
        if let name = cachedIrisWebPublisherProfileName(for: normalized) {
            return name
        }
        for share in shares {
            if let member = share.members.first(where: {
                $0.representativeNpubHint.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == normalized
            }) {
                return member.displayName
            }
            if let invite = share.pendingInvites.first(where: {
                $0.representativeNpubHint.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == normalized
            }) {
                return invite.displayName
            }
        }
        return nil
    }

    func refreshIrisWebPublisherDisplayName(for url: URL?) {
        guard let npub = irisWebPublisherNpub(from: url) else { return }
        refreshIrisWebPublisherDisplayName(forNpub: npub)
    }

    private func refreshIrisWebPublisherDisplayName(forNpub npub: String) {
        let normalized = npub.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return }
        if let entry = irisWebPublisherProfileNameCache[normalized],
           Date().timeIntervalSince1970 - entry.fetchedAt < irisWebPublisherProfileNameFreshAge {
            return
        }
        if let miss = irisWebPublisherProfileNameMisses[normalized],
           Date().timeIntervalSince(miss) < irisWebPublisherProfileNameMissCooldown {
            return
        }
        guard irisWebPublisherProfileNameTasks[normalized] == nil,
              let baseURL = irisNativeHashtreeBaseURL()
        else {
            return
        }

        let profileURL = baseURL
            .appendingPathComponent("api")
            .appendingPathComponent("nostr")
            .appendingPathComponent("profile")
            .appendingPathComponent(normalized)
        irisWebPublisherProfileNameTasks[normalized] = Task { [weak self] in
            let name = await Self.fetchIrisWebPublisherProfileName(from: profileURL)
            await MainActor.run {
                guard let self else { return }
                self.irisWebPublisherProfileNameTasks[normalized] = nil
                if let name {
                    self.irisWebPublisherProfileNameMisses[normalized] = nil
                    self.irisWebPublisherProfileNameCache[normalized] =
                        IrisWebPublisherProfileNameCacheEntry(
                            name: name,
                            fetchedAt: Date().timeIntervalSince1970
                        )
                    self.persistIrisWebPublisherProfileNameCache()
                } else {
                    self.irisWebPublisherProfileNameMisses[normalized] = Date()
                }
            }
        }
    }

    private func cachedIrisWebPublisherProfileName(for normalizedNpub: String) -> String? {
        guard let entry = irisWebPublisherProfileNameCache[normalizedNpub],
              Date().timeIntervalSince1970 - entry.fetchedAt < irisWebPublisherProfileNameMaxCacheAge
        else {
            return nil
        }
        return Self.normalizedIrisWebProfileName(entry.name)
    }

    private func loadIrisWebPublisherProfileNameCache() -> [String: IrisWebPublisherProfileNameCacheEntry] {
        guard let data = defaults.data(forKey: irisWebPublisherProfileNameCacheKey),
              let decoded = try? JSONDecoder().decode(
                [String: IrisWebPublisherProfileNameCacheEntry].self,
                from: data
              )
        else {
            return [:]
        }
        let now = Date().timeIntervalSince1970
        return decoded.filter { key, entry in
            key.starts(with: "npub1")
                && !entry.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && now - entry.fetchedAt < irisWebPublisherProfileNameMaxCacheAge
        }
    }

    private func persistIrisWebPublisherProfileNameCache() {
        let now = Date().timeIntervalSince1970
        let fresh = irisWebPublisherProfileNameCache.filter { key, entry in
            key.starts(with: "npub1")
                && !entry.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && now - entry.fetchedAt < irisWebPublisherProfileNameMaxCacheAge
        }
        irisWebPublisherProfileNameCache = fresh
        guard let data = try? JSONEncoder().encode(fresh) else { return }
        defaults.set(data, forKey: irisWebPublisherProfileNameCacheKey)
    }

    private static func fetchIrisWebPublisherProfileName(from url: URL) async -> String? {
        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = irisWebPublisherProfileFetchTimeout
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse,
                  http.statusCode == 200,
                  let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let profile = object["profile"] as? [String: Any]
            else {
                return nil
            }
            for key in ["display_name", "displayName", "name", "username"] {
                if let name = normalizedIrisWebProfileName(profile[key]) {
                    return name
                }
            }
            return nil
        } catch {
            return nil
        }
    }

    private static func normalizedIrisWebProfileName(_ value: Any?) -> String? {
        guard let raw = value as? String else { return nil }
        let normalized = raw
            .split(whereSeparator: { $0.isWhitespace || $0.isNewline })
            .joined(separator: " ")
        guard !normalized.isEmpty else { return nil }
        return String(normalized.prefix(80))
    }

    func localGatewayURL(_ value: String) -> String {
        localGatewayURL(value, activePortalUrl: sitesPortalUrl)
    }

    func addRelay(_ value: String? = nil) {
        let candidate = (value ?? relayInput).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !candidate.isEmpty else { return }
        dispatch([
            "type": "add_relay",
            "url": candidate,
        ])
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

    func addBlossomServer() {
        let url = blossomEndpointInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty else { return }
        dispatch([
            "type": "add_blossom_server",
            "url": url,
        ])
        blossomEndpointInput = ""
    }

    func removeBlossomServer(_ url: String) {
        dispatch([
            "type": "remove_blossom_server",
            "url": url,
        ])
    }

    func createShare() {
        let sourcePath = shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sourcePath.isEmpty else { return }
        let resolved = resolveShareSourcePathForCreate(sourcePath)
        guard resolved.error.isEmpty else {
            statusTitle = "Share folder failed"
            statusDetail = resolved.error
            return
        }
        dispatch([
            "type": "create_share",
            "source_path": resolved.path,
            "display_name": "",
        ])
        shareSourceInput = ""
    }

    func openShareDialog(
        sourcePath: String,
        displayName: String,
        recipientNpubHint: String = "",
        recipientDisplayName: String = "",
        recipientProfileId: String = ""
    ) {
        let sourcePath = sourcePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sourcePath.isEmpty else { return }
        shareSourceInput = sourcePath
        shareRecipientNpubHint = recipientNpubHint.trimmingCharacters(in: .whitespacesAndNewlines)
        shareRecipientDisplayName = recipientDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        shareRecipientProfileId = recipientProfileId.trimmingCharacters(in: .whitespacesAndNewlines)
        shareDialogRequestId &+= 1
    }

    func acceptShareInvite() {
        let invite = shareInviteInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !invite.isEmpty else { return }
        dispatch([
            "type": "accept_share_invite",
            "invite": invite,
        ])
        shareInviteInput = ""
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
        let trimmedProfile = profileId.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedAppKey = appKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedProfile.isEmpty, !trimmedAppKey.isEmpty else { return }
        dispatch([
            "type": "invite_share_member",
            "share_id": shareId,
            "profile_id": trimmedProfile,
            "app_key": trimmedAppKey,
            "role": role,
            "representative_npub_hint": representativeNpubHint,
            "display_name": displayName,
            "label": label,
        ])
        copyLastShareInvite()
    }

    func inviteShareMemberFromEvidence(
        shareId: String,
        evidenceJson: String,
        role: String,
        displayName: String
    ) {
        let evidenceJson = evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !evidenceJson.isEmpty else { return }
        dispatch([
            "type": "invite_share_member_from_evidence",
            "share_id": shareId,
            "evidence_json": evidenceJson,
            "role": role,
            "display_name": displayName,
        ])
        copyLastShareInvite()
    }

    func exportShareRecipientEvidence(displayName: String) {
        dispatch([
            "type": "export_share_recipient_evidence",
            "display_name": displayName,
        ])
    }

    func recordPendingShareInvite(
        shareId: String,
        representativeNpubHint: String,
        role: String,
        displayName: String
    ) {
        let representativeNpubHint = representativeNpubHint.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !representativeNpubHint.isEmpty else { return }
        dispatch([
            "type": "record_pending_share_invite",
            "share_id": shareId,
            "representative_npub_hint": representativeNpubHint,
            "role": role,
            "display_name": displayName,
        ])
    }

    func revokeShareMember(shareId: String, profileId: String) {
        dispatch([
            "type": "revoke_share_member",
            "share_id": shareId,
            "profile_id": profileId,
            "reason": "",
        ])
    }

    func deleteShare(shareId: String) {
        dispatch([
            "type": "delete_share",
            "share_id": shareId,
        ])
    }

    func openShareFolder(_ share: IrisDriveShare) {
        let path = shareOpenPath(share)
        guard !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        openDriveFolder(path: path)
    }

    func addShareShortcut(shareId: String, displayName _: String) {
        dispatch([
            "type": "add_share_shortcut",
            "share_id": shareId,
            "path": "",
            "parent": "",
            "target_path": "",
        ])
    }

    func repairShareWraps(shareId: String) {
        dispatch([
            "type": "repair_share_wraps",
            "share_id": shareId,
        ])
    }

    private func resolveShareSourcePathForCreate(_ input: String) -> (path: String, error: String) {
        let normalized = IrisDriveNativeProvider.normalizePath(path: input)
        if !normalized.error.isEmpty {
            return ("", normalized.error)
        }
        let sourcePath = normalized.path
        guard !sourcePath.isEmpty else {
            return ("", "Share folder path required")
        }
        if let kind = providerEntryKind(path: sourcePath) {
            return kind == "dir" ? (sourcePath, "") : ("", "Share path must be a folder")
        }
        let createdPath = defaultCreatedShareSourcePath(sourcePath)
        if let kind = providerEntryKind(path: createdPath) {
            return kind == "dir" ? (createdPath, "") : ("", "Share path must be a folder")
        }
        let mkdirJson = nativeJsonObject(IrisDriveNativeProvider.mkdir(dataDir: sharedContainerPath, path: createdPath))
        let mkdirError = mkdirJson["error"] as? String ?? ""
        if !mkdirError.isEmpty {
            return ("", mkdirError)
        }
        let created = (mkdirJson["path"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return (created.isEmpty ? createdPath : created, "")
    }

    private func providerEntryKind(path: String) -> String? {
        let object = nativeJsonObject(IrisDriveNativeProvider.list(dataDir: sharedContainerPath))
        guard let entries = object["entries"] as? [[String: Any]] else { return nil }
        return entries.first { entry in
            entry["path"] as? String == path
        }?["kind"] as? String
    }

    private func defaultCreatedShareSourcePath(_ sourcePath: String) -> String {
        if sourcePath == "Shared" || sourcePath.hasPrefix("Shared/") {
            return sourcePath
        }
        return "Shared/\(sourcePath)"
    }

    private func shareOpenPath(_ share: IrisDriveShare) -> String {
        if let shortcut = share.shortcutPaths.first,
           !shortcut.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return shortcut
        }
        if !share.sourcePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return share.sourcePath
        }
        return share.sharedWithMePath
    }

    private func nativeJsonObject(_ text: String) -> [String: Any] {
        guard let data = text.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return [:]
        }
        return object
    }

    func resetLocalState() {
        let before = configIdentitySnapshot()
        cancelBackgroundSync()
        try? FileManager.default.removeItem(at: IrisDriveSharedContainer.baseDirectory)
        removeFileProviderDomain()
        providerSignalRetryTask?.cancel()
        providerSignalRetryTask = nil
        providerSignalInFlightKey = ""
        currentProviderSignalKey = ""
        lastProviderSignalKey = ""
        lastState = nil
        stateLoaded = false
        restoreSecret = ""
        syncRunning = false
        statusTitle = "Ready"
        statusDetail = "Waiting for this device to be linked."
        profileUsername = ""
        profilePhotoName = ""
        persistLocalSettings()
        applyStateJson(runNative { $0.refreshJson() })
        recordConfigMutation(action: "reset_local_state", before: before)
    }

    func handle(url: URL) {
        let linkInput = IrisDriveNativeLinkInput.classify(url.absoluteString)
        if linkInput.kind == "share_dialog" {
            openShareDialog(
                sourcePath: linkInput.shareSourcePath,
                displayName: linkInput.shareDisplayName,
                recipientNpubHint: linkInput.shareRecipientNpubHint,
                recipientDisplayName: linkInput.shareRecipientDisplayName,
                recipientProfileId: linkInput.shareRecipientProfileId
            )
            return
        }
        if linkInput.kind == "nhash_file" || linkInput.kind == "mutable_file" {
            openContentLink(linkInput)
            return
        }
        if linkInput.kind == "iris_web" {
            openIrisBrowserWhenReady(linkInput.localOpenUrl)
            return
        }
        if linkInput.kind == "invite" {
            profileLinkTarget = url.absoluteString
            linkDevice()
            ensureFileProviderDomainIfProfileExists()
            return
        }
        guard linkInput.kind == "app_key_approval" else {
            statusTitle = "Iris link opened"
            statusDetail = url.absoluteString
            persist()
            load()
            return
        }

        if canAdminProfile, linkInput.isComplete {
            approveDevice(request: url.absoluteString, label: "")
            return
        }

        statusTitle = canAdminProfile ? "Invalid device invite" : "Open on a profile admin"
        statusDetail = canAdminProfile
            ? (linkInput.error.isEmpty ? url.absoluteString : linkInput.error)
            : "Open this request on a profile admin device, or scan an invite link to join."
    }

    func handleDebugLaunchEnvironment() {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        switch environment["IRIS_DRIVE_DEBUG_ACTION"] {
        case "reset-local-state":
            guard allowDebugStateMutation(action: "reset-local-state", environment: environment) else {
                return
            }
            resetLocalState()
        case "link-device":
            guard allowDebugStateMutation(action: "link-device", environment: environment) else {
                return
            }
            guard let target = environment["IRIS_DRIVE_DEBUG_OWNER"],
                  !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else {
                return
            }
            profileLinkTarget = target
            linkDevice()
        case "create-profile":
            guard allowDebugStateMutation(action: "create-profile", environment: environment) else {
                return
            }
            guard !hasLocalProfile else { return }
            createProfile(
                username: environment["IRIS_DRIVE_DEBUG_USERNAME"] ?? "",
                profilePhotoName: ""
            )
        case "seed-provider-file":
            guard allowDebugStateMutation(action: "seed-provider-file", environment: environment) else {
                return
            }
            seedDebugProviderFile(resetFirst: false, environment: environment)
        case "reset-and-seed-provider-file":
            guard allowDebugStateMutation(action: "reset-and-seed-provider-file", environment: environment) else {
                return
            }
            seedDebugProviderFile(resetFirst: true, environment: environment)
        case "probe-iris-apps":
            debugProbeIrisApps()
        case "open-browser":
            guard let target = environment["IRIS_DRIVE_DEBUG_BROWSER_URL"],
                  !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else {
                return
            }
            openIrisBrowser(target)
        case "open-browser-ready":
            guard let target = environment["IRIS_DRIVE_DEBUG_BROWSER_URL"],
                  !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else {
                return
            }
            openIrisBrowserWhenReady(target)
        default:
            return
        }
        #endif
    }

    private func load() {
        removeObsoletePrototypeDefaults()
        irisWebPublisherProfileNameCache = loadIrisWebPublisherProfileNameCache()
        applyStateJson(runNative { $0.stateJson() })
        deviceLabel = defaults.string(forKey: "deviceLabel") ?? UIDevice.current.name
        if hasLocalProfile {
            profileUsername = defaults.string(forKey: "profileUsername") ?? profileUsername
            profilePhotoName = defaults.string(forKey: "profilePhotoName") ?? profilePhotoName
        } else {
            clearStaleProfileSettings()
        }
        relayInput = ""
        syncOverCellular = defaults.bool(forKey: "syncOverCellular")
        refreshAppleCalendarSyncState()
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
            "canAdminProfile",
            "hasOwnerAuthority",
            "ownerPublicKey",
            "profileLinkTarget",
            "relay",
            "relays",
            "statusDetail",
            "statusTitle",
            "syncRunning",
        ].forEach(defaults.removeObject)
    }

    private func clearStaleProfileSettings() {
        profileUsername = ""
        profilePhotoName = ""
        defaults.removeObject(forKey: "profileUsername")
        defaults.removeObject(forKey: "profilePhotoName")
    }

    private func rebuildDerivedState() {
        guard let state = lastState else {
            profileLinkTarget = ""
            currentAppKeyNpub = ""
            devicePublicKey = "local-device"
            authorizationState = "Not linked"
            devices = []
            inboundAppKeyLinkRequests = []
            roots = []
            backups = []
            shares = []
            relays = defaultRelays
            relayStatuses = []
            relay = defaultRelay
            syncRunning = false
            statusTitle = "Ready"
            statusDetail = "Waiting for this device to be linked."
            currentProviderSignalKey = ""
            lastProviderSignalKey = ""
            providerSignalInFlightKey = ""
            providerSignalRetryTask?.cancel()
            providerSignalRetryTask = nil
            currentProviderDirectoryPaths = []
            onlineDeviceCount = 0
            localNhashResolverEnabled = true
            sitesPortalUrl = ""
            return
        }

        currentAppKeyNpub = state.ui.profile?.currentAppKeyNpub ?? ""
        devicePublicKey = state.ui.profile?.devicePubkey ?? "local-device"
        deviceLabel = state.ui.profile?.appKeyLabel.isEmpty == false
            ? state.ui.profile?.appKeyLabel ?? deviceLabel
            : deviceLabel
        syncRunning = state.ui.sync.running
        authorizationState = state.ui.setupLabel
        statusTitle = state.ui.primaryStatusLabel
        statusDetail = state.error.isEmpty ? state.ui.sync.statusLabel : state.error
        if !fileProviderError.isEmpty {
            statusTitle = "Open in Files failed"
            statusDetail = fileProviderError
        }
        relays = state.ui.relays.isEmpty ? defaultRelays : state.ui.relays
        relayStatuses = state.ui.relayStatuses.map(IrisDriveRelayStatus.init)
        relay = relays.first ?? defaultRelay
        devices = state.ui.devices.map { device in
            return IrisDriveDevice(
                label: device.displayLabel,
                actorKind: device.actorKind
                    ?? (device.role == "recovery" || device.connectionState == "recovery"
                        ? "recovery_key"
                        : "device"),
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
        inboundAppKeyLinkRequests = state.ui.profile?.inboundAppKeyLinkRequests.map { request in
            IrisDriveAppKeyLinkRequest(
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
        localNhashResolverEnabled = state.ui.localNhashResolverEnabled
        sitesPortalUrl = state.ui.sitesPortalUrl
        currentProviderSignalKey = state.ui.providerChangeKey
        currentProviderDirectoryPaths = state.ui.providerDirectoryPaths
        signalFileProviderIfNeeded()
        backups = state.ui.backups.map { backup in
            IrisDriveBackup(
                id: backup.id,
                kind: backup.kind,
                target: backup.target,
                label: backup.label,
                configuredLabel: backup.configuredLabel,
                state: backup.state,
                detail: backup.detail,
                enabled: backup.enabled
            )
        }
        shares = state.ui.shares.map { share in
            IrisDriveShare(
                shareId: share.shareId,
                displayName: share.displayName,
                sourcePath: share.sourcePath,
                sharedWithMePath: share.sharedWithMePath,
                role: share.role,
                roleLabel: share.roleLabel,
                keyStatus: share.keyStatus,
                keyStatusLabel: share.keyStatusLabel,
                writeAuthorization: share.writeAuthorization,
                writeAuthorizationLabel: share.writeAuthorizationLabel,
                canWrite: share.canWrite,
                canAdmin: share.canAdmin,
                currentKeyEpoch: share.currentKeyEpoch,
                hasCurrentKeyWrap: share.hasCurrentKeyWrap,
                keyUnavailable: share.keyUnavailable,
                repairNeeded: share.repairNeeded,
                missingKeyWraps: share.missingKeyWraps,
                participantCount: share.participantCount,
                appKeyCount: share.appKeyCount,
                members: share.members.map { member in
                    IrisDriveShareMember(
                        profileId: member.profileId,
                        displayName: member.displayName,
                        representativeNpubHint: member.representativeNpubHint,
                        role: member.role,
                        roleLabel: member.roleLabel,
                        status: member.status,
                        statusLabel: member.statusLabel,
                        appKeyCount: member.appKeyCount
                    )
                },
                pendingInvites: share.pendingInvites.map { invite in
                    IrisDrivePendingShareInvite(
                        representativeNpubHint: invite.representativeNpubHint,
                        displayName: invite.displayName,
                        role: invite.role,
                        roleLabel: invite.roleLabel,
                        status: invite.status,
                        statusLabel: invite.statusLabel
                    )
                },
                shortcutPaths: share.shortcutPaths
            )
        }
        roots = state.ui.roots.map { root in
            IrisDriveRoot(name: root.name, status: root.status, path: root.localPath)
        }
    }

    @discardableResult
    func dispatch(_ action: [String: Any]) -> NativeAppState? {
        guard let actionJson = encodeNativeAction(action) else {
            statusTitle = "Native action failed"
            statusDetail = "Unable to encode action."
            return nil
        }
        stateGeneration &+= 1
        applyStateJson(runNative { $0.dispatchJson(actionJson) })
        return lastState
    }

    func dispatchInBackground(
        _ action: [String: Any],
        invalidatePendingState: Bool = false
    ) async {
        guard let actionJson = encodeNativeAction(action) else {
            statusTitle = "Native action failed"
            statusDetail = "Unable to encode action."
            return
        }
        if invalidatePendingState {
            stateGeneration &+= 1
        }
        let generation = stateGeneration
        let json = await runNativeInBackground { nativeCore in
            nativeCore.dispatchJson(actionJson)
        }
        guard !Task.isCancelled, generation == stateGeneration else { return }
        applyStateJson(json)
    }

    func refreshInBackground() async {
        let generation = stateGeneration
        #if DEBUG
        if let delay = debugRefreshDelayNanoseconds() {
            do {
                try await Task.sleep(nanoseconds: delay)
            } catch {
                return
            }
        }
        #endif
        let json = await runNativeInBackground { nativeCore in
            nativeCore.refreshJson()
        }
        guard !Task.isCancelled, generation == stateGeneration else { return }
        applyStateJson(json)
        ensureFileProviderDomainIfProfileExists()
        await runAppleCalendarSyncIfEnabled()
    }

    private func runNativeInBackground(
        _ operation: @escaping @Sendable (IrisDriveNativeCore) -> String
    ) async -> String {
        let nativeCore = nativeCore
        let nativeCoreQueue = nativeCoreQueue
        return await withCheckedContinuation { continuation in
            nativeCoreQueue.async { continuation.resume(returning: operation(nativeCore)) }
        }
    }

    private func runNative<T>(_ operation: (IrisDriveNativeCore) -> T) -> T {
        nativeCoreQueue.sync { operation(nativeCore) }
    }

    #if DEBUG
    private func debugRefreshDelayNanoseconds() -> UInt64? {
        let value = ProcessInfo.processInfo.environment["IRIS_DRIVE_DEBUG_REFRESH_DELAY_MS"]?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard let milliseconds = UInt64(value), milliseconds > 0 else { return nil }
        return milliseconds * 1_000_000
    }
    #endif

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
            lastState = nil
            syncRunning = false
            stateLoaded = true
            statusTitle = "Native state failed"
            statusDetail = json
            writeDebugState(json)
            reconcileForegroundWorkIfAppActive()
            return
        }
        if json == lastAppliedStateJson && stateLoaded {
            return
        }
        lastAppliedStateJson = json
        stateLoaded = true
        lastState = state
        rebuildDerivedState()
        writeDebugState(json)
        reconcileForegroundWorkIfAppActive()
    }

    private func reconcileForegroundWorkIfAppActive() {
        guard UIApplication.shared.applicationState == .active else { return }
        startForegroundSyncLoop()
    }

    private func writeDebugState(_ json: String) {
        #if DEBUG
        writeDebugState(
            json,
            to: IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent(iosDebugStateFileName, isDirectory: false)
        )
        if let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
            writeDebugState(
                json,
                to: documents.appendingPathComponent(iosDebugStateFileName, isDirectory: false)
            )
        }
        #endif
    }

    private func writeDebugState(_ json: String, to url: URL) {
        #if DEBUG
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let debugJson = jsonWithConfigMutationAudit(json)
        try? debugJson.write(to: url, atomically: true, encoding: .utf8)
        #endif
    }

    private func configIdentitySnapshot() -> ConfigIdentitySnapshot {
        let profile = lastState?.ui.profile
        return ConfigIdentitySnapshot(
            hasProfile: profile != nil,
            setupState: lastState?.ui.setupState ?? "",
            profileId: profile?.profileId ?? "",
            currentAppKeyNpub: profile?.currentAppKeyNpub ?? "",
            currentAppKeyLabel: profile?.appKeyLabel ?? ""
        )
    }

    private func recordConfigMutation(action: String, before: ConfigIdentitySnapshot) {
        let event = ConfigMutationAuditEvent(
            timestamp: ISO8601DateFormatter().string(from: Date()),
            action: action,
            debugAction: ProcessInfo.processInfo.environment["IRIS_DRIVE_DEBUG_ACTION"] ?? "",
            before: before,
            after: configIdentitySnapshot(),
            error: lastState?.error ?? ""
        )
        var events = configMutationAuditEvents()
        events.append(event)
        if events.count > configMutationAuditMaxEvents {
            events.removeFirst(events.count - configMutationAuditMaxEvents)
        }
        guard let data = try? JSONEncoder().encode(events) else { return }
        defaults.set(data, forKey: configMutationAuditDefaultsKey)
        writeDebugState(runNative { $0.stateJson() })
    }

    private func configMutationAuditEvents() -> [ConfigMutationAuditEvent] {
        guard let data = defaults.data(forKey: configMutationAuditDefaultsKey),
              let events = try? JSONDecoder().decode([ConfigMutationAuditEvent].self, from: data)
        else {
            return []
        }
        return events
    }

    private func jsonWithConfigMutationAudit(_ json: String) -> String {
        guard let data = json.data(using: .utf8),
              var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let auditData = try? JSONEncoder().encode(configMutationAuditEvents()),
              let audit = try? JSONSerialization.jsonObject(with: auditData)
        else {
            return json
        }
        object["ios_config_mutation_audit"] = audit
        guard let output = try? JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys]
        ) else {
            return json
        }
        return String(data: output, encoding: .utf8) ?? json
    }

    private func signalFileProviderIfNeeded() {
        guard isSetupComplete, !currentProviderSignalKey.isEmpty else { return }
        guard currentProviderSignalKey != lastProviderSignalKey else { return }
        guard currentProviderSignalKey != providerSignalInFlightKey else { return }
        let signalKey = currentProviderSignalKey
        let domain = irisDriveFileProviderDomain()
        guard let manager = NSFileProviderManager(for: domain) else {
            scheduleFileProviderSignalRetry(for: signalKey)
            return
        }
        providerSignalInFlightKey = signalKey
        providerSignalRetryTask?.cancel()
        providerSignalRetryTask = nil
        var identifiers: [NSFileProviderItemIdentifier] = [.rootContainer, .workingSet]
        identifiers.append(contentsOf: currentProviderDirectoryPaths.map(fileProviderIdentifier))
        let group = DispatchGroup()
        var failed = false
        if #available(iOS 16.0, *) {
            group.enter()
            manager.reimportItems(below: .rootContainer) { error in
                if let error {
                    failed = true
                    NSLog("Iris Drive Files provider reimport failed: \(error)")
                }
                group.leave()
            }
        }
        for identifier in identifiers {
            group.enter()
            manager.signalEnumerator(for: identifier) { error in
                if let error {
                    failed = true
                    NSLog("Iris Drive Files provider signal failed for \(identifier.rawValue): \(error)")
                }
                group.leave()
            }
        }
        group.notify(queue: .main) { [weak self] in
            guard let self, self.providerSignalInFlightKey == signalKey else { return }
            self.providerSignalInFlightKey = ""
            if failed {
                self.scheduleFileProviderSignalRetry(for: signalKey)
                return
            }
            if self.currentProviderSignalKey == signalKey {
                self.lastProviderSignalKey = signalKey
            }
        }
    }

    private func scheduleFileProviderSignalRetry(for signalKey: String) {
        providerSignalRetryTask?.cancel()
        providerSignalRetryTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard let self,
                  self.currentProviderSignalKey == signalKey,
                  self.lastProviderSignalKey != signalKey
            else {
                return
            }
            self.signalFileProviderIfNeeded()
        }
    }

    private func fileProviderIdentifier(for path: String) -> NSFileProviderItemIdentifier {
        guard !path.isEmpty else { return .rootContainer }
        return NSFileProviderItemIdentifier(
            "\(fileProviderPathIdentifierPrefix)\(fileProviderEncodedPath(path))"
        )
    }

    private func fileProviderEncodedPath(_ path: String) -> String {
        Data(path.utf8)
            .base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private func deviceKey(from request: String) -> String {
        let linkInput = IrisDriveNativeLinkInput.classify(request)
        guard linkInput.kind == "app_key_approval", !linkInput.appKeyPubkey.isEmpty else {
            return request
        }
        return linkInput.appKeyPubkey
    }

    func allowDebugStateMutation(action: String, environment: [String: String]) -> Bool {
        #if DEBUG
        #if targetEnvironment(simulator)
        return true
        #else
        if environment["IRIS_DRIVE_ALLOW_DESTRUCTIVE_DEBUG_ACTIONS_ON_DEVICE"] == "1" {
            return true
        }
        if let baseDir = environment["IRIS_DRIVE_UI_TEST_BASE_DIR"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           baseDir == "__TMP__" || baseDir.hasPrefix("__TMP__/") {
            return true
        }
        statusTitle = "Debug action blocked"
        statusDetail = "Refused \(action) on a physical device."
        writeDebugState(runNative { $0.refreshJson() })
        return false
        #endif
        #else
        return false
        #endif
    }

    #if DEBUG
    private func seedDebugProviderFile(resetFirst: Bool, environment: [String: String]) {
        if resetFirst {
            resetLocalState()
        }
        if !hasLocalProfile {
            createProfile(
                username: environment["IRIS_DRIVE_DEBUG_USERNAME"] ?? "",
                profilePhotoName: ""
            )
        }
        let displayName = environment["IRIS_DRIVE_DEBUG_PROVIDER_FILE_NAME"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            ?? "Iris Drive UI provider entry.txt"
        let contents = environment["IRIS_DRIVE_DEBUG_PROVIDER_FILE_CONTENT"] ?? "Iris Drive UI provider entry\n"
        let source = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-provider-\(UUID().uuidString)", isDirectory: false)
        do {
            try contents.write(to: source, atomically: true, encoding: .utf8)
            _ = dispatch([
                "type": "import_file",
                "display_name": displayName,
                "source_path": source.path,
            ])
            ensureFileProviderDomainIfProfileExists()
        } catch {
            statusTitle = "Debug seed failed"
            statusDetail = "\(error)"
        }
    }
    #endif
}
