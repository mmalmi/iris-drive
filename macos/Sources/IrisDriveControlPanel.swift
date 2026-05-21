import AppKit
import SwiftUI

private enum IrisDrivePanelTab: String, CaseIterable, Identifiable {
    case drive
    case peers
    case network
    case hashtree
    case settings

    var id: Self { self }

    var title: String {
        switch self {
        case .drive:
            return "My Drive"
        case .peers:
            return "Devices"
        case .network:
            return "Network"
        case .hashtree:
            return "Hashtree"
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
        case .network:
            return "network"
        case .hashtree:
            return "shippingbox.fill"
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

struct IrisDriveControlPanel: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate
    @State private var selectedTab = IrisDrivePanelTab.drive
    @State private var relayInput = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""
    @State private var setupMode = IrisDriveSetupMode.welcome
    @State private var setupLabel = ""
    @State private var setupSecret = ""
    @State private var setupOwner = ""

    private let columns = [
        GridItem(.adaptive(minimum: 150), spacing: 12, alignment: .topLeading)
    ]

    var body: some View {
        if !status.initialized {
            setup
        } else {
            controlPanel
        }
    }

    private var controlPanel: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    actions
                    header
                    selectedContent
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
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

    private var header: some View {
        HStack(spacing: 14) {
            Image(systemName: "externaldrive.fill")
                .font(.system(size: 36, weight: .semibold))
                .foregroundStyle(.primary)
            VStack(alignment: .leading, spacing: 4) {
                Text(status.driveName)
                    .font(.title2.weight(.semibold))
                Text(status.message)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            StatusPill(text: status.daemonRunning ? "Running" : "Stopped")
        }
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button(action: controller.showDriveFolder) {
                Label("Drive", systemImage: "folder.fill")
            }
            Button(action: controller.copyDriveLink) {
                Label("Copy Snapshot", systemImage: "link")
            }
            .disabled(status.snapshotLinkURL == nil)
            Button(action: controller.openDriveLink) {
                Label("Open Snapshot", systemImage: "safari.fill")
            }
            .disabled(status.snapshotLinkURL == nil)
            Divider()
                .frame(height: 22)
            Button(action: controller.restartSync) {
                Label("Restart", systemImage: "arrow.clockwise")
            }
            Button(action: controller.stopSync) {
                Label("Stop", systemImage: "stop.fill")
            }
            .disabled(!status.daemonRunning)
            Button(action: controller.startSync) {
                Label("Start", systemImage: "play.fill")
            }
            .disabled(status.daemonRunning)
        }
        .buttonStyle(.bordered)
    }

    @ViewBuilder
    private var selectedContent: some View {
        switch selectedTab {
        case .drive:
            overview
        case .peers:
            peers
        case .network:
            network
        case .hashtree:
            hashtree
        case .settings:
            settings
        }
    }

    private var overview: some View {
        LazyVGrid(columns: columns, spacing: 12) {
            StatTile(title: "Files", value: optionalCount(status.fileCount ?? status.topLevelEntries))
            StatTile(title: "Blocks", value: "\(status.localBlockCount)")
            StatTile(title: "Storage", value: byteString(status.localBlockBytes))
            StatTile(
                title: "Devices",
                value: "\(status.publishedDeviceRoots)/\(status.authorizedDeviceCount)"
            )
        }
    }

    private var peers: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle("Devices")
            if status.peers.isEmpty {
                Text("No authorized devices")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(status.peers) { peer in
                    PeerRow(peer: peer)
                }
            }
        }
    }

    private var settings: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionTitle("Settings")
            Toggle(
                "Menu bar on close",
                isOn: Binding(
                    get: { status.closeToMenuBarOnClose },
                    set: { controller.setCloseToMenuBarOnClose($0) }
                )
            )
            .toggleStyle(.checkbox)
        }
    }

    private var network: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionTitle("Network")
            EndpointGroup(title: "Blossom", values: status.blossomServers)
            relayEditor
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

    private var hashtree: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle("Hashtree")
            PathRow(title: "Config", value: status.configDirectory)
            PathRow(title: "Blocks", value: status.blocksDirectory)
            PathRow(title: "Drive", value: status.workingDirectory)
            if let root = status.rootCID {
                PathRow(title: "Root", value: root)
            }
        }
    }

    private func optionalCount(_ value: Int?) -> String {
        value.map(String.init) ?? "0"
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

private struct StatusPill: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .padding(.vertical, 5)
            .padding(.horizontal, 9)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(Capsule())
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

private struct StatTile: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.title3.weight(.semibold))
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .padding(12)
        .frame(maxWidth: .infinity, minHeight: 72, alignment: .leading)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct PeerRow: View {
    let peer: IrisDrivePeerStatus

    private static let timestampFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .short
        formatter.timeStyle = .short
        return formatter
    }()

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: peer.isCurrentDevice ? "desktopcomputer" : "laptopcomputer")
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(peerTitle)
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                Text(peer.npub)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if !peerMetadata.isEmpty {
                    Text(peerMetadata)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer()
            Text(peer.hasRoot ? "Root" : "No root")
                .foregroundStyle(.secondary)
            Text(peerPrivacy)
                .font(.caption.weight(.semibold))
                .padding(.vertical, 4)
                .padding(.horizontal, 8)
                .background(Color(nsColor: .textBackgroundColor))
                .clipShape(Capsule())
        }
        .padding(.vertical, 8)
    }

    private var peerTitle: String {
        peer.label ?? (peer.isCurrentDevice ? "This Mac" : peer.npub)
    }

    private var peerMetadata: String {
        var parts: [String] = []
        if let root = peer.rootCID {
            parts.append("Root \(shortRoot(root))")
        }
        if let generation = peer.dckGeneration {
            parts.append("DCK \(generation)")
        }
        if let published = peer.publishedAt {
            let date = Date(timeIntervalSince1970: TimeInterval(published))
            parts.append("Published \(Self.timestampFormatter.string(from: date))")
        }
        return parts.joined(separator: " | ")
    }

    private var peerPrivacy: String {
        guard peer.hasRoot else {
            return "Pending"
        }
        return peer.rootIsPrivate == false ? "Public" : "Private"
    }

    private func shortRoot(_ root: String) -> String {
        guard root.count > 20 else {
            return root
        }
        return "\(String(root.prefix(10)))...\(String(root.suffix(6)))"
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

private struct PathRow: View {
    let title: String
    let value: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 62, alignment: .leading)
            Text(value ?? "None")
                .font(.system(.callout, design: .monospaced))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
        }
    }
}
