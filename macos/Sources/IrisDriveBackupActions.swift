import AppKit
import Foundation

extension AppDelegate {
    func addBlossomServer(_ value: String) {
        let url = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty else { return }
        dispatchNativeAction(
            [
                "type": "add_blossom_server",
                "url": url,
            ],
            progress: "Adding file server",
            success: "File server added"
        )
    }

    func removeBlossomServer(_ value: String) {
        let url = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty else { return }
        dispatchNativeAction(
            [
                "type": "remove_blossom_server",
                "url": url,
            ],
            progress: "Removing file server",
            success: "File server removed"
        )
    }

    func syncFileServers(_ targets: [IrisDriveBackupTarget]) {
        let urls = fileServerURLs(targets)
        guard !urls.isEmpty else {
            NSSound.beep()
            return
        }
        dispatchFileServerBatch(
            urls,
            actionType: "sync_backups",
            progress: "Syncing file servers",
            success: "File servers synced"
        )
    }

    func checkFileServers(
        _ targets: [IrisDriveBackupTarget],
        completion: (() -> Void)? = nil
    ) {
        let urls = fileServerURLs(targets)
        guard !urls.isEmpty else {
            NSSound.beep()
            completion?()
            return
        }
        dispatchFileServerBatch(
            urls,
            actionType: "check_backups",
            progress: "Checking file servers",
            success: "File servers checked",
            completion: completion
        )
    }

    func checkFileServer(_ target: IrisDriveBackupTarget, completion: (() -> Void)? = nil) {
        checkFileServers([target], completion: completion)
    }

    private func fileServerURLs(_ targets: [IrisDriveBackupTarget]) -> [String] {
        targets.compactMap { target in
            guard target.kind == "blossom" else { return nil }
            let url = target.target.trimmingCharacters(in: .whitespacesAndNewlines)
            return url.isEmpty ? nil : url
        }
    }

    private func dispatchFileServerBatch(
        _ urls: [String],
        actionType: String,
        progress: String,
        success: String,
        index: Int = 0,
        completion: (() -> Void)? = nil
    ) {
        guard urls.indices.contains(index) else {
            completion?()
            return
        }
        dispatchNativeAction(
            [
                "type": actionType,
                "target": urls[index],
            ],
            progress: progress,
            success: success,
            completion: { [weak self] in
                guard let self else {
                    completion?()
                    return
                }
                self.dispatchFileServerBatch(
                    urls,
                    actionType: actionType,
                    progress: progress,
                    success: success,
                    index: index + 1,
                    completion: completion
                )
            }
        )
    }
}
