import SwiftUI

struct RevokedDeviceSetupView: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Device removed")
                .font(.title2.weight(.semibold))
            Text("This device no longer has access to Iris Drive.")
                .foregroundStyle(.secondary)
            if let device = status.deviceNpub, !device.isEmpty {
                keyedValue("Current Device Key", device)
            }
            if status.deviceNpub?.isEmpty == false {
                IrisDriveCopyButton(
                    title: "Copy Device Key",
                    systemImage: "doc.on.doc",
                    fillsWidth: true
                ) {
                    controller.copyDeviceKey()
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
            if let request = status.appKeyLinkRequestURL, !request.isEmpty {
                IrisDriveQRCodeView(value: request)
                    .frame(width: 220, height: 220)
                    .frame(maxWidth: .infinity, alignment: .center)
                IrisDriveCopyButton(
                    title: "Copy Request Link",
                    systemImage: "link",
                    fillsWidth: true
                ) {
                    controller.copyAppKeyLinkRequest()
                }
                .buttonStyle(.borderedProminent)
            }
            if let device = status.deviceNpub, !device.isEmpty {
                keyedValue("Current Device Key", device)
                IrisDriveCopyButton(
                    title: "Copy Device Key",
                    systemImage: "doc.on.doc",
                    fillsWidth: true
                ) {
                    controller.copyDeviceKey()
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
