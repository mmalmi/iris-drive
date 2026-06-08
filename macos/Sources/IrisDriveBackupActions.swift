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
            progress: "Adding backup",
            success: "Backup added"
        )
    }

    func removeBackupTarget(_ target: IrisDriveBackupTarget) {
        removeBackupTarget(target.target)
    }

    func removeBackupTarget(_ target: String) {
        let target = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatchNativeAction(
            [
                "type": "remove_backup_target",
                "target": target,
            ],
            progress: "Removing backup",
            success: "Backup removed"
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

    func syncBackups() {
        dispatchNativeAction(
            [
                "type": "sync_backups",
                "target": "",
            ],
            progress: "Syncing backups",
            success: "Backups synced"
        )
    }

    func checkBackups(completion: (() -> Void)? = nil) {
        dispatchNativeAction(
            [
                "type": "check_backups",
                "target": "",
            ],
            progress: "Checking backups",
            success: "Backups checked",
            completion: completion
        )
    }

    func checkBackupTarget(_ target: IrisDriveBackupTarget, completion: (() -> Void)? = nil) {
        guard !target.target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            NSSound.beep()
            completion?()
            return
        }
        dispatchNativeAction(
            [
                "type": "check_backups",
                "target": target.target,
            ],
            progress: "Checking backup",
            success: "Backup checked",
            completion: completion
        )
    }
}
