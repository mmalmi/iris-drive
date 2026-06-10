import Foundation

extension IrisDriveMobileModel {
    var isCheckingBackups: Bool {
        backupCheckTotal > 0
    }

    var backupCheckProgressLabel: String {
        guard backupCheckTotal > 0 else { return "" }
        return "Checking \(backupCheckCompleted) of \(backupCheckTotal)"
    }

    func backupIsChecking(_ target: String) -> Bool {
        checkingBackupTargets.contains(target.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    func addBackupTarget() {
        let target = backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = backupTargetLabelInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatch([
            "type": "add_backup_target",
            "target": target,
            "label": label,
        ])
        backupTargetInput = ""
        backupTargetLabelInput = ""
    }

    func removeBackupTarget(_ target: String) {
        let target = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        dispatch([
            "type": "remove_backup_target",
            "target": target,
        ])
    }

    func syncBackups(_ backups: [IrisDriveBackup]) {
        dispatchBackupAction("sync_backups", backups: backups)
    }

    func checkBackups(_ backups: [IrisDriveBackup]) {
        dispatchBackupAction("check_backups", backups: backups)
    }

    func dispatchBackupAction(_ type: String, backups: [IrisDriveBackup]) {
        let targets = backups
            .map { $0.target.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        dispatchBackupAction(type, targets: targets)
    }

    func dispatchBackupAction(_ type: String, targets: [String]) {
        guard isSetupComplete, !targets.isEmpty else { return }
        let tracksCheckProgress = type == "check_backups"
        if tracksCheckProgress {
            backupCheckCompleted = 0
            backupCheckTotal = targets.count
            checkingBackupTargets = Set(targets)
        }
        Task {
            defer {
                if tracksCheckProgress {
                    backupCheckCompleted = 0
                    backupCheckTotal = 0
                    checkingBackupTargets = []
                }
            }
            for target in targets {
                await dispatchInBackground([
                    "type": type,
                    "target": target,
                ], invalidatePendingState: true)
                if tracksCheckProgress {
                    backupCheckCompleted += 1
                    checkingBackupTargets.remove(target)
                }
            }
        }
    }
}
