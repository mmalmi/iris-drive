import PhotosUI
import SwiftUI

struct PhotoBackupSection: View {
    @ObservedObject var backup: IrisDrivePhotoBackup
    @Environment(\.openURL) private var openURL

    var body: some View {
        Section {
            Toggle("Back up photos and videos", isOn: Binding(get: { backup.enabled }, set: backup.setEnabled))
                .accessibilityIdentifier("photoBackupToggle")
            if backup.enabled {
                switch backup.access {
                case .denied, .restricted:
                    Text("Photo library access is required.")
                    if backup.access == .denied {
                        Button("Open Settings") {
                            if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                        }
                    }
                case .authorized, .limited:
                    if backup.access == .limited {
                        Button("Manage selected photos", action: manageSelection)
                    }
                    HStack(spacing: 8) {
                        if backup.busy { ProgressView() }
                        Text(backup.status.isEmpty ? "Ready" : backup.status)
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                    .accessibilityIdentifier("photoBackupStatus")
                    if let last = backup.lastComplete {
                        Text(Date().timeIntervalSince(last) < 60
                            ? "Last full backup just now"
                            : "Last full backup \(last.formatted(.relative(presentation: .named)))")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                    if backup.waitingForWiFiNow {
                        Text("Connect to Wi-Fi, or turn on Sync over cellular in Settings.")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                    Button("Back up now", action: backup.requestScan).disabled(backup.busy || backup.waitingForWiFiNow)
                default:
                    Text("Waiting for photo access…")
                }
            }
        } header: {
            Text("Photo backup")
        } footer: {
            Text("Save originals to Photos in My Drive. Removing an item from your library keeps its Drive copy.")
        }
    }

    private func manageSelection() {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .first { $0.activationState == .foregroundActive }
        guard var presenter = scene?.keyWindow?.rootViewController else { return }
        while let next = presenter.presentedViewController { presenter = next }
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: presenter)
    }
}
