import SwiftUI

struct BackupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var addingBackup = false

    var body: some View {
        Form {
            PhotoBackupSection(backup: model.photoBackup)
            Section {
                if model.backups.isEmpty {
                    Text("No backups configured")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.backups) { backup in
                    backupRow(backup)
                }
                if !model.backups.isEmpty {
                    syncAndCheck
                }
                DisclosureGroup("Add Backup", isExpanded: $addingBackup) {
                    TextField("Destination URL, User ID, or folder path", text: $model.backupTargetInput)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(addBackup)
                        .accessibilityIdentifier("backupTargetInput")
                    TextField("Label", text: $model.backupTargetLabelInput)
                        .textInputAutocapitalization(.words)
                    Button(action: addBackup) {
                        Label("Add", systemImage: "plus")
                    }
                    .disabled(model.backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("addBackupButton")
                }
            } header: {
                Text("Backups of My Drive")
            } footer: {
                Text("Encrypted copies of My Drive kept on a file server, with a friend, or in a folder.")
            }
        }
        .navigationTitle("Backup")
    }

    private func addBackup() {
        model.addBackupTarget()
        addingBackup = false
    }

    @ViewBuilder private var syncAndCheck: some View {
        Button {
            model.syncBackups(model.backups)
        } label: {
            Label("Sync Now", systemImage: "arrow.up.circle")
        }
        Button {
            model.checkBackups(model.backups)
        } label: {
            if model.isCheckingBackups {
                Text(model.backupCheckProgressLabel)
            } else {
                Label("Check All", systemImage: "checkmark.shield")
            }
        }
        .disabled(model.isCheckingBackups)
        if model.isCheckingBackups {
            ProgressView(
                value: Double(model.backupCheckCompleted),
                total: Double(max(model.backupCheckTotal, 1))
            )
            Text(model.backupCheckProgressLabel)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func backupRow(_ backup: IrisDriveBackup) -> some View {
        DisclosureGroup {
            Text(backup.detail)
                .font(.footnote)
                .foregroundStyle(.secondary)
            Button {
                model.checkBackups([backup])
            } label: {
                Label(
                    model.backupIsChecking(backup.target) ? "Checking 0 of 1" : "Check",
                    systemImage: "checkmark.shield"
                )
            }
            .disabled(model.backupIsChecking(backup.target))
            Button(role: .destructive) {
                model.removeBackupTarget(backup.target)
            } label: {
                Label("Remove backup", systemImage: "trash")
            }
        } label: {
            VStack(alignment: .leading) {
                Text(backup.label)
                Text(model.backupIsChecking(backup.target) ? "Checking 0 of 1" : backup.state)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
