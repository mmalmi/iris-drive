import AppKit
import CoreImage.CIFilterBuiltins
import SwiftUI

private enum IrisDrivePanelTab: String, CaseIterable, Identifiable {
    case drive
    case peers
    case shares
    case backups
    case settings

    var id: Self { self }

    var title: String {
        switch self {
        case .drive:
            return "My Drive"
        case .peers:
            return "AppKeys"
        case .shares:
            return "Shares"
        case .backups:
            return "Backups"
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
        case .backups:
            return "lock.shield.fill"
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
        case "backup", "backups":
            return .backups
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
    @State private var backupInput = ""
    @State private var backupLabel = ""
    @State private var shareSourceInput = ""
    @State private var shareNameInput = ""
    @State private var shareInviteInput = ""
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
    @State private var showAddDevice = false
    @State private var showAddBackup = false
    @State private var inviteShare: IrisDriveShareStatus?
    @State private var revokeShareMember: ShareMemberRevokeTarget?
    @State private var checkingAllBackups = false
    @State private var showLogoutConfirmation = false
    @State private var recoveryExport: [String: Any]?
    @State private var recoveryExportWordIndex = 0

    var body: some View {
        Group {
            if !status.setupComplete {
                setup
            } else {
                controlPanel
            }
        }
        .onAppear {
            controller.ensureFileProviderDomainIfProfileExists()
        }
    }

    private var controlPanel: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            content
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
        case .backups:
            page { backups }
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
                    setupButtonLabel("Link app install", systemImage: "desktopcomputer")
                }
                .accessibilityLabel("Link app install")
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
            setupForm(title: "Link app install", backTarget: .restoreOptions) {
                TextField("IrisProfile invite link or admin AppKey", text: $setupLinkTarget)
                    .accessibilityLabel("IrisProfile invite link or admin AppKey")
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
                setupSubmit("Link app install") {
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
        return [
            countLabel(files, "file"),
            "\(byteString(usedBytes)) used",
            "\(status.onlineDeviceCount)/\(status.authorizedDeviceCount) online",
        ].joined(separator: "  ·  ")
    }

    // MARK: AppKeys

    private var peers: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("AppKeys")
                Spacer()
                if status.canAdminProfile {
                    Button {
                        showAddDevice = true
                    } label: {
                        Label("Add AppKey", systemImage: "plus")
                    }
                }
            }
            if status.peers.isEmpty {
                emptyState("No AppKeys yet")
            } else {
                let adminCount = status.peers.filter { $0.role == "admin" }.count
                ForEach(status.peers) { peer in
                    PeerRow(
                        peer: peer,
                        canManageDevices: status.canAdminProfile,
                        adminCount: adminCount,
                        controller: controller
                    )
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
    }

    private var addDeviceSheet: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Add an AppKey")
                .font(.title3.weight(.semibold))
            if let invite = status.appKeyLinkInviteURL, !invite.isEmpty {
                Text("Invite app install")
                    .font(.headline)
                IrisDriveQRCodeView(value: invite)
                    .frame(width: 220, height: 220)
                    .frame(maxWidth: .infinity, alignment: .center)
                Button {
                    irisDriveCopyToPasteboard(invite)
                } label: {
                    Label("Copy invite link", systemImage: "link")
                }
                Button {
                    controller.resetInvite()
                } label: {
                    Label("Reset invite", systemImage: "arrow.clockwise")
                }
            }
            if !status.inboundAppKeyLinkRequests.isEmpty {
                Text("AppKey requests")
                    .font(.headline)
                ForEach(status.inboundAppKeyLinkRequests) { request in
                    AppKeyLinkRequestRow(request: request, controller: controller)
                }
            }
            Text("Paste the AppKey shown by the app install you want to approve.")
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField("AppKey", text: $approveDeviceKey)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
                .onChange(of: approveDeviceKey) { _, newValue in
                    refreshApproveAppKeyLinkInput(newValue)
                }
                .onAppear {
                    refreshApproveAppKeyLinkInput(approveDeviceKey)
                }
            TextField("Name (optional)", text: $approveDeviceLabel)
                .textFieldStyle(.roundedBorder)
            HStack {
                Spacer()
                Button("Cancel") {
                    showAddDevice = false
                }
                Button("Add") {
                    controller.approveDevice(approveDeviceKey, label: approveDeviceLabel)
                    approveDeviceKey = ""
                    approveDeviceKeyIsComplete = false
                    approveDeviceLabel = ""
                    showAddDevice = false
                }
                .buttonStyle(.borderedProminent)
                .disabled(!approveDeviceKeyIsComplete)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    // MARK: Shares

    private var shares: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Shares")
                Spacer()
                if let invite = status.lastShareInviteURL {
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(invite, forType: .string)
                    } label: {
                        Label("Copy Invite", systemImage: "doc.on.doc")
                    }
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Create Share")
                    .font(.headline)
                TextField("Folder path", text: $shareSourceInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                TextField("Name (optional)", text: $shareNameInput)
                    .textFieldStyle(.roundedBorder)
                Button {
                    controller.createShare(
                        sourcePath: shareSourceInput,
                        displayName: shareNameInput
                    )
                    shareSourceInput = ""
                    shareNameInput = ""
                } label: {
                    Label("Create Share", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .disabled(shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Accept Invite")
                    .font(.headline)
                TextField("Share invite", text: $shareInviteInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                Button {
                    controller.acceptShareInvite(shareInviteInput)
                    shareInviteInput = ""
                } label: {
                    Label("Accept Invite", systemImage: "tray.and.arrow.down.fill")
                }
                .disabled(shareInviteInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            if status.shares.isEmpty {
                emptyState("No shared folders")
            } else {
                ForEach(status.shares) { share in
                    ShareStatusRow(
                        share: share,
                        localProfileId: status.profileId,
                        onInvite: { inviteShare = share },
                        onRepair: { controller.repairShareWraps(shareId: share.shareId) },
                        onShortcut: {
                            controller.addShareShortcut(
                                shareId: share.shareId,
                                displayName: shareDisplayName(share)
                            )
                        },
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
            InviteShareMemberSheet(controller: controller, share: share)
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

    // MARK: Backups

    private var backups: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Backups")
                Spacer()
                Button {
                    guard !checkingAllBackups else { return }
                    checkingAllBackups = true
                    controller.checkBackups {
                        checkingAllBackups = false
                    }
                } label: {
                    Label(checkingAllBackups ? "Checking..." : "Check All", systemImage: "checkmark.shield")
                }
                .disabled(status.backupTargets.isEmpty || checkingAllBackups)
                Button {
                    controller.syncBackups()
                } label: {
                    Label("Sync Now", systemImage: "arrow.up.circle")
                }
                .disabled(status.backupTargets.isEmpty)
                Button {
                    showAddBackup = true
                } label: {
                    Label("Add Backup", systemImage: "plus")
                }
            }
            if status.backupTargets.isEmpty {
                emptyState("No backups yet")
            } else {
                ForEach(status.backupTargets) { target in
                    BackupTargetRow(
                        target: target,
                        onCheck: { completion in
                            controller.checkBackupTarget(target, completion: completion)
                        },
                        onRemove: {
                            controller.removeBackupTarget(target)
                        }
                    )
                }
            }
        }
        .sheet(isPresented: $showAddBackup) {
            addBackupSheet
        }
    }

    private var addBackupSheet: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Add a backup")
                .font(.title3.weight(.semibold))
            TextField("Destination", text: $backupInput)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
            TextField("Name (optional)", text: $backupLabel)
                .textFieldStyle(.roundedBorder)
            Text("A web address, another AppKey (npub…), or a local path (fs:/…, lmdb:/…).")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel") {
                    showAddBackup = false
                }
                Button("Add") {
                    addBackupFromInput()
                    showAddBackup = false
                }
                .buttonStyle(.borderedProminent)
                .disabled(backupInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 440)
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
                    "nhash.iris.localhost resolver",
                    isOn: Binding(
                        get: { status.localNhashResolverEnabled },
                        set: { controller.setLocalNhashResolver($0) }
                    )
                )
            }

            Section("Account") {
                AccountKeyRow(title: "AppKey", value: status.currentAppKeyNpub) {
                    controller.copyAppKey()
                }
                AccountKeyRow(title: "Current AppKey", value: status.deviceNpub) {
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
                EndpointGroup(title: "Blossom", values: status.blossomServers)
                FipsDiagnostics(status: status.fips)
            }

            Section("Sync & advanced") {
                if status.daemonRunning {
                    Button("Pause sync") { controller.stopSync() }
                } else {
                    Button("Resume sync") { controller.startSync() }
                }
                Button("Copy drive.iris.to link") { controller.copyDriveLink() }
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
                LabeledContent("Version", value: appVersion)
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

    private func addBackupFromInput() {
        let value = backupInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        controller.addBackupTarget(value, label: backupLabel)
        backupInput = ""
        backupLabel = ""
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
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(value, forType: .string)
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
            Button(action: copy) {
                Label("Copy", systemImage: "doc.on.doc")
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

    private var phrase: String {
        payload["recovery_phrase"] as? String ?? ""
    }

    private var secretKey: String {
        payload["secret_key"] as? String ?? ""
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
                HStack {
                    Button {
                        copy(phrase)
                    } label: {
                        Label("Copy recovery phrase", systemImage: "doc.on.doc")
                    }
                    Button {
                        copy(secretKey)
                    } label: {
                        Label("Copy secret key", systemImage: "key")
                    }
                }
            }
        }
        .padding(24)
        .frame(width: 420)
    }

    private func copy(_ value: String) {
        guard !value.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
    }
}

private struct ShareStatusRow: View {
    let share: IrisDriveShareStatus
    let localProfileId: String?
    let onInvite: () -> Void
    let onRepair: () -> Void
    let onShortcut: () -> Void
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
                if share.shortcutPaths.isEmpty {
                    Button {
                        onShortcut()
                    } label: {
                        Label("Shortcut", systemImage: "link")
                    }
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
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct InviteShareMemberSheet: View {
    let controller: AppDelegate
    let share: IrisDriveShareStatus
    @Environment(\.dismiss) private var dismiss
    @State private var profileId = ""
    @State private var appKey = ""
    @State private var role = "reader"
    @State private var representativeNpubHint = ""
    @State private var displayName = ""
    @State private var label = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Invite to \(shareDisplayName(share))")
                .font(.title3.weight(.semibold))
            TextField("IrisProfile UUID", text: $profileId)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
            TextField("Recipient AppKey", text: $appKey)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
            Picker("Role", selection: $role) {
                Text("Reader").tag("reader")
                Text("Editor").tag("editor")
                Text("Admin").tag("admin")
            }
            .pickerStyle(.segmented)
            TextField("Representative npub", text: $representativeNpubHint)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
            TextField("Name", text: $displayName)
                .textFieldStyle(.roundedBorder)
            TextField("AppKey label", text: $label)
                .textFieldStyle(.roundedBorder)
            HStack {
                Spacer()
                Button("Cancel") {
                    dismiss()
                }
                Button("Invite") {
                    controller.inviteShareMember(
                        shareId: share.shareId,
                        profileId: profileId,
                        appKey: appKey,
                        role: role,
                        representativeNpubHint: representativeNpubHint,
                        displayName: displayName,
                        label: label
                    )
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(
                    profileId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                        appKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                )
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

private func shareDisplayName(_ share: IrisDriveShareStatus) -> String {
    share.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Shared folder"
        : share.displayName
}

private func shareSummary(_ share: IrisDriveShareStatus) -> String {
    [
        share.roleLabel.isEmpty ? share.role : share.roleLabel,
        share.keyStatusLabel.isEmpty ? share.keyStatus : share.keyStatusLabel,
        "\(share.participantCount) people",
        share.shortcutPaths.first.map { "shortcut \(shortValue($0))" },
    ].compactMap { $0 }.joined(separator: " | ")
}

private func shareMemberDisplayName(_ member: IrisDriveShareMemberStatus) -> String {
    member.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "IrisProfile"
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
                Button {
                    irisDriveCopyToPasteboard(value)
                } label: {
                    Image(systemName: "doc.on.doc")
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
                    Circle()
                        .fill(
                            peer.fipsOnline ? Color.green : Color.secondary.opacity(0.5)
                        )
                        .frame(width: 8, height: 8)
                    Image(systemName: peer.isCurrentDevice ? "desktopcomputer" : "laptopcomputer")
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
                    DetailRow(label: "Visibility", value: privacy)
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
            Text("This removes the AppKey from Iris Drive and rotates access keys.")
        }
    }

    private var title: String {
        peer.displayLabel
    }

    private var subtitle: String {
        if peer.isCurrentDevice {
            return "\(peer.connectionLabel) | \(peer.roleLabel)"
        }
        return [peer.roleLabel, peer.connectionLabel].joined(separator: " | ")
    }

    private var privacy: String {
        guard peer.hasRoot else {
            if peer.isCurrentDevice {
                return "Local"
            }
            return "Pending"
        }
        return peer.rootIsPrivate == false ? "Public" : "Private"
    }

    private var canManagePeer: Bool {
        canManageDevices && !peer.isCurrentDevice
    }

}

private struct AppKeyLinkRequestRow: View {
    let request: IrisDriveAppKeyLinkRequestStatus
    let controller: AppDelegate

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "iphone.gen3")
                .frame(width: 24)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 3) {
                Text(request.label?.isEmpty == false ? request.label! : "New AppKey")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                Text(request.deviceNpub)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer()
            Button(role: .destructive) {
                controller.rejectDevice(request.requestURL)
            } label: {
                Label("Reject", systemImage: "xmark")
            }
            Button {
                controller.approveDevice(request.requestURL, label: request.label ?? "")
            } label: {
                Label("Add", systemImage: "checkmark")
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
