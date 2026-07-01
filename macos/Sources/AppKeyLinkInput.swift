import Foundation

extension IrisDriveControlPanel {
    func submitSetupLinkTarget(_: String, force _: Bool, inputIsComplete _: Bool = false) {
        controller.startJoinRequest()
    }
}
