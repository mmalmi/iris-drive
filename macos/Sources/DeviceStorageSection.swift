import Foundation
import SwiftUI

/// What each of the profile's devices holds of the Drive, as the app core
/// words it (`ui.device_storage`). The shell only lays it out.
struct IrisDriveDeviceStorage {
    let summary: String
    let devices: [IrisDriveDeviceStorageRow]

    init(json: [String: Any]) {
        summary = json["summary"] as? String ?? ""
        devices = (json["devices"] as? [[String: Any]] ?? []).map(IrisDriveDeviceStorageRow.init)
    }
}

struct IrisDriveDeviceStorageRow: Identifiable {
    var id: String { pubkey }
    let pubkey: String
    let name: String
    let isCurrent: Bool
    let icon: String
    let hasReport: Bool
    let heldLabel: String
    let holdsAll: Bool
    let heldPermille: Int
    let connected: Bool
    let connectionLabel: String
    let connectionDetail: String
    let syncState: String
    let syncLabel: String
    let syncDetail: String
    let spaceLabel: String
    let spaceDetail: String
    let ageLabel: String
    let folders: [IrisDriveDeviceStorageFolder]
    let notes: [String]

    init(json: [String: Any]) {
        pubkey = json["pubkey"] as? String ?? ""
        name = json["name"] as? String ?? ""
        isCurrent = json["is_current"] as? Bool ?? false
        icon = json["icon"] as? String ?? ""
        hasReport = json["has_report"] as? Bool ?? false
        heldLabel = json["held_label"] as? String ?? ""
        holdsAll = json["holds_all"] as? Bool ?? false
        heldPermille = (json["held_permille"] as? NSNumber)?.intValue ?? 0
        connected = json["connected"] as? Bool ?? false
        connectionLabel = json["connection_label"] as? String ?? ""
        connectionDetail = json["connection_detail"] as? String ?? ""
        syncState = json["sync_state"] as? String ?? ""
        syncLabel = json["sync_label"] as? String ?? ""
        syncDetail = json["sync_detail"] as? String ?? ""
        spaceLabel = json["space_label"] as? String ?? ""
        spaceDetail = json["space_detail"] as? String ?? ""
        ageLabel = json["age_label"] as? String ?? ""
        folders = (json["folders"] as? [[String: Any]] ?? []).map(IrisDriveDeviceStorageFolder.init)
        notes = json["notes"] as? [String] ?? []
    }
}

struct IrisDriveDeviceStorageFolder {
    let label: String
    let depth: Int
    let detail: String
    let heldPermille: Int

    init(json: [String: Any]) {
        label = json["label"] as? String ?? ""
        depth = (json["depth"] as? NSNumber)?.intValue ?? 0
        detail = json["detail"] as? String ?? ""
        heldPermille = (json["held_permille"] as? NSNumber)?.intValue ?? 0
    }
}

struct DeviceStorageSection: View {
    let storage: IrisDriveDeviceStorage

    var body: some View {
        if !storage.devices.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("Storage")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                if !storage.summary.isEmpty {
                    Text(storage.summary)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                ForEach(storage.devices) { device in
                    DeviceStorageRowView(device: device)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("device-storage")
        }
    }
}

private struct DeviceStorageRowView: View {
    let device: IrisDriveDeviceStorageRow
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                guard device.hasReport else { return }
                withAnimation(.easeInOut(duration: 0.15)) {
                    expanded.toggle()
                }
            } label: {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: symbol)
                        .frame(width: 24)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 6) {
                        header
                        if !device.isCurrent {
                            status
                        }
                        if device.hasReport {
                            DeviceStorageBar(permille: device.heldPermille, height: 6)
                            HStack {
                                Text(device.spaceLabel).help(device.spaceDetail)
                                Spacer()
                                Text(device.ageLabel)
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        } else if device.isCurrent, !device.ageLabel.isEmpty {
                            Text(device.ageLabel)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    if device.hasReport {
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                            .padding(.top, 2)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(device.name)

            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(device.folders.enumerated()), id: \.offset) { _, folder in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(alignment: .firstTextBaseline) {
                                Text(folder.label).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(folder.detail).foregroundStyle(.secondary)
                            }
                            .font(.caption)
                            DeviceStorageBar(permille: folder.heldPermille, height: 4)
                        }
                        .padding(.leading, CGFloat(folder.depth) * 12)
                    }
                    ForEach(device.notes, id: \.self) { note in
                        Text(note)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.leading, 36)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(device.name)
                .font(.callout.weight(.medium))
                .lineLimit(1)
            if device.isCurrent {
                Text("This device")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Text(device.heldLabel)
                .font(.callout)
                .foregroundStyle(device.holdsAll ? Color.green : Color.secondary)
        }
    }

    private var status: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(device.connected ? Color.green : Color.secondary.opacity(0.5))
                .frame(width: 7, height: 7)
            Text(device.connectionLabel)
                .foregroundStyle(.secondary)
                .help(device.connectionDetail)
            if !device.syncLabel.isEmpty {
                Text("·").foregroundStyle(.secondary)
                Text(device.syncLabel)
                    .foregroundStyle(syncColor)
                    .help(device.syncDetail)
            }
        }
        .font(.caption)
    }

    private var syncColor: Color {
        switch device.syncState {
        case "in-sync": return .green
        case "behind", "apart": return .orange
        default: return .secondary
        }
    }

    private var symbol: String {
        switch device.icon {
        case "web": return "globe"
        case "phone": return "iphone"
        case "computer": return "laptopcomputer"
        default: return "externaldrive.fill"
        }
    }
}

private struct DeviceStorageBar: View {
    let permille: Int
    let height: CGFloat

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.secondary.opacity(0.2))
                Capsule()
                    .fill(Color.accentColor)
                    .frame(width: proxy.size.width * CGFloat(min(max(permille, 0), 1000)) / 1000)
            }
        }
        .frame(height: height)
        .accessibilityHidden(true)
    }
}
