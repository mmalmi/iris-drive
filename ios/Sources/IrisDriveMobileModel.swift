import BackgroundTasks
import FileProvider
import Foundation
import SwiftUI
import UIKit

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveFileProviderDisplayName = "Iris Drive"
private let defaultRelay = "wss://relay.damus.io"
private let defaultRelays = [defaultRelay]
private let defaultBlossomServers = ["https://upload.iris.to"]
private let iosDebugStateFileName = "debug-state.json"
private let fileProviderPathIdentifierPrefix = "path:"
private let fileProviderRegistrationIdentityKey = "fileProviderRegistrationIdentity"
private let foregroundSyncIntervalNanoseconds: UInt64 = 5_000_000_000
private let nativeBackgroundStackSize = 8 * 1024 * 1024
#if DEBUG
private let fileProviderDebugRegistrationVersion = 2
private let fileProviderDebugRegistrationVersionKey = "fileProviderDebugRegistrationVersion"
#endif

enum IrisDriveBackgroundSyncTask { static let identifier = "to.iris.drive.ios.background-sync" }

@MainActor
final class IrisDriveMobileModel: ObservableObject {
    @Published var driveName = "My Drive"
    @Published var statusTitle = "Ready"
    @Published var statusDetail = "Waiting for this app install to be linked."
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
    @Published var backupLabelInput = ""
    @Published var blossomEndpointInput = ""
    @Published var shareSourceInput = ""
    @Published var shareNameInput = ""
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
    @Published var approveDeviceLabel = ""
    @Published var devices: [IrisDriveDevice] = []
    @Published var inboundAppKeyLinkRequests: [IrisDriveAppKeyLinkRequest] = []
    @Published var backups: [IrisDriveBackup] = []
    @Published var shares: [IrisDriveShare] = []
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
    private var fileProviderDomainRemovalInFlight = false
    private var stateGeneration: UInt64 = 0

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
        lastState?.ui.sync.statusLabel ?? "Sync paused"
    }

    var snapshotLink: String {
        lastState?.ui.snapshotLink ?? ""
    }

    var lastShareInvite: String {
        lastState?.ui.lastShareInvite ?? ""
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

    var isSetupComplete: Bool {
        lastState?.ui.setupComplete ?? false
    }

    var isAwaitingApproval: Bool {
        lastState?.ui.awaitingApproval ?? false
    }

    var isRevoked: Bool {
        lastState?.ui.revoked ?? false
    }

    private var shouldRunBackgroundSync: Bool {
        syncRunning && !isRevoked && (isSetupComplete || isAwaitingApproval)
    }

    var canAdminProfile: Bool {
        lastState?.ui.profile?.canAdminProfile ?? false
    }

    var canExportRecoveryPhrase: Bool {
        lastState?.ui.profile?.canExportRecoveryPhrase ?? false
    }

    func ensureFileProviderDomainIfProfileExists() {
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
            showFileProviderError("Link this app install before opening Iris Drive in Files.")
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
        fileProviderDomainRemovalInFlight = true
        NSFileProviderManager.remove(domain) { [weak self] _ in
            Task { @MainActor in
                self?.fileProviderDomainRemovalInFlight = false
            }
        }
        defaults.removeObject(forKey: fileProviderRegistrationIdentityKey)
        #if DEBUG
        defaults.removeObject(forKey: fileProviderDebugRegistrationVersionKey)
        #endif
    }

    private func waitForFileProviderRemovalThenEnsure(completion: ((Bool) -> Void)?) {
        Task { @MainActor [weak self] in
            for _ in 0..<30 where self?.fileProviderDomainRemovalInFlight == true {
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
            guard let self else {
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

    private func markFileProviderRegistrationCurrent() {
        let identity = fileProviderRegistrationIdentity
        if identity.isEmpty {
            defaults.removeObject(forKey: fileProviderRegistrationIdentityKey)
        } else {
            defaults.set(identity, forKey: fileProviderRegistrationIdentityKey)
        }
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
        applyStateJson(nativeCore.refreshJson())
        ensureFileProviderDomainIfProfileExists()
    }

    func scheduleBackgroundSyncIfNeeded() {
        guard shouldRunBackgroundSync else { return cancelBackgroundSync() }
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
        guard syncRunning, isSetupComplete || isAwaitingApproval else { return }
        if isSetupComplete {
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
            "app_key_label": deviceLabel,
        ])
        persistLocalSettings()
        ensureFileProviderDomainIfProfileExists()
        scheduleBackgroundSyncIfNeeded()
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
        }
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

    func rejectDevice(request: String) {
        dispatch([
            "type": "reject_device",
            "request": request,
        ])
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

    func logout() {
        cancelBackgroundSync()
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
        scheduleBackgroundSyncIfNeeded()
    }

    func stopSync() {
        dispatch(["type": "stop_sync"])
        cancelBackgroundSync()
    }

    func restartSync() {
        guard isSetupComplete else { return }
        dispatch(["type": "restart_sync"])
        scheduleBackgroundSyncIfNeeded()
    }

    func copyAppKey() {
        UIPasteboard.general.string = currentAppKeyNpub
    }

    func copyDeviceKey() {
        UIPasteboard.general.string = devicePublicKey
    }

    func copyLinkRequest() {
        UIPasteboard.general.string = appKeyLinkRequest
    }

    func copyLinkInvite() {
        UIPasteboard.general.string = appKeyLinkInvite
    }

    func qrMatrix(for value: String) -> QrMatrix {
        nativeCore.qrMatrix(text: value)
    }

    func copySnapshotLink() {
        guard !snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        UIPasteboard.general.string = snapshotLink
    }

    func copyLastShareInvite() {
        guard !lastShareInvite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        UIPasteboard.general.string = lastShareInvite
    }

    func openSnapshotLink() {
        guard let url = URL(string: snapshotLink) else { return }
        UIApplication.shared.open(url)
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

    func addBackupTarget() {
        let target = backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatch([
            "type": "add_backup_target",
            "target": target,
            "label": backupLabelInput,
        ])
        backupTargetInput = ""
        backupLabelInput = ""
    }

    func removeBackupTarget(_ target: String) {
        dispatch([
            "type": "remove_backup_target",
            "target": target,
        ])
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

    func syncBackups(_ target: String = "") {
        guard isSetupComplete else { return }
        Task {
            await dispatchInBackground([
                "type": "sync_backups",
                "target": target,
            ], invalidatePendingState: true)
        }
    }

    func checkBackups(_ target: String = "") {
        guard isSetupComplete else { return }
        Task {
            await dispatchInBackground([
                "type": "check_backups",
                "target": target,
            ], invalidatePendingState: true)
        }
    }

    func createShare() {
        let sourcePath = shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sourcePath.isEmpty else { return }
        dispatch([
            "type": "create_share",
            "source_path": sourcePath,
            "display_name": shareNameInput,
        ])
        shareSourceInput = ""
        shareNameInput = ""
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
        shareNameInput = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
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

    func revokeShareMember(shareId: String, profileId: String) {
        dispatch([
            "type": "revoke_share_member",
            "share_id": shareId,
            "profile_id": profileId,
            "reason": "",
        ])
    }

    func addShareShortcut(shareId: String, displayName: String) {
        dispatch([
            "type": "add_share_shortcut",
            "share_id": shareId,
            "path": displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? "Shared folder"
                : displayName,
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

    func resetLocalState() {
        cancelBackgroundSync()
        try? FileManager.default.removeItem(at: IrisDriveSharedContainer.baseDirectory)
        removeFileProviderDomain()
        lastState = nil
        restoreSecret = ""
        syncRunning = false
        statusTitle = "Ready"
        statusDetail = "Waiting for this app install to be linked."
        profileUsername = ""
        profilePhotoName = ""
        persistLocalSettings()
        applyStateJson(nativeCore.refreshJson())
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
            approveDevice(request: url.absoluteString, label: "Linked app install")
            return
        }

        statusTitle = canAdminProfile ? "Invalid AppKey invite" : "Open on a profile admin"
        statusDetail = canAdminProfile
            ? (linkInput.error.isEmpty ? url.absoluteString : linkInput.error)
            : "Open this request on a profile admin install, or scan an invite link to join."
    }

    func handleDebugLaunchEnvironment() {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        guard environment["IRIS_DRIVE_DEBUG_ACTION"] == "link-device",
              let target = environment["IRIS_DRIVE_DEBUG_OWNER"],
              !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return
        }
        profileLinkTarget = target
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
            "canAdminProfile",
            "profileLinkTarget",
            "relay",
            "relays",
            "statusDetail",
            "statusTitle",
            "syncRunning",
        ].forEach(defaults.removeObject)
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
            statusDetail = "Waiting for this app install to be linked."
            currentProviderSignalKey = ""
            lastProviderSignalKey = ""
            currentProviderDirectoryPaths = []
            onlineDeviceCount = 0
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
                shortcutPaths: share.shortcutPaths
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
        stateGeneration &+= 1
        applyStateJson(nativeCore.dispatchJson(actionJson))
    }

    private func dispatchInBackground(
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

    private func refreshInBackground() async {
        let generation = stateGeneration
        let json = await runNativeInBackground { nativeCore in
            nativeCore.refreshJson()
        }
        guard !Task.isCancelled, generation == stateGeneration else { return }
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
        guard linkInput.kind == "app_key_approval", !linkInput.appKeyPubkey.isEmpty else {
            return request
        }
        return linkInput.appKeyPubkey
    }
}
