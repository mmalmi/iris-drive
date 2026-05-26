import AppKit
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
    case restore
    case link
}

private enum IrisDriveSyncState {
    case upToDate
    case syncing(Int, Int)
    case paused
    case attention
}

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
    @State private var setupLabel = ""
    @State private var setupSecret = ""
    @State private var setupOwner = ""
    @State private var approveDeviceKey = ""
    @State private var approveDeviceLabel = ""
    @State private var showAddDevice = false
    @State private var showAddBackup = false

    var body: some View {
        Group {
            if !status.initialized {
                setup
            } else {
                controlPanel
            }
        }
        .onAppear {
            controller.ensureFileProviderDomain()
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
            Image(systemName: "externaldrive.fill")
                .font(.system(size: 72, weight: .semibold))
                .foregroundStyle(.primary)
            Text("Iris Drive")
                .font(.title.weight(.semibold))
            setupContent
                .frame(width: 340)
            if status.message != "Setup needed" {
                Text(status.message)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(32)
        .frame(minWidth: 520, minHeight: 420)
    }

    @ViewBuilder
    private var setupContent: some View {
        switch setupMode {
        case .welcome:
            VStack(spacing: 12) {
                Button {
                    setupMode = .create
                } label: {
                    Label("Create profile", systemImage: "plus")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                Button {
                    setupMode = .restore
                } label: {
                    Label("Restore profile", systemImage: "key.fill")
                        .frame(maxWidth: .infinity)
                }
                Button {
                    setupMode = .link
                } label: {
                    Label("Link this device", systemImage: "desktopcomputer")
                        .frame(maxWidth: .infinity)
                }
            }
        case .create:
            setupForm(title: "Create profile") {
                TextField("Device label", text: $setupLabel)
                setupSubmit("Create profile") {
                    controller.createProfile(label: setupLabel)
                }
            }
        case .restore:
            setupForm(title: "Restore profile") {
                SecureField("Secret key", text: $setupSecret)
                TextField("Device label", text: $setupLabel)
                setupSubmit("Restore profile") {
                    controller.restoreProfile(secretKey: setupSecret, label: setupLabel)
                }
                .disabled(setupSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        case .link:
            setupForm(title: "Link this device") {
                TextField("Owner public key", text: $setupOwner)
                TextField("Device label", text: $setupLabel)
                setupSubmit("Link device") {
                    controller.linkDevice(owner: setupOwner, label: setupLabel)
                }
                .disabled(setupOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    private func setupForm<Content: View>(
        title: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                setupMode = .welcome
            } label: {
                Image(systemName: "chevron.left")
            }
            .buttonStyle(.borderless)
            Text(title)
                .font(.title2.weight(.semibold))
            content()
        }
        .textFieldStyle(.roundedBorder)
    }

    private func setupSubmit(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
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
           upload.totalHashes > 0,
           upload.uploaded < upload.totalHashes {
            return .syncing(upload.uploaded, upload.totalHashes)
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
                ForEach(status.peers) { peer in
                    PeerRow(peer: peer)
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
            Text("Paste the public key shown on the other device when you link it.")
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField("Device public key", text: $approveDeviceKey)
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
                AccountInfoRow(title: "State", value: status.authorizationState ?? "-")
            }

            Section("Network") {
                relayEditor
                EndpointGroup(title: "Blossom", values: status.blossomServers)
                FipsDiagnostics(status: status.fips)
            }

            Section("Sync & advanced") {
                HStack(spacing: 10) {
                    Button("Start") { controller.startSync() }
                        .disabled(status.daemonRunning)
                    Button("Stop") { controller.stopSync() }
                        .disabled(!status.daemonRunning)
                    Button("Restart") { controller.restartSync() }
                }
                Button("Copy snapshot link") { controller.copyDriveLink() }
                    .disabled(status.snapshotLinkURL == nil)
                Button("Open snapshot link") { controller.openDriveLink() }
                    .disabled(status.snapshotLinkURL == nil)
                LabeledContent("Blocks", value: "\(status.localBlockCount)")
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
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 12) {
                    Circle()
                        .fill(peer.fipsOnline ? Color.green : Color.secondary.opacity(0.5))
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
                }
                .padding(.top, 12)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var title: String {
        peer.label ?? (peer.isCurrentDevice ? "This Mac" : peer.npub)
    }

    private var subtitle: String {
        if peer.isCurrentDevice {
            return "This device"
        }
        return peer.fipsOnline ? "Online" : "Offline"
    }

    private var privacy: String {
        guard peer.hasRoot else {
            return "Pending"
        }
        return peer.rootIsPrivate == false ? "Public" : "Private"
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
