import Foundation
import UIKit

struct PendingContentLink: Identifiable {
    let id = UUID()
    let linkInput: NativeLinkInputClassification

    var label: String {
        let displayName = linkInput.openDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        return displayName.isEmpty ? "file" : displayName
    }

    var title: String {
        "Open \(label)?"
    }
}

extension IrisDriveMobileModel {
    func openContentLink(_ linkInput: NativeLinkInputClassification) {
        let displayName = linkInput.openDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = displayName.isEmpty ? "file" : displayName
        guard linkInput.isValid,
              URL(string: linkInput.localOpenUrl) != nil
        else {
            statusTitle = "Could not open \(label)"
            statusDetail = linkInput.error.isEmpty ? linkInput.normalizedInput : linkInput.error
            persist()
            return
        }
        pendingContentLink = PendingContentLink(linkInput: linkInput)
        statusTitle = "Iris link opened"
        statusDetail = label
        persist()
    }

    func openPendingContentLink() {
        guard let pending = pendingContentLink else { return }
        pendingContentLink = nil
        openResolvedContentLink(pending.linkInput)
    }

    func savePendingContentLink() {
        guard let pending = pendingContentLink else { return }
        pendingContentLink = nil
        saveContentLink(pending.linkInput)
    }

    func cancelPendingContentLink() {
        pendingContentLink = nil
    }

    private func openResolvedContentLink(_ linkInput: NativeLinkInputClassification) {
        let displayName = linkInput.openDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = displayName.isEmpty ? "file" : displayName
        guard let url = URL(string: linkInput.localOpenUrl) else { return }
        statusTitle = "Opening \(label)"
        statusDetail = linkInput.localOpenUrl
        UIApplication.shared.open(url)
        persist()
    }

    private func saveContentLink(_ linkInput: NativeLinkInputClassification) {
        let displayName = linkInput.openDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = displayName.isEmpty ? "file" : displayName
        statusTitle = "Saving \(label)"
        statusDetail = linkInput.normalizedInput
        let state = dispatch([
            "type": "import_content_link",
            "link": linkInput.normalizedInput,
        ])
        let error = state?.error.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if error.isEmpty {
            statusTitle = "Saved \(label)"
            statusDetail = "Iris Drive"
        }
        persist()
    }
}
