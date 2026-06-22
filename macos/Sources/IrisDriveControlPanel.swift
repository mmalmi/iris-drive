import AppKit
import CoreImage.CIFilterBuiltins
import SwiftUI
import UniformTypeIdentifiers
import Vision

private enum IrisDrivePanelTab: String, CaseIterable, Identifiable {
    case drive
    case peers
    case shares
    case backup
    case settings

    var id: Self { self }

    var title: String {
        switch self {
        case .drive:
            return "My Drive"
        case .peers:
            return "Devices"
        case .shares:
            return "Shares"
        case .backup:
            return "Backup"
        case .settings:
            return "Settings"
        }
    }

    var symbol: String {
        switch self {
        case .drive:
            return "externaldrive.fill"
        case .peers:
            return "person.2.fill"
        case .shares:
            return "person.3.fill"
        case .backup:
            return "arrow.triangle.2.circlepath"
        case .settings:
            return "gearshape.fill"
        }
    }

    static var initialScreenshotSelection: IrisDrivePanelTab {
        guard IrisDriveScreenshotFixtures.enabled else {
            return .drive
        }
        switch IrisDriveScreenshotFixtures.tabArgument {
        case "device", "devices", "peers":
            return .peers
        case "share", "shares":
            return .shares
        case "backup", "backups", "file-server", "file-servers", "server", "servers":
            return .backup
        case "setting", "settings":
            return .settings
        default:
            return .drive
        }
    }
}

private enum IrisDriveSetupMode {
    case welcome
    case create
    case createPhoto
    case restoreOptions
    case restorePhrase
    case restoreSecretKey
    case link
}

private enum RecoveryKeyFlowMode {
    case choose
    case generateNew
    case importExisting
}

private enum IrisDriveSyncState {
    case upToDate
    case syncing(Int, Int)
    case paused
    case attention
}

private struct ShareMemberRevokeTarget: Identifiable {
    let id = UUID()
    let share: IrisDriveShareStatus
    let member: IrisDriveShareMemberStatus
}

private let setupControlWidth: CGFloat = 340
private let recoveryPhraseWordCount = 12
let setupButtonMinHeight: CGFloat = 44

struct IrisDriveControlPanel: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate
    @State private var selectedTab = IrisDrivePanelTab.initialScreenshotSelection
    @State private var relayInput = ""
    @State private var backupURLInput = ""
    @State private var shareSourceInput = ""
    @State private var shareInviteInput = ""
    @State private var shareRecipientNpubHint = ""
    @State private var shareRecipientDisplayName = ""
    @State private var shareRecipientProfileId = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""
    @State private var setupMode = IrisDriveSetupMode.welcome
    @State private var setupUsername = ""
    @State private var setupPhotoPath = ""
    @State private var setupSecret = ""
    @State private var setupRecoveryWords = Array(repeating: "", count: recoveryPhraseWordCount)
    @State private var setupRecoveryWordIndex = 0
    @State private var setupLinkTarget = ""
    @State var submittedSetupLinkTarget = ""
    @State private var setupLinkTargetInputIsComplete = false
    @State private var approveDeviceKey = ""
    @State private var approveDeviceKeyIsComplete = false
    @State private var approveDeviceLabel = ""
    @State private var approveDeviceError = ""
    @State private var approveDevicePending = false
    @State private var generatedRecoveryWords: [String] = []
    @State private var generatedRecoveryPubkey = ""
    @State private var generatedRecoveryWordIndex = 0
    @State private var generatedRecoveryWrittenDown = false
    @State private var generatedRecoveryError = ""
    @State private var recoveryKeyFlowMode = RecoveryKeyFlowMode.choose
    @State private var importedRecoveryWords = Array(repeating: "", count: recoveryPhraseWordCount)
    @State private var importedRecoveryWordIndex = 0
    @State private var showAddDevice = false
    @State private var showAddRecoveryKey = false
    @State private var inviteShare: IrisDriveShareStatus?
    @State private var deleteShare: IrisDriveShareStatus?
    @State private var showMyNpub = false
    @State private var revokeShareMember: ShareMemberRevokeTarget?
    @State private var checkingAllBackups = false
    @State private var checkedBackupCount = 0
    @State private var backupCheckTotal = 0
    @State private var showLogoutConfirmation = false
    @State private var recoveryExport: [String: Any]?
    @State private var recoveryExportWordIndex = 0
    @State private var showStartupLoading = false

    var body: some View {
        Group {
            if !status.stateLoaded {
                startupLoading
            } else if !status.setupComplete {
                setup
            } else {
                controlPanel
            }
        }
        .onAppear {
            if status.stateLoaded {
                controller.ensureFileProviderDomainIfProfileExists()
            }
            applyPendingShareDialog()
        }
        .onChange(of: status.stateLoaded) { _, loaded in
            if loaded {
                controller.ensureFileProviderDomainIfProfileExists()
            }
        }
        .onChange(of: status.pendingShareDialog?.id) { _, _ in
            applyPendingShareDialog()
        }
        .task(id: status.stateLoaded) {
            await revealStartupLoadingIfNeeded()
        }
        .overlay(alignment: .bottom) {
            if let copyStatus = status.copyStatus, !copyStatus.isEmpty {
                IrisDriveCopyToast(message: copyStatus)
                    .padding(.bottom, 18)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }
        }
        .animation(.easeInOut(duration: 0.18), value: status.copyStatus)
        .animation(.easeInOut(duration: 0.18), value: status.stateLoaded)
        .animation(.easeInOut(duration: 0.18), value: showStartupLoading)
    }

    private var startupLoading: some View {
        StartupLoadingPanelView(showLabel: showStartupLoading)
    }

    @MainActor
    private func revealStartupLoadingIfNeeded() async {
        showStartupLoading = false
        guard !status.stateLoaded else { return }
        do {
            try await Task.sleep(nanoseconds: 2_000_000_000)
        } catch {
            return
        }
        guard !Task.isCancelled, !status.stateLoaded else { return }
        showStartupLoading = true
    }

    private var controlPanel: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            VStack(spacing: 0) {
                runtimeVersionStripe
                updateStripe
                content
            }
        }
    }

    @ViewBuilder
    private var runtimeVersionStripe: some View {
        if status.runtimeVersionMismatch {
            HStack(spacing: 12) {
                Label(status.runtimeVersionStripeText, systemImage: "exclamationmark.triangle")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Button(status.serviceVersionMismatch ? "Update Service" : "Restart Sync") {
                    controller.updateDaemonService()
                }
                .disabled(status.updateInstalling)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 8)
            .background(Color(nsColor: .controlBackgroundColor))
            Divider()
        }
    }

    @ViewBuilder
    private var updateStripe: some View {
        if status.updateAvailable {
            HStack(spacing: 12) {
                Label(status.updateStripeText, systemImage: "arrow.down.circle")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Toggle(
                    "Install automatically",
                    isOn: Binding(
                        get: { status.autoInstallUpdates },
                        set: { controller.setAutoInstallUpdates($0) }
                    )
                )
                .toggleStyle(.checkbox)
                .font(.caption)
                Button(status.updateAsset.lowercased().hasSuffix(".app.tar.gz") ? "Install" : "Download") {
                    controller.installUpdate()
                }
                .disabled(!status.updateCanInstall || status.updateChecking || status.updateInstalling)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 8)
            .background(Color(nsColor: .controlBackgroundColor))
            Divider()
        }
    }

    @ViewBuilder
    private var content: some View {
        switch selectedTab {
        case .drive:
            page { driveHome }
        case .peers:
            page { peers }
        case .shares:
            page { shares }
        case .backup:
            page { backup }
        case .settings:
            settingsForm
        }
    }

    private func page<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                content()
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func refreshSetupLinkTargetInput(_ value: String) {
        let query = value.trimmingCharacters(in: .whitespacesAndNewlines)
        setupLinkTargetInputIsComplete = false
        guard !query.isEmpty else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
            guard setupLinkTarget.trimmingCharacters(in: .whitespacesAndNewlines) == query else {
                return
            }
            controller.classifyLinkInput(query) { input, isComplete in
                guard setupLinkTarget.trimmingCharacters(in: .whitespacesAndNewlines) == input else {
                    return
                }
                setupLinkTargetInputIsComplete = isComplete
                if isComplete {
                    submitSetupLinkTarget(input, force: false, inputIsComplete: true)
                }
            }
        }
    }

    private func refreshApproveAppKeyLinkInput(_ value: String) {
        let query = value.trimmingCharacters(in: .whitespacesAndNewlines)
        approveDeviceKeyIsComplete = false
        guard !query.isEmpty else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
            guard approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines) == query else {
                return
            }
            controller.classifyLinkInput(query) { input, isComplete in
                guard approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines) == input else {
                    return
                }
                approveDeviceKeyIsComplete = isComplete
            }
        }
    }

    @ViewBuilder
    private var setup: some View {
        VStack(spacing: 18) {
            Spacer()
            Image("BrandIcon")
                .resizable()
                .interpolation(.high)
                .frame(width: 96, height: 96)
            Text("Iris Drive")
                .font(.title.weight(.semibold))
            setupContent
                .frame(width: setupControlWidth)
                .controlSize(.large)
                .buttonBorderShape(.roundedRectangle(radius: 5))
            if let setupStatusMessage {
                Text(setupStatusMessage)
                    .font(.callout)
                    .foregroundStyle(setupStatusColor)
                    .frame(width: setupControlWidth)
                    .multilineTextAlignment(.center)
            }
            Spacer()
        }
        .padding(32)
        .frame(minWidth: 520, minHeight: 420)
    }

    @ViewBuilder
    private var setupContent: some View {
        if status.revoked {
            RevokedDeviceSetupView(status: status, controller: controller)
        } else if status.awaitingApproval {
            AwaitingApprovalSetupView(status: status, controller: controller)
        } else {
            switch setupMode {
        case .welcome:
            VStack(spacing: 12) {
                Button {
                    setupMode = .create
                } label: {
                    setupButtonLabel("Create profile", systemImage: "plus")
                }
                .accessibilityLabel("Create profile")
                .buttonStyle(.borderedProminent)
                Button {
                    setupMode = .restoreOptions
                } label: {
                    setupButtonLabel("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .accessibilityLabel("Sign in")
                .buttonStyle(.bordered)
            }
        case .create:
            setupForm(title: "Create profile") {
                TextField("Username (optional)", text: $setupUsername)
                    .accessibilityLabel("Username")
                    .onSubmit {
                        let username = setupUsername.trimmingCharacters(in: .whitespacesAndNewlines)
                        if username.isEmpty {
                            controller.createProfile(username: "", profilePhotoPath: "")
                        } else {
                            setupMode = .createPhoto
                        }
                    }
                setupSubmit("Create profile") {
                    let username = setupUsername.trimmingCharacters(in: .whitespacesAndNewlines)
                    if username.isEmpty {
                        controller.createProfile(username: "", profilePhotoPath: "")
                    } else {
                        setupMode = .createPhoto
                    }
                }
            }
        case .createPhoto:
            setupForm(title: "Profile photo", backTarget: .create) {
                Button {
                    chooseProfilePhoto()
                } label: {
                    setupButtonLabel(
                        setupPhotoPath.isEmpty ? "Choose photo" : profilePhotoName,
                        systemImage: "photo"
                    )
                }
                .accessibilityLabel(setupPhotoPath.isEmpty ? "Choose photo" : profilePhotoName)
                .buttonStyle(.bordered)
                if !setupPhotoPath.isEmpty {
                    Button {
                        setupPhotoPath = ""
                    } label: {
                        setupButtonLabel("Remove photo", systemImage: "xmark")
                    }
                    .accessibilityLabel("Remove photo")
                    .buttonStyle(.bordered)
                }
                setupSubmit(setupPhotoPath.isEmpty ? "Later" : "Create profile") {
                    controller.createProfile(
                        username: setupUsername,
                        profilePhotoPath: setupPhotoPath
                    )
                }
            }
        case .restoreOptions:
            setupForm(title: "Restore") {
                Button {
                    setupMode = .link
                } label: {
                    setupButtonLabel("Link device", systemImage: "desktopcomputer")
                }
                .accessibilityLabel("Link device")
                .buttonStyle(.bordered)
                Button {
                    setupMode = .restorePhrase
                } label: {
                    setupButtonLabel("Restore from recovery phrase", systemImage: "text.badge.checkmark")
                }
                .accessibilityLabel("Restore from recovery phrase")
                .buttonStyle(.bordered)
                Button {
                    setupMode = .restoreSecretKey
                } label: {
                    setupButtonLabel("Restore from secret key", systemImage: "key")
                }
                .accessibilityLabel("Restore from secret key")
                .buttonStyle(.bordered)
            }
        case .restorePhrase:
            setupForm(title: "Recovery phrase", backTarget: .restoreOptions) {
                TextField("Word \(setupRecoveryWordIndex + 1)", text: recoveryWordBinding)
                    .onSubmit {
                        advanceOrRestoreRecoveryPhrase()
                    }
                Button {
                    applyRecoveryWordInput(NSPasteboard.general.string(forType: .string) ?? "")
                } label: {
                    setupButtonLabel("Paste from Clipboard", systemImage: "doc.on.clipboard")
                }
                .buttonStyle(.bordered)
                HStack(spacing: 8) {
                    Button {
                        setupRecoveryWordIndex = max(0, setupRecoveryWordIndex - 1)
                    } label: {
                        setupButtonLabel("Back", systemImage: "chevron.left")
                    }
                    .disabled(setupRecoveryWordIndex == 0)
                    .buttonStyle(.bordered)
                    setupSubmit(setupRecoveryWordIndex == recoveryPhraseWordCount - 1 ? "Restore" : "Next") {
                        advanceOrRestoreRecoveryPhrase()
                    }
                    .disabled(setupRecoveryWordIndex == recoveryPhraseWordCount - 1 ? !allRecoveryWordsFilled : currentRecoveryWord.isEmpty)
                }
            }
        case .restoreSecretKey:
            setupForm(title: "Secret key", backTarget: .restoreOptions) {
                SecureField("nsec1... or hex secret key", text: $setupSecret)
                    .onSubmit {
                        controller.restoreProfile(recoverySecret: setupSecret)
                    }
                setupSubmit("Restore") {
                    controller.restoreProfile(recoverySecret: setupSecret)
                }
                .disabled(setupSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        case .link:
            setupForm(title: "Link device", backTarget: .restoreOptions) {
                TextField("Invite link or device key", text: $setupLinkTarget)
                    .accessibilityLabel("Invite link or device key")
                    .onSubmit {
                        submitSetupLinkTarget(
                            setupLinkTarget,
                            force: true,
                            inputIsComplete: setupLinkTargetInputIsComplete
                        )
                    }
                    .onChange(of: setupLinkTarget) { _, newValue in
                        refreshSetupLinkTargetInput(newValue)
                    }
                    .onAppear {
                        refreshSetupLinkTargetInput(setupLinkTarget)
                    }
                setupSubmit("Link device") {
                    submitSetupLinkTarget(
                        setupLinkTarget,
                        force: true,
                        inputIsComplete: setupLinkTargetInputIsComplete
                    )
                }
                .disabled(!setupLinkTargetInputIsComplete)
            }
        }
        }
    }

    private func setupForm<Content: View>(
        title: String,
        backTarget: IrisDriveSetupMode = .welcome,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                setupMode = backTarget
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .buttonStyle(.borderless)
            Text(title)
                .font(.title2.weight(.semibold))
            content()
        }
        .textFieldStyle(.roundedBorder)
    }

    private var profilePhotoName: String {
        URL(fileURLWithPath: setupPhotoPath).lastPathComponent
    }

    private var recoveryWordBinding: Binding<String> {
        Binding(
            get: { setupRecoveryWords[setupRecoveryWordIndex] },
            set: { applyRecoveryWordInput($0) }
        )
    }

    private var currentRecoveryWord: String {
        setupRecoveryWords[setupRecoveryWordIndex]
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var allRecoveryWordsFilled: Bool {
        setupRecoveryWords.allSatisfy {
            !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    private var setupRecoveryPhrase: String {
        setupRecoveryWords
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .joined(separator: " ")
    }

    private func advanceOrRestoreRecoveryPhrase() {
        if setupRecoveryWordIndex == recoveryPhraseWordCount - 1 {
            guard allRecoveryWordsFilled else { return }
            controller.restoreProfile(recoverySecret: setupRecoveryPhrase)
        } else if !currentRecoveryWord.isEmpty {
            setupRecoveryWordIndex = min(recoveryPhraseWordCount - 1, setupRecoveryWordIndex + 1)
        }
    }

    private func applyRecoveryWordInput(_ value: String) {
        let words = value
            .split(whereSeparator: { $0.isWhitespace })
            .map { String($0).lowercased() }
        if words.count <= 1 {
            setupRecoveryWords[setupRecoveryWordIndex] =
                value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return
        }
        for (offset, word) in words.enumerated() where setupRecoveryWordIndex + offset < setupRecoveryWords.count {
            setupRecoveryWords[setupRecoveryWordIndex + offset] = word
        }
        setupRecoveryWordIndex = min(recoveryPhraseWordCount - 1, setupRecoveryWordIndex + words.count - 1)
    }

    private var setupStatusMessage: String? {
        let message = status.message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else { return nil }
        let lowercased = message.lowercased()
        if lowercased == "setting up"
            || lowercased.contains("failed")
            || lowercased.hasSuffix("required") {
            return message
        }
        return nil
    }

    private var setupStatusColor: Color {
        (setupStatusMessage ?? "").localizedCaseInsensitiveContains("failed") ? .red : .secondary
    }

    private func chooseProfilePhoto() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            setupPhotoPath = url.path
        }
    }

    private func setupSubmit(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            setupButtonLabel(title)
        }
        .accessibilityLabel(title)
        .buttonStyle(.borderedProminent)
    }

    private func setupButtonLabel(_ title: String, systemImage: String? = nil) -> some View {
        HStack(spacing: 8) {
            if let systemImage {
                Image(systemName: systemImage)
                    .frame(width: 18)
            }
            Text(title)
        }
        .frame(maxWidth: .infinity, minHeight: setupButtonMinHeight)
        .contentShape(Rectangle())
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(IrisDrivePanelTab.allCases) { tab in
                SidebarRow(
                    symbol: tab.symbol,
                    title: tab.title,
                    selected: selectedTab == tab
                ) {
                    selectedTab = tab
                }
            }
            Divider()
                .padding(.vertical, 4)
            Button {
                controller.showDriveFolder()
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "folder.fill")
                        .frame(width: 16)
                    Text("Open")
                    Spacer(minLength: 0)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.bordered)
            .controlSize(.regular)
            .accessibilityIdentifier("sidebarOpenDrive")
            .accessibilityLabel("Open")
            .padding(.bottom, 4)
            Spacer()
        }
        .padding(.vertical, 18)
        .padding(.horizontal, 12)
        .frame(width: 160)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    // MARK: My Drive

    private var driveHome: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 16) {
                    Image(systemName: heroIcon)
                        .font(.system(size: 40, weight: .semibold))
                        .foregroundStyle(heroColor)
                        .frame(width: 48)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(status.driveName)
                            .font(.title2.weight(.semibold))
                        Text(heroText)
                            .font(.headline)
                            .foregroundStyle(heroColor)
                    }
                    Spacer()
                }
                Divider()
                Text(summaryLine)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 8))

            Button(action: controller.showDriveFolder) {
                Label("Open in Finder", systemImage: "folder.fill")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            if status.localNhashResolverEnabled {
                Button(action: controller.openSitesPortal) {
                    Label("Open Iris Apps", systemImage: "safari")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .disabled(status.sitesPortalURL == nil)
            }

            Button(action: controller.openDriveLink) {
                Label("View on drive.iris.to", systemImage: "safari")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .controlSize(.large)
            .disabled(status.snapshotLinkURL == nil)
        }
    }

    private var syncState: IrisDriveSyncState {
        if !status.daemonRunning || status.syncStatus == "paused" {
            return .paused
        }
        if status.syncStatus == "sync error" {
            return .attention
        }
        if let upload = status.lastUpload,
           upload.isInProgress {
            return .syncing(upload.completedHashes, upload.totalHashes)
        }
        return .upToDate
    }

    private var heroIcon: String {
        switch syncState {
        case .upToDate:
            return "checkmark.circle.fill"
        case .syncing:
            return "arrow.triangle.2.circlepath"
        case .paused:
            return "pause.circle.fill"
        case .attention:
            return "exclamationmark.triangle.fill"
        }
    }

    private var heroColor: Color {
        switch syncState {
        case .upToDate:
            return .green
        case .syncing:
            return .accentColor
        case .paused:
            return .secondary
        case .attention:
            return .orange
        }
    }

    private var heroText: String {
        if !status.daemonRunning {
            return "Daemon offline"
        }
        switch syncState {
        case let .syncing(done, total):
            return "Syncing \(done) of \(total)…"
        case .upToDate, .paused, .attention:
            return status.syncStatusLabel
        }
    }

    private var summaryLine: String {
        let files = status.fileCount ?? 0
        let usedBytes = status.visibleFileBytes ?? 0
        let parts: [String?] = [
            countLabel(files, "file"),
            "\(byteString(usedBytes)) used",
            "\(status.onlineDeviceCount)/\(status.authorizedDeviceCount) online",
            status.daemonRunning ? nil : "daemon not running",
        ]
        return parts.compactMap { $0 }.joined(separator: "  ·  ")
    }

    // MARK: Devices

    private var devicePeers: [IrisDrivePeerStatus] {
        status.peers.filter(\.isDeviceActor)
    }

    private var recoveryKeyPeers: [IrisDrivePeerStatus] {
        status.peers.filter { !$0.isDeviceActor }
    }

    private var peers: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Devices")
                Spacer()
                if status.canAdminProfile {
                    HStack(spacing: 8) {
                        Button {
                            showAddDevice = true
                        } label: {
                            Label("Add Device", systemImage: "plus")
                        }
                        Button {
                            openRecoveryKeyFlow()
                        } label: {
                            Label("Add Recovery Key", systemImage: "key.fill")
                        }
                    }
                }
            }
            if devicePeers.isEmpty {
                emptyState("No devices yet")
            } else {
                let adminCount = devicePeers.filter { $0.role == "admin" }.count
                ForEach(devicePeers) { peer in
                    PeerRow(
                        peer: peer,
                        canManageDevices: status.canAdminProfile,
                        adminCount: adminCount,
                        controller: controller
                    )
                }
            }
            if !recoveryKeyPeers.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Recovery Keys")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(recoveryKeyPeers) { peer in
                        PeerRow(
                            peer: peer,
                            canManageDevices: status.canAdminProfile,
                            adminCount: 0,
                            controller: controller
                        )
                    }
                }
            }
            if !status.inboundAppKeyLinkRequests.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Requests")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(status.inboundAppKeyLinkRequests) { request in
                        AppKeyLinkRequestRow(request: request, controller: controller)
                    }
                }
            }
        }
        .sheet(isPresented: $showAddDevice) {
            addDeviceSheet
        }
        .sheet(isPresented: $showAddRecoveryKey, onDismiss: resetRecoveryKeyFlow) {
            addRecoveryKeySheet
        }
    }

    private var addDeviceSheet: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Add a Device")
                .font(.title3.weight(.semibold))
            if let invite = status.appKeyLinkInviteURL, !invite.isEmpty {
                Text("Invite device")
                    .font(.headline)
                IrisDriveQRCodeView(value: invite)
                    .frame(width: 220, height: 220)
                    .frame(maxWidth: .infinity, alignment: .center)
                IrisDriveCopyButton(title: "Copy invite link", systemImage: "link") {
                    irisDriveCopyToPasteboard(invite, feedback: "Invite link copied")
                }
                Button {
                    controller.resetInvite()
                } label: {
                    Label("Reset invite", systemImage: "arrow.clockwise")
                }
            }
            if !status.inboundAppKeyLinkRequests.isEmpty {
                Text("Device requests")
                    .font(.headline)
                ForEach(status.inboundAppKeyLinkRequests) { request in
                    AppKeyLinkRequestRow(request: request, controller: controller)
                }
            }
            Text("Paste the device key or request link.")
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField("Device key", text: $approveDeviceKey)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
                .onChange(of: approveDeviceKey) { _, newValue in
                    approveDeviceError = ""
                    refreshApproveAppKeyLinkInput(newValue)
                }
                .onAppear {
                    refreshApproveAppKeyLinkInput(approveDeviceKey)
                }
            if !approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
               !approveDeviceKeyIsComplete {
                Text("That is not a complete device key or request link.")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            if !approveDeviceError.isEmpty {
                Text(approveDeviceError)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            TextField("Name (optional)", text: $approveDeviceLabel)
                .textFieldStyle(.roundedBorder)
            HStack {
                Spacer()
                Button("Cancel") {
                    showAddDevice = false
                }
                Button("Add") {
                    approveDevicePending = true
                    approveDeviceError = ""
                    controller.approveDevice(approveDeviceKey, label: approveDeviceLabel) { result in
                        approveDevicePending = false
                        switch result {
                        case .success:
                            approveDeviceKey = ""
                            approveDeviceKeyIsComplete = false
                            approveDeviceLabel = ""
                            showAddDevice = false
                        case let .failure(error):
                            approveDeviceError = error.localizedDescription
                        }
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(!approveDeviceKeyIsComplete || approveDevicePending)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private var addRecoveryKeySheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Add Recovery Key")
                    .font(.title3.weight(.semibold))
                Spacer()
                Button {
                    showAddRecoveryKey = false
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Close")
            }
            recoveryKeySheetContent
        }
        .padding(20)
        .frame(width: 420)
    }

    @ViewBuilder
    private var recoveryKeySheetContent: some View {
        switch recoveryKeyFlowMode {
        case .choose:
            Button {
                startRecoveryKeyFlow()
            } label: {
                Label("Generate New", systemImage: "sparkles")
                    .frame(maxWidth: .infinity, minHeight: setupButtonMinHeight)
            }
            .buttonStyle(.borderedProminent)
            Button {
                startRecoveryImportFlow()
            } label: {
                Label("Import Existing", systemImage: "square.and.arrow.down")
                    .frame(maxWidth: .infinity, minHeight: setupButtonMinHeight)
            }
            .buttonStyle(.bordered)
            HStack {
                Spacer()
                Button("Cancel") {
                    showAddRecoveryKey = false
                }
            }
        case .generateNew:
            generatedRecoveryKeySheetContent
        case .importExisting:
            importRecoveryKeySheetContent
        }
    }

    @ViewBuilder
    private var generatedRecoveryKeySheetContent: some View {
        if !generatedRecoveryError.isEmpty {
            Text(generatedRecoveryError)
                .foregroundStyle(.red)
            HStack {
                Button {
                    recoveryKeyFlowMode = .choose
                } label: {
                    Label("Back", systemImage: "chevron.left")
                }
                Spacer()
                Button("Generate Again") {
                    startRecoveryKeyFlow()
                }
                .buttonStyle(.borderedProminent)
            }
        } else if generatedRecoveryWords.indices.contains(generatedRecoveryWordIndex) {
            Text("Word \(generatedRecoveryWordIndex + 1) of \(generatedRecoveryWords.count)")
                .font(.headline)
                .foregroundStyle(.secondary)
            Text(generatedRecoveryWords[generatedRecoveryWordIndex])
                .font(.system(size: 34, weight: .semibold, design: .serif))
                .frame(maxWidth: .infinity, minHeight: 84)
                .background(Color(nsColor: .textBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .textSelection(.enabled)
            HStack {
                Button {
                    generatedRecoveryWordIndex = max(0, generatedRecoveryWordIndex - 1)
                } label: {
                    Label("Back", systemImage: "chevron.left")
                }
                .disabled(generatedRecoveryWordIndex == 0)
                Spacer()
                if generatedRecoveryWordIndex + 1 < generatedRecoveryWords.count {
                    Button {
                        generatedRecoveryWordIndex += 1
                    } label: {
                        Label("Next", systemImage: "chevron.right")
                    }
                    .buttonStyle(.borderedProminent)
                }
            }
            if generatedRecoveryWordIndex + 1 == generatedRecoveryWords.count {
                Toggle("I wrote down all 12 words", isOn: $generatedRecoveryWrittenDown)
                HStack {
                    Spacer()
                    Button("Cancel") {
                        showAddRecoveryKey = false
                    }
                    Button("Add Recovery Key") {
                        controller.addRecoveryDevice(generatedRecoveryPubkey)
                        resetRecoveryKeyFlow()
                        showAddRecoveryKey = false
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!generatedRecoveryWrittenDown || generatedRecoveryPubkey.isEmpty)
                }
            }
        }
    }

    @ViewBuilder
    private var importRecoveryKeySheetContent: some View {
        if !generatedRecoveryError.isEmpty {
            Text(generatedRecoveryError)
                .foregroundStyle(.red)
        }
        Text("Word \(importedRecoveryWordIndex + 1) of \(recoveryPhraseWordCount)")
            .font(.headline)
            .foregroundStyle(.secondary)
        SecureField(
            "Word \(importedRecoveryWordIndex + 1)",
            text: importedRecoveryWordBinding
        )
        .textFieldStyle(.roundedBorder)
        .disableAutocorrection(true)
        .onSubmit {
            advanceRecoveryImportWord()
        }
        HStack {
            Button {
                if importedRecoveryWordIndex == 0 {
                    recoveryKeyFlowMode = .choose
                    generatedRecoveryError = ""
                } else {
                    importedRecoveryWordIndex -= 1
                    generatedRecoveryError = ""
                }
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            Spacer()
            if importedRecoveryWordIndex + 1 < recoveryPhraseWordCount {
                Button {
                    advanceRecoveryImportWord()
                } label: {
                    Label("Next", systemImage: "chevron.right")
                }
                .buttonStyle(.borderedProminent)
                .disabled(currentImportedRecoveryWord.isEmpty)
            } else {
                Button("Add Recovery Key") {
                    saveImportedRecoveryKey()
                }
                .buttonStyle(.borderedProminent)
                .disabled(!allImportedRecoveryWordsFilled)
            }
        }
    }

    private var importedRecoveryWordBinding: Binding<String> {
        Binding(
            get: {
                guard importedRecoveryWords.indices.contains(importedRecoveryWordIndex) else {
                    return ""
                }
                return importedRecoveryWords[importedRecoveryWordIndex]
            },
            set: { value in
                guard importedRecoveryWords.indices.contains(importedRecoveryWordIndex) else {
                    return
                }
                importedRecoveryWords[importedRecoveryWordIndex] = value
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                    .lowercased()
            }
        )
    }

    private var currentImportedRecoveryWord: String {
        guard importedRecoveryWords.indices.contains(importedRecoveryWordIndex) else {
            return ""
        }
        return importedRecoveryWords[importedRecoveryWordIndex]
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var allImportedRecoveryWordsFilled: Bool {
        importedRecoveryWords.allSatisfy {
            !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    private func advanceRecoveryImportWord() {
        guard !currentImportedRecoveryWord.isEmpty else { return }
        generatedRecoveryError = ""
        importedRecoveryWordIndex = min(recoveryPhraseWordCount - 1, importedRecoveryWordIndex + 1)
    }

    private func saveImportedRecoveryKey() {
        let phrase = importedRecoveryWords
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .joined(separator: " ")
        let payload = IrisDriveDesktopCore.recoveryPubkeyForPhrase(phrase)
        let error = payload["error"] as? String ?? ""
        guard error.isEmpty else {
            generatedRecoveryError = error
            return
        }
        let recoveryPubkey = payload["recovery_pubkey"] as? String ?? ""
        guard !recoveryPubkey.isEmpty else {
            generatedRecoveryError = "Recovery key import failed"
            return
        }
        controller.addRecoveryDevice(recoveryPubkey)
        resetRecoveryKeyFlow()
        showAddRecoveryKey = false
    }

    private func openRecoveryKeyFlow() {
        resetRecoveryKeyFlow()
        showAddRecoveryKey = true
    }

    private func startRecoveryKeyFlow() {
        generatedRecoveryWords = []
        generatedRecoveryPubkey = ""
        generatedRecoveryWordIndex = 0
        generatedRecoveryWrittenDown = false
        generatedRecoveryError = ""
        importedRecoveryWords = Array(repeating: "", count: recoveryPhraseWordCount)
        importedRecoveryWordIndex = 0
        recoveryKeyFlowMode = .generateNew
        let payload = IrisDriveDesktopCore.generateRecoveryKey()
        generatedRecoveryError = payload["error"] as? String ?? ""
        generatedRecoveryWords = payload["words"] as? [String] ?? []
        generatedRecoveryPubkey = payload["recovery_pubkey"] as? String ?? ""
        if generatedRecoveryError.isEmpty,
           (generatedRecoveryWords.count != recoveryPhraseWordCount || generatedRecoveryPubkey.isEmpty)
        {
            generatedRecoveryError = "Recovery key generation failed"
        }
    }

    private func startRecoveryImportFlow() {
        generatedRecoveryWords = []
        generatedRecoveryPubkey = ""
        generatedRecoveryWordIndex = 0
        generatedRecoveryWrittenDown = false
        generatedRecoveryError = ""
        importedRecoveryWords = Array(repeating: "", count: recoveryPhraseWordCount)
        importedRecoveryWordIndex = 0
        recoveryKeyFlowMode = .importExisting
    }

    private func resetRecoveryKeyFlow() {
        recoveryKeyFlowMode = .choose
        generatedRecoveryWords = []
        generatedRecoveryPubkey = ""
        generatedRecoveryWordIndex = 0
        generatedRecoveryWrittenDown = false
        generatedRecoveryError = ""
        importedRecoveryWords = Array(repeating: "", count: recoveryPhraseWordCount)
        importedRecoveryWordIndex = 0
    }

    // MARK: Shares

    private func applyPendingShareDialog() {
        guard let request = status.pendingShareDialog else { return }
        selectedTab = .shares
        shareSourceInput = request.sourcePath
        shareRecipientNpubHint = request.recipientNpubHint
        shareRecipientDisplayName = request.recipientDisplayName
        shareRecipientProfileId = request.recipientProfileId
    }

    private var shares: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Shares")
                Spacer()
                if myShareNpub != nil {
                    Button {
                        showMyNpub = true
                    } label: {
                        Label("My User ID", systemImage: "qrcode")
                    }
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Create Shared Folder")
                    .font(.headline)
                TextField("Folder path", text: $shareSourceInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                Button {
                    controller.createShare(sourcePath: shareSourceInput) {
                        shareSourceInput = ""
                    }
                } label: {
                    Label("Create Shared Folder", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .disabled(shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Join Shared Folder")
                    .font(.headline)
                TextField("Paste invite link", text: $shareInviteInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                HStack(spacing: 8) {
                    Button {
                        shareInviteInput = NSPasteboard.general
                            .string(forType: .string)?
                            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    } label: {
                        Label("Paste Invite", systemImage: "doc.on.clipboard")
                    }
                    Button {
                        scanQRCodeFromImage { code in
                            shareInviteInput = code
                        }
                    } label: {
                        Label("Scan QR", systemImage: "qrcode.viewfinder")
                    }
                    Spacer()
                    Button {
                        controller.acceptShareInvite(shareInviteInput)
                        shareInviteInput = ""
                    } label: {
                        Label("Join", systemImage: "tray.and.arrow.down.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(shareInviteInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }

            if status.shares.isEmpty {
                emptyState("No shared folders")
            } else {
                ForEach(status.shares) { share in
                    ShareStatusRow(
                        share: share,
                        localProfileId: status.profileId,
                        driveLink: shareDriveLink(share, status: status),
                        onOpen: {
                            controller.openShareFolder(path: shareOpenPath(share))
                        },
                        onOpenDriveLink: { link in
                            openDriveIrisLink(link)
                        },
                        onInvite: { inviteShare = share },
                        onDelete: { deleteShare = share },
                        onRepair: { controller.repairShareWraps(shareId: share.shareId) },
                        onRevoke: { member in
                            revokeShareMember = ShareMemberRevokeTarget(
                                share: share,
                                member: member
                            )
                        }
                    )
                }
            }
        }
        .sheet(item: $inviteShare) { share in
            InviteShareMemberSheet(
                controller: controller,
                status: status,
                share: share,
                profileId: shareRecipientProfileId,
                representativeNpubHint: shareRecipientNpubHint,
                displayName: shareRecipientDisplayName
            )
        }
        .sheet(isPresented: $showMyNpub) {
            MyShareNpubSheet(npub: myShareNpub ?? "")
        }
        .alert(
            "Delete share?",
            isPresented: Binding(
                get: { deleteShare != nil },
                set: { presented in
                    if !presented {
                        deleteShare = nil
                    }
                }
            ),
            presenting: deleteShare
        ) { share in
            Button("Delete", role: .destructive) {
                controller.deleteShare(
                    shareId: share.shareId,
                    providerPaths: shareProviderSignalPaths(share)
                )
                deleteShare = nil
            }
            Button("Cancel", role: .cancel) {
                deleteShare = nil
            }
        } message: { share in
            Text("Delete \(shareDisplayName(share)) from this device? Folder contents stay in My Drive.")
        }
        .alert(
            "Revoke access?",
            isPresented: Binding(
                get: { revokeShareMember != nil },
                set: { presented in
                    if !presented {
                        revokeShareMember = nil
                    }
                }
            ),
            presenting: revokeShareMember
        ) { target in
            Button("Revoke", role: .destructive) {
                controller.revokeShareMember(
                    shareId: target.share.shareId,
                    profileId: target.member.profileId
                )
                revokeShareMember = nil
            }
            Button("Cancel", role: .cancel) {
                revokeShareMember = nil
            }
        } message: { target in
            Text("Revoke \(shareMemberDisplayName(target.member)) from \(shareDisplayName(target.share))?")
        }
    }

    private var myShareNpub: String? {
        let value = (status.currentAppKeyNpub ?? status.deviceNpub ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }

    private var checkingAllBackupsLabel: String {
        if backupCheckTotal > 0 {
            return "Checking \(checkedBackupCount) of \(backupCheckTotal)"
        }
        return "Checked \(checkedBackupCount)"
    }

    // MARK: Backup

    private var backup: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Backup")
                Spacer()
                Button {
                    guard !checkingAllBackups else { return }
                    let targets = status.backupTargets
                    checkedBackupCount = 0
                    backupCheckTotal = targets.count
                    checkingAllBackups = true
                    controller.checkBackups(targets, progress: { checked, total in
                        checkedBackupCount = checked
                        backupCheckTotal = total
                    }) {
                        checkingAllBackups = false
                        checkedBackupCount = 0
                        backupCheckTotal = 0
                    }
                } label: {
                    if checkingAllBackups {
                        Text(checkingAllBackupsLabel)
                    } else {
                        Label("Check All", systemImage: "checkmark.shield")
                    }
                }
                .disabled(status.backupTargets.isEmpty || checkingAllBackups)
                Button {
                    controller.syncBackups(status.backupTargets)
                } label: {
                    Label("Sync Now", systemImage: "arrow.up.circle")
                }
                .disabled(status.backupTargets.isEmpty)
            }
            if checkingAllBackups {
                VStack(alignment: .leading, spacing: 4) {
                    ProgressView(
                        value: Double(checkedBackupCount),
                        total: Double(max(backupCheckTotal, 1))
                    )
                    Text(checkingAllBackupsLabel)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            HStack(spacing: 8) {
                Button {
                    chooseBackupFolder()
                } label: {
                    Label("Choose Folder", systemImage: "folder.badge.plus")
                }
                TextField("https://backup.example", text: $backupURLInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                    .onSubmit { addBackupURLFromInput() }
                Button {
                    addBackupURLFromInput()
                } label: {
                    Label("Add Backup", systemImage: "plus")
                }
                .disabled(backupURLInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            if status.backupTargets.isEmpty {
                emptyState("No backups configured")
            } else {
                ForEach(status.backupTargets) { target in
                    BackupTargetRow(
                        target: target,
                        onCheck: { completion in
                            controller.checkBackups([target], completion: completion)
                        },
                        onRemove: {
                            controller.removeBackupTarget(target.target)
                        }
                    )
                }
            }
        }
    }

    // MARK: Settings

    private var settingsForm: some View {
        Form {
            Section("General") {
                Toggle(
                    "Keep in menu bar when closed",
                    isOn: Binding(
                        get: { status.closeToMenuBarOnClose },
                        set: { controller.setCloseToMenuBarOnClose($0) }
                    )
                )
                Toggle(
                    "Launch on startup",
                    isOn: Binding(
                        get: { status.launchOnStartup },
                        set: { controller.setLaunchOnStartup($0) }
                    )
                )
                Toggle(
                    "*.iris.localhost resolver",
                    isOn: Binding(
                        get: { status.localNhashResolverEnabled },
                        set: { controller.setLocalNhashResolver($0) }
                    )
                )
            }

            Section("Calendar") {
                let caldavURL = status.caldavURL?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                LabeledContent("CalDAV URL", value: caldavURL.isEmpty ? "Unavailable" : caldavURL)
                LabeledContent("Account Type", value: "Advanced")
                LabeledContent("User Name", value: "iris")
                LabeledContent("Password", value: "iris")
                LabeledContent("Server Address", value: "localhost")
                LabeledContent("Server Path", value: "/caldav/")
                LabeledContent("Port", value: irisDriveCalDAVPort(caldavURL))
                LabeledContent("Use SSL", value: "Off")
                LabeledContent("Use Kerberos", value: "Off")
                if !caldavURL.isEmpty {
                    IrisDriveCopyButton(title: "Copy CalDAV URL", systemImage: "calendar.badge.plus") {
                        irisDriveCopyToPasteboard(caldavURL, feedback: "CalDAV URL copied")
                    }
                }
            }

            Section("Updates") {
                LabeledContent("Version", value: appVersion)
                if !status.daemonBinaryVersion.isEmpty {
                    LabeledContent("Daemon", value: status.daemonBinaryVersion)
                }
                if !status.serviceBinaryVersion.isEmpty {
                    LabeledContent("Daemon service", value: status.serviceBinaryVersion)
                }
                Toggle(
                    "Check automatically",
                    isOn: Binding(
                        get: { status.autoCheckUpdates },
                        set: { controller.setAutoCheckUpdates($0) }
                    )
                )
                Toggle(
                    "Install automatically",
                    isOn: Binding(
                        get: { status.autoInstallUpdates },
                        set: { controller.setAutoInstallUpdates($0) }
                    )
                )
                HStack {
                    Button {
                        controller.checkForUpdates()
                    } label: {
                        Label(
                            status.updateChecking ? "Checking for Updates" : "Check for Updates",
                            systemImage: "arrow.clockwise"
                        )
                    }
                    .disabled(status.updateChecking || status.updateInstalling)
                    Button {
                        controller.installUpdate()
                    } label: {
                        Label(
                            status.updateAsset.lowercased().hasSuffix(".app.tar.gz") ? "Install Update" : "Download Update",
                            systemImage: "arrow.down.circle"
                        )
                    }
                    .disabled(!status.updateCanInstall || status.updateChecking || status.updateInstalling)
                }
                if !status.updateStatus.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    LabeledContent("Status", value: status.updateStatus)
                }
            }

            Section("Account") {
                AccountKeyRow(title: "Device", value: status.currentAppKeyNpub) {
                    controller.copyAppKey()
                }
                AccountKeyRow(title: "Current Device Key", value: status.deviceNpub) {
                    controller.copyDeviceKey()
                }
                if status.canExportRecoveryPhrase {
                    Button {
                        recoveryExport = controller.exportRecoverySecret()
                        recoveryExportWordIndex = 0
                    } label: {
                        Label("Recovery Phrase", systemImage: "text.badge.checkmark")
                    }
                }
                Button(role: .destructive) {
                    showLogoutConfirmation = true
                } label: {
                    Label("Log Out", systemImage: "rectangle.portrait.and.arrow.right")
                }
            }

            Section("Network") {
                relayEditor
                EndpointGroup(title: "File Servers", values: status.blossomServers)
                FipsDiagnostics(status: status.fips)
            }

            Section("Sync & advanced") {
                if status.daemonRunning {
                    Button("Pause sync") { controller.stopSync() }
                } else {
                    Button("Resume sync") { controller.startSync() }
                }
                IrisDriveCopyButton(title: "Copy drive.iris.to link", systemImage: "link") {
                    controller.copyDriveLink()
                }
                    .disabled(status.snapshotLinkURL == nil)
                Button("View on drive.iris.to") { controller.openDriveLink() }
                    .disabled(status.snapshotLinkURL == nil)
                LabeledContent(
                    "Storage",
                    value: byteString(status.visibleFileBytes ?? 0)
                )
            }

            Section("About") {
                LabeledContent("Drive", value: status.driveName)
            }
        }
        .formStyle(.grouped)
        .confirmationDialog(
            "Log out of Iris Drive on this Mac?",
            isPresented: $showLogoutConfirmation,
            titleVisibility: .visible
        ) {
            Button("Log Out", role: .destructive) {
                controller.logout()
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(
            isPresented: Binding(
                get: { recoveryExport != nil },
                set: { presented in
                    if !presented { recoveryExport = nil }
                }
            )
        ) {
            MacRecoveryPhraseView(
                payload: recoveryExport ?? [:],
                wordIndex: $recoveryExportWordIndex
            )
        }
    }

    private var relayEditor: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Relays")
                .font(.caption)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 6) {
                ForEach(relayRows) { relay in
                    relayRow(relay)
                }
            }
            HStack(spacing: 8) {
                TextField("wss://relay.example", text: $relayInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                    .onSubmit { addRelayFromInput() }
                Button {
                    addRelayFromInput()
                } label: {
                    Image(systemName: "plus")
                }
                .disabled(relayInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            Button {
                controller.resetRelays()
            } label: {
                Label("Reset", systemImage: "arrow.counterclockwise")
            }
            .buttonStyle(.borderless)
        }
    }

    private var relayRows: [IrisDriveRelayStatus] {
        status.relayStatuses
    }

    @ViewBuilder
    private func relayRow(_ relay: IrisDriveRelayStatus) -> some View {
        if editingRelayURL == relay.url {
            HStack(spacing: 8) {
                TextField("Relay URL", text: $editingRelayDraft)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                    .onSubmit { saveRelayEdit(relay.url) }
                Button {
                    saveRelayEdit(relay.url)
                } label: {
                    Image(systemName: "checkmark")
                }
                .buttonStyle(.borderless)
                Button {
                    editingRelayURL = nil
                    editingRelayDraft = ""
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
            }
        } else {
            HStack(spacing: 8) {
                Circle()
                    .fill(relayHealthColor(relay.health))
                    .frame(width: 8, height: 8)
                Text(relay.url)
                    .font(.system(.body, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 8)
                Text(relay.statusLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button {
                    editingRelayURL = relay.url
                    editingRelayDraft = relay.url
                } label: {
                    Image(systemName: "pencil")
                }
                .buttonStyle(.borderless)
                Button(role: .destructive) {
                    controller.removeRelay(relay.url)
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
            }
            .padding(.vertical, 2)
        }
    }

    // MARK: Helpers

    private func emptyState(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 8)
    }

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
    }

    private func countLabel(_ value: Int, _ singular: String) -> String {
        "\(value) \(singular)\(value == 1 ? "" : "s")"
    }

    private func byteString(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private func addRelayFromInput() {
        let value = relayInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        controller.addRelay(value)
        relayInput = ""
    }

    private func addBackupURLFromInput() {
        let value = backupURLInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        guard value.hasPrefix("https://") || value.hasPrefix("http://") else {
            NSSound.beep()
            controller.updateStatus("Use http(s) URL or choose a folder")
            return
        }
        controller.addBackupTarget(value, label: "")
        backupURLInput = ""
    }

    private func chooseBackupFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.prompt = "Choose"
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            controller.addBackupTarget(url.path, label: "")
        }
    }

    private func saveRelayEdit(_ oldURL: String) {
        let value = editingRelayDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        controller.updateRelay(oldURL, newValue: value)
        editingRelayURL = nil
        editingRelayDraft = ""
    }

    private func relayHealthColor(_ health: String) -> Color {
        switch health {
        case "online":
            return .green
        case "connecting":
            return .yellow
        case "error":
            return .red.opacity(0.85)
        default:
            return .secondary.opacity(0.65)
        }
    }
}

func irisDriveCopyToPasteboard(_ value: String) {
    irisDriveCopyToPasteboard(value, feedback: "Copied")
}

func irisDriveCopyToPasteboard(_ value: String, feedback: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(value, forType: .string)
    irisDriveShowCopyFeedback(feedback)
}

private var irisDriveCopyFeedbackGeneration = 0

private func irisDriveShowCopyFeedback(_ message: String) {
    DispatchQueue.main.async {
        irisDriveCopyFeedbackGeneration += 1
        let generation = irisDriveCopyFeedbackGeneration
        IrisDriveStatus.shared.copyStatus = message
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            guard generation == irisDriveCopyFeedbackGeneration else { return }
            IrisDriveStatus.shared.copyStatus = nil
        }
    }
}

private struct IrisDriveCopyToast: View {
    let message: String

    var body: some View {
        Text(message)
            .font(.callout.weight(.medium))
            .lineLimit(1)
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule())
            .shadow(radius: 10, y: 4)
    }
}

private func irisDriveCalDAVPort(_ url: String) -> String {
    URL(string: url)?.port.map(String.init) ?? "17321"
}

struct IrisDriveCopyButton: View {
    let title: String
    var copiedTitle = "Copied"
    var systemImage: String?
    var fillsWidth = false
    let action: () -> Void

    @State private var copied = false
    @State private var copyGeneration = 0

    var body: some View {
        Button {
            action()
            showCopied()
        } label: {
            ZStack {
                copyLabel(title, systemImage: systemImage)
                    .opacity(copied ? 0 : 1)
                copyLabel(copiedTitle, systemImage: "checkmark")
                    .opacity(copied ? 1 : 0)
            }
            .frame(
                maxWidth: fillsWidth ? .infinity : nil,
                minHeight: fillsWidth ? setupButtonMinHeight : nil
            )
            .contentShape(Rectangle())
        }
        .animation(.easeInOut(duration: 0.18), value: copied)
    }

    @ViewBuilder
    private func copyLabel(_ text: String, systemImage: String?) -> some View {
        if let systemImage {
            Label(text, systemImage: systemImage)
        } else {
            Text(text)
        }
    }

    private func showCopied() {
        copyGeneration += 1
        let generation = copyGeneration
        copied = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            guard generation == copyGeneration else { return }
            copied = false
        }
    }
}

func scanQRCodeFromImage(_ completion: @escaping (String) -> Void) {
    let panel = NSOpenPanel()
    panel.allowedContentTypes = [.image]
    panel.allowsMultipleSelection = false
    panel.canChooseDirectories = false
    panel.begin { response in
        guard response == .OK,
              let url = panel.url,
              let code = irisDriveQRCodePayload(from: url)
        else {
            return
        }
        completion(code)
    }
}

private func irisDriveQRCodePayload(from url: URL) -> String? {
    guard let image = CIImage(contentsOf: url) else {
        return nil
    }
    let request = VNDetectBarcodesRequest()
    request.symbologies = [.qr]
    let handler = VNImageRequestHandler(ciImage: image)
    do {
        try handler.perform([request])
    } catch {
        return nil
    }
    return request.results?
        .compactMap { $0.payloadStringValue?.trimmingCharacters(in: .whitespacesAndNewlines) }
        .first { !$0.isEmpty }
}

private struct IrisDriveQRCodeView: View {
    let value: String

    var body: some View {
        if let image = Self.makeImage(value) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(10)
                .background(Color.white)
                .clipShape(RoundedRectangle(cornerRadius: 8))
        } else {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color.secondary.opacity(0.15))
        }
    }

    private static func makeImage(_ value: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(value.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else {
            return nil
        }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
        let representation = NSCIImageRep(ciImage: scaled)
        let image = NSImage(size: representation.size)
        image.addRepresentation(representation)
        return image
    }
}

private let irisDriveTimestampFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
}()

private func irisDriveDateString(_ epoch: Int) -> String {
    irisDriveTimestampFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(epoch)))
}

private struct AccountKeyRow: View {
    let title: String
    let value: String?
    let copy: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 82, alignment: .leading)
            Text(value ?? "-")
                .font(.system(.callout, design: .monospaced))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            IrisDriveCopyButton(title: "Copy", systemImage: "doc.on.doc") {
                copy()
            }
            .disabled((value ?? "").isEmpty)
        }
    }
}

private struct MacRecoveryPhraseView: View {
    let payload: [String: Any]
    @Binding var wordIndex: Int
    @Environment(\.dismiss) private var dismiss

    private var error: String {
        payload["error"] as? String ?? ""
    }

    private var words: [String] {
        payload["words"] as? [String] ?? []
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Recovery phrase")
                    .font(.title2.weight(.semibold))
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Close")
            }

            if !error.isEmpty {
                Text(error)
                    .foregroundStyle(.secondary)
            } else if words.count == recoveryPhraseWordCount {
                Text("Word \(wordIndex + 1) of \(recoveryPhraseWordCount)")
                    .foregroundStyle(.secondary)
                Text(words[wordIndex])
                    .font(.largeTitle.monospaced().weight(.bold))
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 12)
                HStack {
                    Button {
                        wordIndex = max(0, wordIndex - 1)
                    } label: {
                        Label("Back", systemImage: "chevron.left")
                    }
                    .disabled(wordIndex == 0)
                    Spacer()
                    Button {
                        wordIndex = min(recoveryPhraseWordCount - 1, wordIndex + 1)
                    } label: {
                        Label("Next", systemImage: "chevron.right")
                    }
                    .disabled(wordIndex == recoveryPhraseWordCount - 1)
                }
            }
        }
        .padding(24)
        .frame(width: 420)
    }
}

private struct ShareStatusRow: View {
    let share: IrisDriveShareStatus
    let localProfileId: String?
    let driveLink: String?
    let onOpen: () -> Void
    let onOpenDriveLink: (String) -> Void
    let onInvite: () -> Void
    let onDelete: () -> Void
    let onRepair: () -> Void
    let onRevoke: (IrisDriveShareMemberStatus) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(shareDisplayName(share))
                        .font(.headline)
                    Text(shareSummary(share))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    onOpen()
                } label: {
                    Label("Open", systemImage: "folder")
                }
                if let driveLink = driveLink?.trimmingCharacters(in: .whitespacesAndNewlines),
                   !driveLink.isEmpty {
                    Button {
                        onOpenDriveLink(driveLink)
                    } label: {
                        Label("Open on drive.iris.to", systemImage: "safari")
                    }
                }
                if share.canAdmin {
                    Button {
                        onInvite()
                    } label: {
                        Label("Invite", systemImage: "person.badge.plus")
                    }
                }
                if share.repairNeeded || !share.missingKeyWraps.isEmpty {
                    Button {
                        onRepair()
                    } label: {
                        Label("Repair", systemImage: "arrow.triangle.2.circlepath")
                    }
                }
                Button(role: .destructive) {
                    onDelete()
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
            ForEach(share.members) { member in
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(shareMemberDisplayName(member))
                        Text(shareMemberSummary(member))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if share.canAdmin,
                       member.status != "revoked",
                       member.profileId != localProfileId {
                        Button(role: .destructive) {
                            onRevoke(member)
                        } label: {
                            Label("Revoke", systemImage: "trash")
                        }
                    }
                }
            }
            ForEach(share.pendingInvites) { invite in
                VStack(alignment: .leading, spacing: 2) {
                    Text(pendingShareInviteDisplayName(invite))
                    Text(pendingShareInviteSummary(invite))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct InviteShareMemberSheet: View {
    let controller: AppDelegate
    @ObservedObject var status: IrisDriveStatus
    let share: IrisDriveShareStatus
    @Environment(\.dismiss) private var dismiss
    @State private var role = "reader"
    @State private var representativeNpubHint = ""
    @State private var displayName = ""
    @State private var showManualInvite = false

    init(
        controller: AppDelegate,
        status: IrisDriveStatus,
        share: IrisDriveShareStatus,
        profileId: String = "",
        representativeNpubHint: String = "",
        displayName: String = ""
    ) {
        self.controller = controller
        self.status = status
        self.share = share
        _representativeNpubHint = State(initialValue: representativeNpubHint)
        _displayName = State(initialValue: displayName)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Invite to \(shareDisplayName(share))")
                .font(.title3.weight(.semibold))
            if !viewLink.isEmpty {
                Text("Anyone with link")
                    .font(.headline)
                IrisDriveQRCodeView(value: viewLink)
                    .frame(width: 200, height: 200)
                    .frame(maxWidth: .infinity, alignment: .center)
                IrisDriveCopyButton(title: "Copy view link", systemImage: "link") {
                    irisDriveCopyToPasteboard(viewLink, feedback: "View link copied")
                }
            }
            Divider()
            Button {
                showManualInvite.toggle()
            } label: {
                Label("Invite specific user", systemImage: "person.badge.plus")
            }
            .buttonStyle(.bordered)
            if showManualInvite {
                TextField("Paste their User ID", text: $representativeNpubHint)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                HStack(spacing: 8) {
                    Button {
                        representativeNpubHint = NSPasteboard.general
                            .string(forType: .string)?
                            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    } label: {
                        Label("Paste User ID", systemImage: "doc.on.clipboard")
                    }
                    Button {
                        scanQRCodeFromImage { code in
                            representativeNpubHint = code
                        }
                    } label: {
                        Label("Scan QR", systemImage: "qrcode.viewfinder")
                    }
                }
                TextField("Name (optional)", text: $displayName)
                    .textFieldStyle(.roundedBorder)
                Picker("Access", selection: $role) {
                    Text("View").tag("reader")
                    Text("Edit").tag("editor")
                    Text("Manage").tag("admin")
                }
                .pickerStyle(.segmented)
            }
            HStack {
                Spacer()
                Button("Cancel") {
                    dismiss()
                }
                Button(showManualInvite ? "Add User" : "Done") {
                    if showManualInvite {
                        controller.recordPendingShareInvite(
                            shareId: share.shareId,
                            representativeNpubHint: representativeNpubHint,
                            role: role,
                            displayName: displayName
                        )
                    }
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(showManualInvite && !canSubmitInvite)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private var viewLink: String {
        shareDriveLink(share, status: status) ?? ""
    }

    private var canSubmitInvite: Bool {
        !representativeNpubHint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

private struct MyShareNpubSheet: View {
    let npub: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("My User ID")
                .font(.title3.weight(.semibold))
            IrisDriveQRCodeView(value: npub)
                .frame(width: 220, height: 220)
                .frame(maxWidth: .infinity, alignment: .center)
            Text(npub)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .lineLimit(4)
            HStack {
                Spacer()
                IrisDriveCopyButton(title: "Copy", systemImage: "doc.on.doc") {
                    irisDriveCopyToPasteboard(npub, feedback: "User ID copied")
                }
                Button("Done") {
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
        .frame(width: 420)
    }
}

private func shareDisplayName(_ share: IrisDriveShareStatus) -> String {
    share.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Shared folder"
        : share.displayName
}

private func shareSummary(_ share: IrisDriveShareStatus) -> String {
    let displayPath = share.sourcePath
        .trimmingCharacters(in: .whitespacesAndNewlines)
    return [
        share.roleLabel.isEmpty ? share.role : share.roleLabel,
        sharePeopleCount(share.participantCount),
        displayPath.isEmpty
            ? nil
            : shortValue(displayPath),
    ].compactMap { $0 }.joined(separator: " | ")
}

private func shareOpenPath(_ share: IrisDriveShareStatus) -> String {
    share.sourcePath
}

private func sharePeopleCount(_ count: Int) -> String {
    "\(count) \(count == 1 ? "person" : "people")"
}

private func shareProviderSignalPaths(_ share: IrisDriveShareStatus) -> [String] {
    ([share.sourcePath] + share.shortcutPaths)
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
}

private func shareDriveLink(_ share: IrisDriveShareStatus, status: IrisDriveStatus) -> String? {
    guard let configDirectory = status.configDirectory?.trimmingCharacters(in: .whitespacesAndNewlines),
          !configDirectory.isEmpty
    else {
        return nil
    }
    let provider = IrisDriveDesktopCore.providerList(dataDir: configDirectory)
    let entries = provider["entries"] as? [[String: Any]] ?? []
    let paths = shareLinkCandidatePaths(share)
    for path in paths {
        guard let entry = entries.first(where: { $0["path"] as? String == path }),
              let version = entry["version"] as? String,
              !version.isEmpty
        else {
            continue
        }
        let payload = IrisDriveDesktopCore.driveLinkForCid(version)
        let url = payload["url"] as? String ?? ""
        if !url.isEmpty {
            return url
        }
    }
    return nil
}

private func shareLinkCandidatePaths(_ share: IrisDriveShareStatus) -> [String] {
    ([share.sourcePath, share.sharedWithMePath] + share.shortcutPaths)
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
}

private func openDriveIrisLink(_ link: String) {
    guard let url = URL(string: link.trimmingCharacters(in: .whitespacesAndNewlines)) else {
        NSSound.beep()
        return
    }
    NSWorkspace.shared.open(url)
}

private func shareMemberDisplayName(_ member: IrisDriveShareMemberStatus) -> String {
    member.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "User"
        : member.displayName
}

private func shareMemberSummary(_ member: IrisDriveShareMemberStatus) -> String {
    [
        member.roleLabel.isEmpty ? member.role : member.roleLabel,
        member.statusLabel.isEmpty ? member.status : member.statusLabel,
        shortValue(
            member.representativeNpubHint.isEmpty
                ? member.profileId
                : member.representativeNpubHint
        ),
    ].joined(separator: " | ")
}

private func pendingShareInviteDisplayName(_ invite: IrisDrivePendingShareInviteStatus) -> String {
    invite.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Pending contact"
        : invite.displayName
}

private func pendingShareInviteSummary(_ invite: IrisDrivePendingShareInviteStatus) -> String {
    [
        invite.roleLabel.isEmpty ? invite.role : invite.roleLabel,
        invite.statusLabel.isEmpty ? invite.status : invite.statusLabel,
        shortValue(invite.representativeNpubHint),
    ].joined(separator: " | ")
}

private struct AccountInfoRow: View {
    let title: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 82, alignment: .leading)
            Text(value)
                .font(.system(.callout, design: .monospaced))
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

private struct SidebarRow: View {
    let symbol: String
    let title: String
    var selected = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 7) {
                Image(systemName: symbol)
                    .frame(width: 16)
                Text(title)
                Spacer(minLength: 0)
            }
            .font(.callout.weight(selected ? .semibold : .regular))
            .padding(.vertical, 6)
            .padding(.horizontal, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            selected
                ? Color(nsColor: .selectedContentBackgroundColor).opacity(0.18)
                : .clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .contentShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct SectionTitle: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(.headline)
    }
}

struct DetailRow: View {
    let label: String
    let value: String
    var copyable = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 110, alignment: .leading)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: .infinity, alignment: .leading)
            if copyable {
                IrisDriveCopyButton(title: "Copy", systemImage: "doc.on.doc") {
                    irisDriveCopyToPasteboard(value)
                }
                .buttonStyle(.borderless)
                .font(.caption)
            }
        }
    }
}

private struct PeerRow: View {
    let peer: IrisDrivePeerStatus
    let canManageDevices: Bool
    let adminCount: Int
    let controller: AppDelegate
    @State private var expanded = false
    @State private var showDeleteConfirmation = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 12) {
                    if peer.isDeviceActor {
                        Circle()
                            .fill(
                                peerOnlineForDisplay ? Color.green : Color.secondary.opacity(0.5)
                            )
                            .frame(width: 8, height: 8)
                    }
                    Image(
                        systemName: peer.role == "recovery"
                            ? "key.fill"
                            : (peer.isCurrentDevice ? "desktopcomputer" : "laptopcomputer")
                    )
                        .frame(width: 24)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title)
                            .font(.callout.weight(.medium))
                            .lineLimit(1)
                        Text(subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    DetailRow(label: "Public key", value: peer.npub, copyable: true)
                    DetailRow(label: "Role", value: peer.roleLabel)
                    if let root = peer.rootCID {
                        DetailRow(label: "Root", value: root, copyable: true)
                    }
                    if let generation = peer.dckGeneration {
                        DetailRow(label: "Key generation", value: "\(generation)")
                    }
                    if let published = peer.publishedAt {
                        DetailRow(label: "Updated", value: irisDriveDateString(published))
                    }
                    if canManagePeer {
                        HStack(spacing: 8) {
                            if peer.role == "admin" {
                                if adminCount > 1 {
                                    Button {
                                        controller.demoteAdmin(peer.npub)
                                    } label: {
                                        Label("Remove admin", systemImage: "person.badge.minus")
                                    }
                                }
                            } else {
                                Button {
                                    controller.appointAdmin(peer.npub)
                                } label: {
                                    Label("Make admin", systemImage: "person.badge.key")
                                }
                            }
                            Button(role: .destructive) {
                                showDeleteConfirmation = true
                            } label: {
                                Label("Remove", systemImage: "trash")
                            }
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .padding(.top, 12)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .confirmationDialog(
            "Remove \(title)?",
            isPresented: $showDeleteConfirmation,
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                controller.deleteDevice(peer.npub)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the device from Iris Drive and rotates access keys.")
        }
    }

    private var title: String {
        peer.displayLabel
    }

    private var subtitle: String {
        if !peer.isDeviceActor {
            return [peer.roleLabel, peer.stateLabel]
                .filter { !$0.isEmpty }
                .joined(separator: " | ")
        }
        if peer.isCurrentDevice {
            return "\(connectionLabelForDisplay) | \(peer.roleLabel)"
        }
        return [peer.roleLabel, connectionLabelForDisplay].joined(separator: " | ")
    }

    private var canManagePeer: Bool {
        canManageDevices && !peer.isCurrentDevice && peer.role != "recovery"
    }

    private var peerOnlineForDisplay: Bool {
        peer.isCurrentDevice || peer.fipsOnline
    }

    private var connectionLabelForDisplay: String {
        peer.isCurrentDevice ? "This Device" : peer.connectionLabel
    }

}

private struct AppKeyLinkRequestRow: View {
    let request: IrisDriveAppKeyLinkRequestStatus
    let controller: AppDelegate
    @State private var approvalError = ""
    @State private var approvalPending = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Image(systemName: "iphone.gen3")
                    .frame(width: 24)
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 3) {
                    Text(request.label?.isEmpty == false ? request.label! : "New Device")
                        .font(.callout.weight(.medium))
                        .lineLimit(1)
                    Text(request.deviceNpub)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer()
                if approvalPending {
                    ProgressView()
                        .controlSize(.small)
                        .frame(width: 18, height: 18)
                }
                Button(role: .destructive) {
                    controller.rejectDevice(request.requestURL)
                } label: {
                    Label("Reject", systemImage: "xmark")
                }
                .disabled(approvalPending)
                Button {
                    approvalPending = true
                    approvalError = ""
                    controller.approveDevice(request.requestURL, label: request.label ?? "") { result in
                        approvalPending = false
                        if case let .failure(error) = result {
                            approvalError = error.localizedDescription
                        }
                    }
                } label: {
                    Label(approvalPending ? "Adding" : "Add", systemImage: "checkmark")
                }
                .disabled(approvalPending)
            }
            if !approvalError.isEmpty {
                Text(approvalError)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct EndpointGroup: View {
    let title: String
    let values: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            if values.isEmpty {
                Text("None")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(values, id: \.self) { value in
                    Text(value)
                        .font(.system(.callout, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                }
            }
        }
    }
}

private struct FipsDiagnostics: View {
    let status: IrisDriveFipsStatus

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("Connectivity")
                .font(.caption)
                .foregroundStyle(.secondary)
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 110), alignment: .leading)],
                alignment: .leading,
                spacing: 8
            ) {
                NetworkMetric(title: "State", value: status.stateLabel)
                NetworkMetric(title: "Roster", value: status.rosterLabel)
                NetworkMetric(title: "Other", value: "\(status.otherPeerCount)")
                NetworkMetric(title: "Direct", value: "\(status.directDeviceCount)")
                NetworkMetric(title: "Mesh", value: "\(status.meshDeviceCount)")
            }
            if let endpoint = status.endpointNpub, !endpoint.isEmpty {
                Text(endpoint)
                    .font(.system(.callout, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            if let scope = status.discoveryScope, !scope.isEmpty {
                Text(scope)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            ForEach(status.peerStatuses) { peer in
                HStack(spacing: 8) {
                    Text(shortValue(peer.npub))
                        .font(.system(.caption, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(peer.connectionLabel)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .textSelection(.enabled)
            }
            if let error = status.error, !error.isEmpty {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .lineLimit(2)
                    .truncationMode(.tail)
            }
        }
    }
}

private struct NetworkMetric: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.callout.weight(.medium))
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}
