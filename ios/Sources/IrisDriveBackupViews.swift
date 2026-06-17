import SwiftUI

struct BackupView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section("Actions") {
                Button {
                    model.syncBackups(model.backups)
                } label: {
                    Label("Sync Now", systemImage: "arrow.up.circle")
                }
                .disabled(model.backups.isEmpty)
                Button {
                    model.checkBackups(model.backups)
                } label: {
                    if model.isCheckingBackups {
                        Text(model.backupCheckProgressLabel)
                    } else {
                        Label("Check All", systemImage: "checkmark.shield")
                    }
                }
                .disabled(model.backups.isEmpty || model.isCheckingBackups)
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

            Section("Add Backup") {
                TextField("Destination URL, User ID, or folder path", text: $model.backupTargetInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addBackupTarget() }
                    .accessibilityIdentifier("backupTargetInput")
                TextField("Label", text: $model.backupTargetLabelInput)
                    .textInputAutocapitalization(.words)
                Button {
                    model.addBackupTarget()
                } label: {
                    Label("Add Backup", systemImage: "plus")
                }
                .disabled(model.backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("addBackupButton")
            }

            backupListSection("Destinations", backups: model.backups) { target in
                model.removeBackupTarget(target)
            }
        }
        .navigationTitle("Backup")
    }

    private func backupListSection(
        _ title: String,
        backups: [IrisDriveBackup],
        remove: @escaping (String) -> Void
    ) -> some View {
        Section(title) {
            if backups.isEmpty {
                Text("No backups configured")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(backups) { backup in
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
                            remove(backup.target)
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
        }
    }
}
