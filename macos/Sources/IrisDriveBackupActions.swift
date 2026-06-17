import AppKit
import Foundation

extension AppDelegate {
    func addBackupTarget(_ value: String, label: String) {
        let target = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatchNativeAction(
            [
                "type": "add_backup_target",
                "target": target,
                "label": label.trimmingCharacters(in: .whitespacesAndNewlines),
            ],
            progress: "Adding backup target",
            success: "Backup target added"
        )
    }

    func removeBackupTarget(_ value: String) {
        let target = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatchNativeAction(
            [
                "type": "remove_backup_target",
                "target": target,
            ],
            progress: "Removing backup target",
            success: "Backup target removed"
        )
    }

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

    func syncBackups(_ targets: [IrisDriveBackupTarget]) {
        let targets = backupTargetURLs(targets)
        guard !targets.isEmpty else {
            NSSound.beep()
            return
        }
        dispatchBackupTargetBatch(
            targets,
            actionType: "sync_backups",
            progress: "Syncing backups",
            success: "Backups synced"
        )
    }

    func checkBackups(
        _ targets: [IrisDriveBackupTarget],
        progress: ((Int, Int) -> Void)? = nil,
        completion: (() -> Void)? = nil
    ) {
        let targets = backupTargetURLs(targets)
        guard !targets.isEmpty else {
            NSSound.beep()
            completion?()
            return
        }
        dispatchBackupTargetBatch(
            targets,
            actionType: "check_backups",
            progress: "Checking backups",
            success: "Backups checked",
            progressUpdate: progress,
            completion: completion
        )
    }

    private func backupTargetURLs(_ targets: [IrisDriveBackupTarget]) -> [String] {
        targets.compactMap { target in
            let url = target.target.trimmingCharacters(in: .whitespacesAndNewlines)
            return url.isEmpty ? nil : url
        }
    }

    private func dispatchBackupTargetBatch(
        _ targets: [String],
        actionType: String,
        progress: String,
        success: String,
        index: Int = 0,
        progressUpdate: ((Int, Int) -> Void)? = nil,
        completion: (() -> Void)? = nil
    ) {
        guard targets.indices.contains(index) else {
            progressUpdate?(targets.count, targets.count)
            completion?()
            return
        }
        progressUpdate?(index, targets.count)
        dispatchNativeAction(
            [
                "type": actionType,
                "target": targets[index],
            ],
            progress: progress,
            success: success,
            completion: { [weak self] in
                guard let self else {
                    completion?()
                    return
                }
                self.dispatchBackupTargetBatch(
                    targets,
                    actionType: actionType,
                    progress: progress,
                    success: success,
                    index: index + 1,
                    progressUpdate: progressUpdate,
                    completion: completion
                )
            }
        )
    }

}
