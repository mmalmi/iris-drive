import SwiftUI

struct IrisDriveRootView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        if model.ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            SetupWelcomeView(model: model)
        } else {
            TabView {
                NavigationStack {
                    DriveHomeView(model: model)
                }
                .tabItem {
                    Label("My Drive", systemImage: "externaldrive.fill")
                }

                NavigationStack {
                    DevicesView(model: model)
                }
                .tabItem {
                    Label("Devices", systemImage: "person.2.fill")
                }

                NavigationStack {
                    BackupsView(model: model)
                }
                .tabItem {
                    Label("Backups", systemImage: "lock.shield.fill")
                }

                NavigationStack {
                    SettingsView(model: model)
                }
                .tabItem {
                    Label("Settings", systemImage: "gearshape.fill")
                }
            }
        }
    }
}

private struct SetupWelcomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var ownerPublicKey = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(spacing: 14) {
                        Image("BrandIcon")
                            .resizable()
                            .interpolation(.high)
                            .frame(width: 96, height: 96)
                        Text("Iris Drive")
                            .font(.title.bold())
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                }

                Section {
                    TextField("Device label", text: $model.deviceLabel)
                    Button {
                        model.createProfile()
                    } label: {
                        Label("Create profile", systemImage: "plus")
                    }
                }

                Section {
                    TextField("Owner public key", text: $ownerPublicKey)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button {
                        model.ownerPublicKey = ownerPublicKey
                        model.linkDevice()
                    } label: {
                        Label("Link this device", systemImage: "link")
                    }
                    .disabled(ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }

                Section {
                    SecureField("Restore secret", text: $model.restoreSecret)
                    Button {
                        model.restoreProfile()
                    } label: {
                        Label("Restore profile", systemImage: "key.fill")
                    }
                    .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .navigationTitle("Setup")
        }
    }
}

private struct DriveHomeView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        List {
            Section {
                HStack(spacing: 16) {
                    Image(systemName: model.statusSymbol)
                        .font(.system(size: 40, weight: .semibold))
                        .foregroundStyle(model.statusTint)
                        .frame(width: 48)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.driveName)
                            .font(.title3.weight(.semibold))
                        Text(model.statusTitle)
                            .font(.headline)
                            .foregroundStyle(model.statusTint)
                        Text(model.statusDetail)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 8)
            }

            Section("Files") {
                LabeledContent("Provider", value: model.fileProviderStatus)
                Button {
                    model.ensureFileProviderDomain()
                } label: {
                    Label("Refresh Files provider", systemImage: "folder.badge.gearshape")
                }
                Button {
                    model.copySnapshotLink()
                } label: {
                    Label("Copy snapshot link", systemImage: "doc.on.doc")
                }
                Button {
                    model.openSnapshotLink()
                } label: {
                    Label("Open snapshot link", systemImage: "safari")
                }
            }

            Section("Sync") {
                LabeledContent("State", value: model.syncStateTitle)
                Button {
                    model.startSync()
                } label: {
                    Label("Start sync", systemImage: "play.fill")
                }
                Button {
                    model.stopSync()
                } label: {
                    Label("Stop sync", systemImage: "stop.fill")
                }
                Button {
                    model.restartSync()
                } label: {
                    Label("Restart sync", systemImage: "arrow.triangle.2.circlepath")
                }
            }
        }
        .navigationTitle("My Drive")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    model.refresh()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .accessibilityLabel("Refresh")
            }
        }
    }
}

private struct DevicesView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        List {
            Section {
                ForEach(model.devices) { device in
                    DisclosureGroup {
                        Text(device.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    } label: {
                        HStack {
                            Image(systemName: device.isOnline ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(device.isOnline ? .green : .secondary)
                            VStack(alignment: .leading) {
                                Text(device.label)
                                Text(device.state)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .swipeActions {
                        if device.canRevoke {
                            Button(role: .destructive) {
                                model.revokeDevice(label: device.label)
                            } label: {
                                Label("Revoke", systemImage: "trash")
                            }
                        }
                    }
                }
            }

            Section("Approve Device") {
                TextField("Device request", text: $model.approveDeviceKey)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("Label", text: $model.approveDeviceLabel)
                Button {
                    model.approveDevice()
                } label: {
                    Label("Approve device", systemImage: "checkmark.circle")
                }
                .disabled(model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .navigationTitle("Devices")
    }
}

private struct BackupsView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        List {
            Section {
                ForEach(model.backups) { backup in
                    DisclosureGroup {
                        Text(backup.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(backup.label)
                            Text(backup.state)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .navigationTitle("Backups")
    }
}

private struct SettingsView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section("Account") {
                TextField("Device label", text: $model.deviceLabel)
                    .onSubmit { model.persist() }
                Button {
                    model.copyOwnerKey()
                } label: {
                    Label("Copy owner key", systemImage: "doc.on.doc")
                }
                Button {
                    model.copyDeviceKey()
                } label: {
                    Label("Copy device key", systemImage: "doc.on.doc")
                }
                SecureField("Restore secret", text: $model.restoreSecret)
                Button {
                    model.restoreProfile()
                } label: {
                    Label("Restore profile", systemImage: "key.fill")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Network") {
                TextField("Relay", text: $model.relay)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.persist() }
                Button {
                    model.resetRelay()
                } label: {
                    Label("Reset relay", systemImage: "arrow.counterclockwise")
                }
                Toggle("Sync over cellular", isOn: $model.syncOverCellular)
                    .onChange(of: model.syncOverCellular) { _, _ in
                        model.persist()
                    }
            }

            Section("Advanced") {
                LabeledContent("App group", value: IrisDriveSharedContainer.appGroupIdentifier)
                LabeledContent("Runtime path", value: model.sharedContainerPath)
                Button(role: .destructive) {
                    model.resetLocalState()
                } label: {
                    Label("Reset local state", systemImage: "trash")
                }
            }
        }
        .navigationTitle("Settings")
    }
}
