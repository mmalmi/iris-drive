import SwiftUI

struct RevokedDeviceSetupView: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate

    var body: some View {
        let target = status.currentAppKeyNpub ?? ""
        VStack(alignment: .leading, spacing: 12) {
            Text("Device removed")
                .font(.title2.weight(.semibold))
            Text("This device no longer has access to Iris Drive.")
                .foregroundStyle(.secondary)
            if !target.isEmpty {
                keyedValue("Device", target)
            }
            if let device = status.deviceNpub, !device.isEmpty {
                keyedValue("Current Device Key", device)
            }
            Button {
                controller.linkDevice(target: target)
            } label: {
                buttonLabel("Link this device again", systemImage: "link")
            }
            .buttonStyle(.borderedProminent)
            .disabled(target.isEmpty)
            if status.deviceNpub?.isEmpty == false {
                Button {
                    controller.copyDeviceKey()
                } label: {
                    buttonLabel("Copy Device Key", systemImage: "doc.on.doc")
                }
                .buttonStyle(.bordered)
            }
            Button(role: .destructive) {
                controller.logout()
            } label: {
                buttonLabel("Log out", systemImage: "rectangle.portrait.and.arrow.right")
            }
            .buttonStyle(.bordered)
        }
    }

    private func keyedValue(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .lineLimit(2)
                .textSelection(.enabled)
        }
    }

    private func buttonLabel(_ title: String, systemImage: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
                .frame(width: 18)
            Text(title)
        }
        .frame(maxWidth: .infinity, minHeight: setupButtonMinHeight)
        .contentShape(Rectangle())
    }
}

struct AwaitingApprovalSetupView: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Waiting for approval")
                .font(.title2.weight(.semibold))
            if let appKey = status.currentAppKeyNpub, !appKey.isEmpty {
                keyedValue("Device", appKey)
            }
            if let device = status.deviceNpub, !device.isEmpty {
                keyedValue("Current Device Key", device)
                Button {
                    controller.copyDeviceKey()
                } label: {
                    buttonLabel("Copy Device Key", systemImage: "doc.on.doc")
                }
                .buttonStyle(.bordered)
            }
            Button(role: .destructive) {
                controller.logout()
            } label: {
                buttonLabel("Log out", systemImage: "rectangle.portrait.and.arrow.right")
            }
            .buttonStyle(.bordered)
        }
    }

    private func keyedValue(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .lineLimit(2)
                .textSelection(.enabled)
        }
    }

    private func buttonLabel(_ title: String, systemImage: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
                .frame(width: 18)
            Text(title)
        }
        .frame(maxWidth: .infinity, minHeight: setupButtonMinHeight)
        .contentShape(Rectangle())
    }
}
