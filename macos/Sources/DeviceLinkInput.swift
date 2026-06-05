import Foundation

extension IrisDriveControlPanel {
    func submitSetupLinkTarget(_ value: String, force _: Bool, inputIsComplete: Bool = false) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard inputIsComplete else { return }
        guard submittedSetupLinkTarget != trimmed else { return }
        submittedSetupLinkTarget = trimmed
        controller.linkDevice(target: trimmed)
    }
}
