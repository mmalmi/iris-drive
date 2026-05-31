import AppKit
import CoreImage.CIFilterBuiltins
import SwiftUI

private enum IrisDrivePanelTab: String, CaseIterable, Identifiable {
    case drive
    case peers
    case backups
    case settings

    var id: Self { self }

    var title: String {
        switch self {
        case .drive:
            return "My Drive"
        case .peers:
            return "Devices"
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
        case .backups:
            return "lock.shield.fill"
        case .settings:
            return "gearshape.fill"
        }
    }
}

private enum IrisDriveSetupMode {
    case welcome
    case create
    case createPhoto
    case restore
    case link
}

private enum IrisDriveSyncState {
    case upToDate
    case syncing(Int, Int)
    case paused
    case attention
}

private let setupControlWidth: CGFloat = 340
let setupButtonMinHeight: CGFloat = 44

struct IrisDriveControlPanel: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate
    @State private var selectedTab = IrisDrivePanelTab.drive
    @State private var relayInput = ""
    @State private var backupInput = ""
    @State private var backupLabel = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""
    @State private var setupMode = IrisDriveSetupMode.welcome
    @State private var setupUsername = ""
    @State private var setupPhotoPath = ""
    @State private var setupSecret = ""
    @State private var setupOwner = ""
    @State var submittedSetupOwner = ""
    @State private var approveDeviceKey = ""
    @State private var approveDeviceLabel = ""
    @State private var showAddDevice = false
    @State private var showAddBackup = false
    @State private var showLogoutConfirmation = false

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
                    setupMode = .restore
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
        case .restore:
            setupForm(title: "Sign in") {
                SecureField("Secret key", text: $setupSecret)
                    .onSubmit {
                        controller.restoreProfile(secretKey: setupSecret)
                    }
                setupSubmit("Sign in") {
                    controller.restoreProfile(secretKey: setupSecret)
                }
                .disabled(setupSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    setupMode = .link
                } label: {
                    setupButtonLabel("Link this device", systemImage: "desktopcomputer")
                }
                .accessibilityLabel("Link this device")
                .buttonStyle(.bordered)
            }
        case .link:
            setupForm(title: "Link this device") {
                TextField("Owner public key or invite link", text: $setupOwner)
                    .accessibilityLabel("Owner public key or invite link")
                    .onSubmit {
                        submitSetupOwner(setupOwner, force: true)
                    }
                    .onChange(of: setupOwner) { _, newValue in
                        submitSetupOwner(newValue, force: false)
                    }
                setupSubmit("Link device") {
                    submitSetupOwner(setupOwner, force: true)
                }
                .disabled(setupOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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
        if !status.daemonRunning {
            return .paused
        }
        let message = status.message.lowercased()
        if message.contains("attention") || message.contains("failed") {
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
        case .upToDate:
            return "Up to date"
        case let .syncing(done, total):
            return "Syncing \(done) of \(total)…"
        case .paused:
            return "Paused"
        case .attention:
            return "Needs attention"
        }
    }

    private var summaryLine: String {
        let files = status.fileCount ?? status.topLevelEntries ?? 0
        let usedBytes = status.visibleFileBytes ?? status.localBlockBytes
        return [
            countLabel(files, "file"),
            "\(byteString(usedBytes)) used",
            countLabel(status.authorizedDeviceCount, "device"),
        ].joined(separator: "  ·  ")
    }

    // MARK: Devices

    private var peers: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Devices")
                Spacer()
                if status.hasOwnerSigningAuthority {
                    Button {
                        showAddDevice = true
                    } label: {
                        Label("Add Device", systemImage: "plus")
                    }
                }
            }
            if status.peers.isEmpty {
                emptyState("No devices yet")
            } else {
                let adminCount = status.peers.filter { $0.role == "admin" }.count
                ForEach(status.peers) { peer in
                    PeerRow(
                        peer: peer,
                        canManageDevices: status.hasOwnerSigningAuthority,
                        adminCount: adminCount,
                        controller: controller
                    )
                }
            }
            if !status.inboundDeviceLinkRequests.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Requests")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(status.inboundDeviceLinkRequests) { request in
                        DeviceLinkRequestRow(request: request, controller: controller)
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
            Text("Add a device")
                .font(.title3.weight(.semibold))
            if let invite = status.deviceLinkInviteURL, !invite.isEmpty {
                Text("Invite device")
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
            if !status.inboundDeviceLinkRequests.isEmpty {
                Text("Device requests")
                    .font(.headline)
                ForEach(status.inboundDeviceLinkRequests) { request in
                    DeviceLinkRequestRow(request: request, controller: controller)
                }
            }
            Text("Paste the Device ID shown on the other device when you link it manually.")
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField("Device ID", text: $approveDeviceKey)
                .textFieldStyle(.roundedBorder)
                .disableAutocorrection(true)
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
                    approveDeviceLabel = ""
                    showAddDevice = false
                }
                .buttonStyle(.borderedProminent)
                .disabled(approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    // MARK: Backups

    private var backups: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                SectionTitle("Backups")
                Spacer()
                Button {
                    controller.checkBackups()
                } label: {
                    Label("Check", systemImage: "checkmark.shield")
                }
                .disabled(status.backupTargets.isEmpty)
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
                    BackupTargetRow(target: target)
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
            Text("A web address, another device (npub…), or a local path (fs:/…, lmdb:/…).")
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
                AccountKeyRow(title: "Owner", value: status.ownerNpub) {
                    controller.copyOwnerKey()
                }
                AccountKeyRow(title: "This device", value: status.deviceNpub) {
                    controller.copyDeviceKey()
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
                    value: byteString(status.visibleFileBytes ?? status.localBlockBytes)
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
        let byURL = status.relayStatuses.reduce(into: [String: IrisDriveRelayStatus]()) {
            partial, relay in
            partial[relay.url] = relay
        }
        return status.relays.map { relay in
            byURL[relay] ?? IrisDriveRelayStatus(url: relay, status: "configured")
        }
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
                    .fill(relayStatusColor(relay.status))
                    .frame(width: 8, height: 8)
                Text(relay.url)
                    .font(.system(.body, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 8)
                Text(relayStatusLabel(relay.status))
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

    private func relayStatusColor(_ status: String) -> Color {
        switch status {
        case "connected":
            return .green
        case "connecting":
            return .yellow
        case "blocked", "offline", "terminated":
            return .red.opacity(0.85)
        default:
            return .secondary.opacity(0.65)
        }
    }

    private func relayStatusLabel(_ status: String) -> String {
        status == "configured" ? "saved" : status
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

private struct DetailRow: View {
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
                    DetailRow(label: "Role", value: roleLabel)
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
                                        Label("Remove Admin", systemImage: "person.badge.minus")
                                    }
                                }
                            } else {
                                Button {
                                    controller.appointAdmin(peer.npub)
                                } label: {
                                    Label("Make Admin", systemImage: "person.badge.key")
                                }
                            }
                            Button(role: .destructive) {
                                showDeleteConfirmation = true
                            } label: {
                                Label("Delete", systemImage: "trash")
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
            "Delete \(title)?",
            isPresented: $showDeleteConfirmation,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                controller.deleteDevice(peer.npub)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the device from Iris Drive and rotates access keys.")
        }
    }

    private var title: String {
        peer.label ?? (peer.isCurrentDevice ? "This Mac" : peer.npub)
    }

    private var subtitle: String {
        if peer.isCurrentDevice {
            return "This device | \(roleLabel)"
        }
        return [roleLabel, fipsConnectionLabel].joined(separator: " | ")
    }

    private var fipsConnectionLabel: String {
        guard peer.fipsOnline else {
            return "Offline"
        }
        let via = peer.fipsOnlineVia ?? "direct"
        let transport = peer.fipsTransportType?.uppercased()
        let latency = peer.fipsSrttMS.map { "\($0) ms" }
        switch (transport, latency, via) {
        case let (transport?, latency?, _):
            return "Online (\(transport), \(latency))"
        case let (transport?, nil, _):
            return "Online (\(transport))"
        case let (nil, latency?, _):
            return "Online (\(latency))"
        case (_, _, "mesh"):
            return "Online (Mesh)"
        default:
            return "Online"
        }
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

    private var roleLabel: String {
        if !peer.authorized {
            switch peer.authorizationState {
            case "revoked":
                return "Revoked"
            case "awaiting_approval":
                return "Awaiting approval"
            default:
                return "Unavailable"
            }
        }
        return peer.role == "admin" ? "Admin" : "Member"
    }
}

private struct DeviceLinkRequestRow: View {
    let request: IrisDriveDeviceLinkRequestStatus
    let controller: AppDelegate

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "iphone.gen3")
                .frame(width: 24)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 3) {
                Text(request.label?.isEmpty == false ? request.label! : "New device")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                Text(request.deviceNpub)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer()
            Button {
                controller.approveDevice(request.requestURL, label: request.label ?? "")
            } label: {
                Label("Approve", systemImage: "checkmark")
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct BackupTargetRow: View {
    let target: IrisDriveBackupTarget
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 12) {
                    Image(systemName: target.iconName)
                        .frame(width: 24)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(target.title)
                            .font(.callout.weight(.medium))
                            .lineLimit(1)
                        Text(statusLine)
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
                    DetailRow(label: "Destination", value: target.target, copyable: true)
                    if let uploaded = target.uploaded, let total = target.totalHashes {
                        DetailRow(label: "Progress", value: "\(uploaded)/\(total)")
                    }
                    if let checkState = target.checkState {
                        DetailRow(label: "Check", value: checkState)
                    }
                    if let latencyMs = target.latencyMs {
                        DetailRow(label: "Latency", value: "\(latencyMs) ms")
                    }
                    if let bandwidth = target.downloadBytesPerSecond {
                        DetailRow(
                            label: "Bandwidth",
                            value: "\(ByteCountFormatter.string(fromByteCount: Int64(bandwidth), countStyle: .file))/s"
                        )
                    }
                    if let sampled = target.sampledHashes {
                        DetailRow(
                            label: "Sample",
                            value: "\(sampled) checked, \(target.missing ?? 0) missing, \(target.unknown ?? 0) unknown"
                        )
                    }
                }
                .padding(.top, 12)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var statusLine: String {
        if let uploaded = target.uploaded,
           let total = target.totalHashes,
           total > 0,
           uploaded < total {
            return "Syncing \(Int(Double(uploaded) / Double(total) * 100))%"
        }
        switch target.state.lowercased() {
        case "synced":
            if target.checkState?.lowercased() == "verified" {
                return "Up to date | verified"
            }
            return "Up to date"
        default:
            return target.state
        }
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
                NetworkMetric(title: "State", value: status.stateText)
                NetworkMetric(title: "Roster", value: status.rosterText)
                NetworkMetric(title: "Other", value: "\(status.otherPeerCount)")
                NetworkMetric(title: "Connected", value: "\(status.connectedPeerCount)")
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
                    Text(peer.connectionText)
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
