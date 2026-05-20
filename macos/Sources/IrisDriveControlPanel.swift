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
            return "Peers"
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

struct IrisDriveControlPanel: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate
    @State private var selectedTab = IrisDrivePanelTab.drive

    private let columns = [
        GridItem(.adaptive(minimum: 150), spacing: 12, alignment: .topLeading)
    ]

    var body: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    header
                    actions
                    selectedContent
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
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
                Label("Copy Link", systemImage: "link")
            }
            .disabled(status.filesIrisURL == nil)
            Button(action: controller.openDriveLink) {
                Label("Open Link", systemImage: "safari.fill")
            }
            .disabled(status.filesIrisURL == nil)
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
            StatTile(title: "Files", value: optionalCount(status.topLevelEntries))
            StatTile(title: "Blocks", value: "\(status.localBlockCount)")
            StatTile(title: "Storage", value: byteString(status.localBlockBytes))
            StatTile(
                title: "Devices",
                value: "\(status.publishedDeviceRoots)/\(status.authorizedDeviceCount)"
            )
            StatTile(title: "Privacy", value: privacyLabel)
            StatTile(title: "Upload", value: uploadLabel)
        }
    }

    private var peers: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle("Peers")
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
            EndpointGroup(title: "Relays", values: status.relays)
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

    private var privacyLabel: String {
        switch status.rootIsPrivate {
        case true:
            return "Private"
        case false:
            return "Public"
        case nil:
            return "Pending"
        }
    }

    private var uploadLabel: String {
        guard let upload = status.lastUpload else {
            return status.blossomServers.first ?? "None"
        }
        return "\(upload.uploaded) up, \(upload.alreadyPresent) cached"
    }

    private func optionalCount(_ value: Int?) -> String {
        value.map(String.init) ?? "0"
    }

    private func byteString(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}

private struct SidebarRow: View {
    let symbol: String
    let title: String
    var selected = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.callout.weight(selected ? .semibold : .regular))
                .padding(.vertical, 6)
                .padding(.horizontal, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    selected
                        ? Color(nsColor: .selectedContentBackgroundColor).opacity(0.18)
                        : .clear
                )
                .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
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
