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
                keyedValue("Current Device ID", device)
            }
            if status.deviceNpub?.isEmpty == false {
                IrisDriveCopyButton(
                    title: "Copy Device ID",
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
    let openSigner: () -> Void

    var body: some View {
        VStack(alignment: .center, spacing: 12) {
            Button {
                controller.logout()
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .buttonStyle(.borderless)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityIdentifier("awaitingApprovalBack")
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
            IrisDriveCopyButton(title: "Sign in with remote signer", systemImage: "signature", fillsWidth: true) {
                openSigner()
            }
            .accessibilityIdentifier("openRemoteSigner")
            .buttonStyle(.bordered)
            IrisDriveCopyButton(
                title: "Restore from secret key",
                systemImage: "key",
                fillsWidth: true
            ) {
                openSecretKey()
            }
            .buttonStyle(.bordered)
        }
        .multilineTextAlignment(.center)
    }
}

struct DriveCredentialSetupFields: View {
    let signer: Bool
    let controller: AppDelegate
    @State private var input = ""
    @State private var submitted = false
    private var valid: Bool {
        let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        return !value.isEmpty && (!signer || value.hasPrefix("bunker://"))
    }

    private func submit() {
        guard valid else { return }
        submitted = true
        controller.restoreProfile(recoverySecret: input)
    }

    var body: some View {
        SecureField(signer ? "Signer connection link (bunker://…)" : "nsec1... or hex secret key", text: $input)
            .accessibilityIdentifier(signer ? "remoteSignerInput" : "restoreSecretKeyInput")
            .onSubmit(submit)
        IrisDriveCopyButton(title: signer ? "Connect signer" : "Restore", fillsWidth: true, action: submit)
            .accessibilityIdentifier(signer ? "remoteSignerSubmit" : "restoreSecretKeySubmit")
            .buttonStyle(.borderedProminent)
            .disabled(!valid)
        if signer && submitted {
            Button("Open signer") { controller.signerInteraction(cancel: false) }
                .onDisappear { controller.signerInteraction(cancel: true) }
        }
    }
}

extension AppDelegate {
    func signerInteraction(cancel: Bool) {
        let method = cancel ? "cancel_signer_recovery" : "signer_approval_link"
        let json = desktopCore.dispatchJson("{\"type\":\"\(method)\"}")
        guard !cancel else { return }
        if let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
           let value = object["signer_approval_url"] as? String, let url = URL(string: value), url.scheme == "https" {
            NSWorkspace.shared.open(url)
        } else {
            updateStatus("Check your signer for an approval request")
        }
    }
}
