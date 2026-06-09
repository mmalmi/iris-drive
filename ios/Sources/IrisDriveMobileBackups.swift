import Foundation

extension IrisDriveMobileModel {
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

    private func dispatchBackupAction(_ type: String, backups: [IrisDriveBackup]) {
        let targets = backups
            .map { $0.target.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard isSetupComplete, !targets.isEmpty else { return }
        Task {
            for target in targets {
                await dispatchInBackground([
                    "type": type,
                    "target": target,
                ], invalidatePendingState: true)
            }
        }
    }
}
