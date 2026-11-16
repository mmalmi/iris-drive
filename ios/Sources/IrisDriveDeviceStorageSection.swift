import SwiftUI

/// What each of the profile's devices holds of the Drive. The core words
/// every label; this only lays them out.
struct DeviceStorageDetails: View {
    let device: NativeDeviceStorageRow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ProgressView(value: fraction(device.heldPermille))
            HStack {
                Text(device.spaceLabel).accessibilityHint(device.spaceDetail)
                Spacer()
                Text(device.ageLabel)
            }
            if !device.syncLabel.isEmpty {
                Text(device.syncLabel).accessibilityHint(device.syncDetail)
            }
            folderLines
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .accessibilityIdentifier("deviceStorageDetails")
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

    private func fraction(_ permille: UInt32) -> Double {
        Double(min(permille, 1000)) / 1000
    }
}
