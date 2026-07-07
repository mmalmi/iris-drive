import UIKit

extension IrisDriveMobileModel {
    func copyAppKey() {
        copyToClipboard(currentAppKeyNpub, feedback: "Device copied")
    }

    func copyDeviceKey() {
        copyToClipboard(devicePublicKey, feedback: "Device key copied")
    }

    func copyLinkRequest() {
        copyToClipboard(appKeyLinkRequest, feedback: "Request link copied")
    }

    func copySnapshotLink() {
        guard !snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        copyToClipboard(snapshotLink, feedback: "drive.iris.to link copied")
    }

    func copyLastShareInvite() {
        guard !lastShareInvite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        copyToClipboard(lastShareInvite, feedback: "Share invite copied")
    }

    func copyShareRecipientEvidence() {
        exportShareRecipientEvidence(displayName: deviceLabel)
        guard !lastShareRecipientEvidence.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        copyToClipboard(lastShareRecipientEvidence, feedback: "Share identity copied")
    }

    func copyToClipboard(_ value: String, feedback: String) {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        UIPasteboard.general.string = value
        showCopyFeedback(feedback)
    }

    private func showCopyFeedback(_ message: String) {
        copyFeedbackTask?.cancel()
        copyFeedback = message
        copyFeedbackTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            copyFeedback = ""
        }
    }
}
