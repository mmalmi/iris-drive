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
    let openRecoveryPhrase: () -> Void
    let openSecretKey: () -> Void

    var body: some View {
        VStack(alignment: .center, spacing: 12) {
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
            IrisDriveCopyButton(
                title: "Restore from recovery phrase",
                systemImage: "text.badge.checkmark",
                fillsWidth: true
            ) {
                openRecoveryPhrase()
            }
            .buttonStyle(.bordered)
            IrisDriveCopyButton(
                title: "Restore from secret key",
                systemImage: "key",
                fillsWidth: true
            ) {
                openSecretKey()
            }
            .buttonStyle(.bordered)
            Button(role: .destructive) {
                controller.logout()
            } label: {
                buttonLabel("Log out", systemImage: "rectangle.portrait.and.arrow.right")
            }
            .buttonStyle(.bordered)
        }
        .multilineTextAlignment(.center)
    }

    private func keyedValue(_ title: String, _ value: String) -> some View {
        VStack(alignment: .center, spacing: 4) {
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
