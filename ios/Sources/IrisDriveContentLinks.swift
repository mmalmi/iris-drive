import UIKit

extension IrisDriveMobileModel {
    func openContentLink(_ linkInput: NativeLinkInputClassification) {
        let displayName = linkInput.openDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = displayName.isEmpty ? "file" : displayName
        guard linkInput.isValid,
              let url = URL(string: linkInput.localOpenUrl)
        else {
            statusTitle = "Could not open \(label)"
            statusDetail = linkInput.error.isEmpty ? linkInput.normalizedInput : linkInput.error
            persist()
            return
        }
        statusTitle = "Opening \(label)"
        statusDetail = linkInput.localOpenUrl
        UIApplication.shared.open(url)
        persist()
    }
}
