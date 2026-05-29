import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import UIKit

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
            .sheet(isPresented: $model.isDriveBrowserPresented) {
                DriveFolderBrowser(initialDirectoryURL: model.driveBrowserInitialURL)
            }
        }
    }
}

private enum SetupRoute: Hashable {
    case create
    case photo(String)
    case signIn
    case link
}

private struct SetupWelcomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var path: [SetupRoute] = []

    var body: some View {
        NavigationStack(path: $path) {
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
                    Button {
                        path.append(.create)
                    } label: {
                        Label("Create profile", systemImage: "plus")
                    }
                    Button {
                        path.append(.signIn)
                    } label: {
                        Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .navigationTitle("Setup")
            .navigationDestination(for: SetupRoute.self) { route in
                switch route {
                case .create:
                    CreateProfileSetupView(model: model) { username in
                        if username.isEmpty {
                            model.createProfile(username: "", profilePhotoName: "")
                        } else {
                            path.append(.photo(username))
                        }
                    }
                case .photo(let username):
                    ProfilePhotoSetupView(model: model, username: username)
                case .signIn:
                    SignInSetupView(model: model) {
                        path.append(.link)
                    }
                case .link:
                    LinkDeviceSetupView(model: model)
                }
            }
        }
    }
}

private struct CreateProfileSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let continueWithUsername: (String) -> Void
    @State private var username = ""

    private var trimmedUsername: String {
        username.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        Form {
            Section {
                TextField("Username (optional)", text: $username)
                    .textInputAutocapitalization(.words)
                Button {
                    continueWithUsername(trimmedUsername)
                } label: {
                    Label(
                        trimmedUsername.isEmpty ? "Create profile" : "Continue",
                        systemImage: "plus"
                    )
                }
            }
        }
        .navigationTitle("Create profile")
    }
}

private struct ProfilePhotoSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let username: String
    @State private var selectedPhoto: PhotosPickerItem?

    var body: some View {
        Form {
            Section {
                PhotosPicker(selection: $selectedPhoto, matching: .images) {
                    Label(
                        selectedPhoto == nil ? "Choose photo" : "Photo selected",
                        systemImage: "photo"
                    )
                }
                if selectedPhoto != nil {
                    Button {
                        selectedPhoto = nil
                    } label: {
                        Label("Remove photo", systemImage: "xmark")
                    }
                }
                Button {
                    model.createProfile(
                        username: username,
                        profilePhotoName: selectedPhoto == nil ? "" : "selected-profile-photo"
                    )
                } label: {
                    Label(selectedPhoto == nil ? "Later" : "Create profile", systemImage: "plus")
                }
            }
        }
        .navigationTitle("Profile photo")
    }
}

private struct SignInSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let openLinkDevice: () -> Void

    var body: some View {
        Form {
            Section {
                SecureField("Secret key", text: $model.restoreSecret)
                Button {
                    model.restoreProfile()
                } label: {
                    Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button(action: openLinkDevice) {
                    Label("Link this device", systemImage: "link")
                }
            }
        }
        .navigationTitle("Sign in")
    }
}

private struct LinkDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var ownerPublicKey = ""
    @State private var scannerPresented = false

    var body: some View {
        Form {
            Section {
                TextField("Owner public key or invite link", text: $ownerPublicKey)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button {
                    model.ownerPublicKey = ownerPublicKey
                    model.linkDevice()
                } label: {
                    Label("Link device", systemImage: "link")
                }
                .disabled(ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    scannerPresented = true
                } label: {
                    Label("Scan invite QR", systemImage: "qrcode.viewfinder")
                }
            }
        }
        .navigationTitle("Link this device")
        .sheet(isPresented: $scannerPresented) {
            QRCodeScannerSheet { code in
                ownerPublicKey = code
            }
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

            Section("Summary") {
                LabeledContent("Files", value: "\(model.fileCount)")
                LabeledContent("Storage", value: byteString(model.visibleFileBytes))
                LabeledContent("Devices", value: "\(model.authorizedDeviceCount)")
            }

            Section("Files") {
                Button {
                    model.openDriveFolder()
                } label: {
                    Label("Open in Files", systemImage: "folder")
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
                    Label("Resume sync", systemImage: "play.fill")
                }
                .disabled(model.syncRunning)
                Button {
                    model.stopSync()
                } label: {
                    Label("Pause sync", systemImage: "pause.fill")
                }
                .disabled(!model.syncRunning)
            }
        }
        .navigationTitle("My Drive")
    }
}

private struct DriveFolderBrowser: UIViewControllerRepresentable {
    let initialDirectoryURL: URL?

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let controller = UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: false)
        controller.allowsMultipleSelection = false
        controller.directoryURL = initialDirectoryURL
        return controller
    }

    func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {
        controller.directoryURL = initialDirectoryURL
    }
}

private struct DevicesView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        List {
            Section {
                ForEach(model.devices) { device in
                    DisclosureGroup {
                        if device.detail == model.devicePublicKey {
                            LabeledContent("Device ID", value: model.devicePublicKey)
                        }
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
                                Text("\(device.role) | \(device.state)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .swipeActions {
                        if device.canAppointAdmin {
                            Button {
                                model.appointAdmin(id: device.id)
                            } label: {
                                Label("Make Admin", systemImage: "person.badge.key")
                            }
                        }
                        if device.canDemoteAdmin {
                            Button {
                                model.demoteAdmin(id: device.id)
                            } label: {
                                Label("Remove Admin", systemImage: "person.badge.minus")
                            }
                        }
                        if device.canRevoke {
                            Button(role: .destructive) {
                                model.revokeDevice(id: device.id)
                            } label: {
                                Label("Revoke", systemImage: "trash")
                            }
                        }
                    }
                }
            }

            if !model.inboundDeviceLinkRequests.isEmpty {
                Section("Device Requests") {
                    ForEach(model.inboundDeviceLinkRequests) { request in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(request.label.isEmpty ? "New device" : request.label)
                                .font(.headline)
                            Text(request.devicePubkey)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                            Button {
                                model.approveDevice(request: request.requestLink, label: request.label)
                            } label: {
                                Label("Approve device", systemImage: "checkmark.circle")
                            }
                        }
                    }
                }
            }

            if !model.deviceLinkInvite.isEmpty {
                Section("Invite Device") {
                    QrCodeView(matrix: model.qrMatrix(for: model.deviceLinkInvite))
                        .frame(width: 260, height: 260)
                        .frame(maxWidth: .infinity, alignment: .center)
                    Text(model.deviceLinkInvite)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    Button {
                        model.copyLinkInvite()
                    } label: {
                        Label("Copy invite link", systemImage: "link")
                    }
                }
            }

            Section("Add Manually") {
                TextField("Device ID", text: $model.approveDeviceKey)
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
                LabeledContent("Owner", value: model.ownerPublicKey)
                LabeledContent("Device", value: model.devicePublicKey)
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
                    Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Network") {
                ForEach(model.relays, id: \.self) { relay in
                    HStack {
                        Text(relay)
                        Spacer()
                        Button(role: .destructive) {
                            model.removeRelay(relay)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel("Remove relay")
                    }
                }
                TextField("Relay URL", text: $model.relayInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addRelay() }
                Button {
                    model.addRelay()
                } label: {
                    Label("Add relay", systemImage: "plus")
                }
                .disabled(model.relayInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    model.resetRelays()
                } label: {
                    Label("Reset relay", systemImage: "arrow.counterclockwise")
                }
                Toggle("Sync over cellular", isOn: $model.syncOverCellular)
                    .onChange(of: model.syncOverCellular) { _, _ in
                        model.persist()
                    }
            }

            Section("Advanced") {
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

private struct QrCodeView: View {
    let matrix: QrMatrix

    var body: some View {
        GeometryReader { proxy in
            Canvas { context, size in
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
                guard matrix.width > 0, matrix.cells.count >= matrix.width * matrix.width else {
                    return
                }
                let quiet = 3
                let modules = matrix.width + quiet * 2
                let cell = min(size.width, size.height) / CGFloat(modules)
                let origin = CGPoint(
                    x: (size.width - cell * CGFloat(modules)) / 2,
                    y: (size.height - cell * CGFloat(modules)) / 2
                )
                for y in 0..<matrix.width {
                    for x in 0..<matrix.width where matrix.cells[y * matrix.width + x] {
                        let rect = CGRect(
                            x: origin.x + CGFloat(x + quiet) * cell,
                            y: origin.y + CGFloat(y + quiet) * cell,
                            width: cell,
                            height: cell
                        )
                        context.fill(Path(rect), with: .color(.black))
                    }
                }
            }
            .background(.white)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .frame(width: proxy.size.width, height: proxy.size.height)
        }
    }
}

private func byteString(_ bytes: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
}
