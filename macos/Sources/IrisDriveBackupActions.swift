import AppKit
import Foundation

extension AppDelegate {
    func addBackupTarget(_ value: String, label: String) {
        let target = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        var arguments = ["backups", "add", target]
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        if !label.isEmpty {
            arguments += ["--label", label]
        }
        mutateBackupConfig(arguments: arguments, progress: "Adding backup", success: "Backup added")
    }

    func syncBackups() {
        mutateBackupConfig(arguments: ["backups", "sync"], progress: "Syncing backups", success: "Backups synced")
    }

    func checkBackups(completion: (() -> Void)? = nil) {
        mutateBackupConfig(
            arguments: ["backups", "check"],
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
        mutateBackupConfig(
            arguments: ["backups", "check", "--target", target.target],
            progress: "Checking backup",
            success: "Backup checked",
            completion: completion
        )
    }

    private func mutateBackupConfig(
        arguments: [String],
        progress: String,
        success: String,
        completion: (() -> Void)? = nil
    ) {
        guard let paths = runtimePathsForMenu else {
            NSSound.beep()
            completion?()
            return
        }
        let idrive = idriveExecutableURL()
        updateStatus(progress)
        DispatchQueue.global(qos: .utility).async {
            do {
                _ = try self.runIDrive(idrive, arguments: arguments, paths: paths)
                DispatchQueue.main.async {
                    self.updateStatus(success)
                    self.refreshStatus()
                    completion?()
                }
            } catch {
                NSLog("Iris Drive backup update failed: \(error)")
                DispatchQueue.main.async {
                    self.updateStatus("Backup failed")
                    NSSound.beep()
                    completion?()
                }
            }
        }
    }
}
