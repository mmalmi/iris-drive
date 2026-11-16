import Foundation
import SwiftUI

/// What each of the profile's devices holds of the Drive, as the app core
/// words it (`ui.device_storage`). The shell only lays it out.
struct IrisDriveDeviceStorage {
    let devices: [IrisDriveDeviceStorageRow]

    init(json: [String: Any]) {
        devices = (json["devices"] as? [[String: Any]] ?? []).map(IrisDriveDeviceStorageRow.init)
    }
}

struct IrisDriveDeviceStorageRow {
    let npub: String
    let icon: String
    let hasReport: Bool
    let heldLabel: String
    let heldPermille: Int
    let syncLabel: String
    let syncDetail: String
    let spaceLabel: String
    let spaceDetail: String
    let ageLabel: String
    let folders: [IrisDriveDeviceStorageFolder]
    let notes: [String]

    init(json: [String: Any]) {
        npub = json["npub"] as? String ?? ""
        icon = json["icon"] as? String ?? ""
        hasReport = json["has_report"] as? Bool ?? false
        heldLabel = json["held_label"] as? String ?? ""
        heldPermille = (json["held_permille"] as? NSNumber)?.intValue ?? 0
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

struct DeviceStorageDetails: View {
    let device: IrisDriveDeviceStorageRow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            DeviceStorageBar(permille: device.heldPermille, height: 5)
            HStack {
                Text(device.spaceLabel).help(device.spaceDetail)
                Spacer()
                Text(device.ageLabel)
            }
            if !device.syncLabel.isEmpty {
                Text(device.syncLabel).help(device.syncDetail)
            }
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
        .font(.caption)
        .foregroundStyle(.secondary)
        .accessibilityIdentifier("device-storage")
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
