import SwiftUI

/// What each of the profile's devices holds of the Drive. The core words
/// every label; this only lays them out.
struct DeviceStorageSection: View {
    let storage: NativeDeviceStorage

    var body: some View {
        if !storage.devices.isEmpty {
            Section("Storage") {
                if !storage.summary.isEmpty {
                    Text(storage.summary)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("deviceStorageSummary")
                }
                ForEach(storage.devices) { device in
                    DeviceStorageRow(device: device)
                }
            }
        }
    }
}

private struct DeviceStorageRow: View {
    let device: NativeDeviceStorageRow

    var body: some View {
        if device.hasReport {
            DisclosureGroup {
                folderLines
            } label: {
                overview
            }
        } else {
            overview
        }
    }

    private var overview: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: iconName)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(device.name)
                        .lineLimit(1)
                    if device.isCurrent {
                        Text("This device")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 8)
                    Text(device.heldLabel)
                        .font(.subheadline)
                        .foregroundStyle(device.holdsAll ? .green : .secondary)
                }
                if !device.isCurrent {
                    statusLine
                }
                if device.hasReport {
                    ProgressView(value: fraction(device.heldPermille))
                        .accessibilityHidden(true)
                    HStack(alignment: .firstTextBaseline) {
                        Text(device.spaceLabel)
                            .accessibilityHint(device.spaceDetail)
                        Spacer(minLength: 8)
                        Text(device.ageLabel)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                } else if device.isCurrent && !device.ageLabel.isEmpty {
                    Text(device.ageLabel)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .accessibilityIdentifier("deviceStorageRow")
    }

    private var statusLine: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(device.connected ? .green : .secondary)
                .frame(width: 7, height: 7)
                .accessibilityHidden(true)
            Text(device.connectionLabel)
                .accessibilityHint(device.connectionDetail)
            if !device.syncLabel.isEmpty {
                Text("·")
                    .accessibilityHidden(true)
                Text(device.syncLabel)
                    .foregroundStyle(syncColor)
                    .accessibilityHint(device.syncDetail)
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .lineLimit(1)
    }

    @ViewBuilder private var folderLines: some View {
        ForEach(Array(device.folders.enumerated()), id: \.offset) { _, folder in
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(folder.label)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    Text(folder.detail)
                        .foregroundStyle(.secondary)
                }
                .font(.footnote)
                ProgressView(value: fraction(folder.heldPermille))
                    .scaleEffect(x: 1, y: 0.5, anchor: .center)
                    .accessibilityHidden(true)
            }
            .padding(.leading, CGFloat(folder.depth) * 16)
        }
        ForEach(Array(device.notes.enumerated()), id: \.offset) { _, note in
            Text(note)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var iconName: String {
        switch device.icon {
        case "web":
            return "globe"
        case "phone":
            return "iphone"
        case "computer":
            return "laptopcomputer"
        default:
            return "externaldrive.fill"
        }
    }

    private var syncColor: Color {
        switch device.syncState {
        case "in-sync":
            return .green
        case "behind", "apart":
            return .orange
        default:
            return .secondary
        }
    }

    private func fraction(_ permille: UInt32) -> Double {
        Double(min(permille, 1000)) / 1000
    }
}
