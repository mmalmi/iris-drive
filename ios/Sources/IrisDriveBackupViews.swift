import SwiftUI

struct FileServersView: View {
    @ObservedObject var model: IrisDriveMobileModel

    private var customTargets: [IrisDriveBackup] {
        model.backups.filter { $0.kind != "blossom" }
    }

    private var fileServers: [IrisDriveBackup] {
        model.backups.filter { $0.kind == "blossom" }
    }

    var body: some View {
        Form {
            Section("Actions") {
                Button {
                    model.syncBackups(customTargets)
                    model.syncFileServers(fileServers)
                } label: {
                    Label("Sync Now", systemImage: "arrow.up.circle")
                }
                .disabled(customTargets.isEmpty && fileServers.isEmpty)
                Button {
                    model.checkBackups(customTargets + fileServers)
                } label: {
                    if model.isCheckingBackups {
                        Text(model.backupCheckProgressLabel)
                    } else {
                        Label("Check All", systemImage: "checkmark.shield")
                    }
                }
                .disabled((customTargets.isEmpty && fileServers.isEmpty) || model.isCheckingBackups)
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

            Section("Add Custom Target") {
                TextField("Target URL", text: $model.backupTargetInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addBackupTarget() }
                TextField("Label", text: $model.backupTargetLabelInput)
                    .textInputAutocapitalization(.words)
                Button {
                    model.addBackupTarget()
                } label: {
                    Label("Add Custom Target", systemImage: "plus")
                }
                .disabled(model.backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Add File Server") {
                TextField("Server URL", text: $model.blossomEndpointInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addBlossomServer() }
                Button {
                    model.addBlossomServer()
                } label: {
                    Label("Add File Server", systemImage: "plus")
                }
                .disabled(model.blossomEndpointInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            backupListSection("Custom Targets", backups: customTargets, removeTitle: "Remove target") { target in
                model.removeBackupTarget(target)
            }

            backupListSection("File Servers", backups: fileServers, removeTitle: "Remove file server") { target in
                model.removeBlossomServer(target)
            }
        }
        .navigationTitle("File Servers")
    }

    private func backupListSection(
        _ title: String,
        backups: [IrisDriveBackup],
        removeTitle: String,
        remove: @escaping (String) -> Void
    ) -> some View {
        Section(title) {
            if backups.isEmpty {
                Text(title == "File Servers" ? "No file servers yet" : "No backup targets yet")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(backups) { backup in
                    DisclosureGroup {
                        Text(backup.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                        Button {
                            model.checkFileServer(backup.target)
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
                            Label(removeTitle, systemImage: "trash")
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
