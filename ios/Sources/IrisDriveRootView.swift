import PhotosUI
import SwiftUI

private enum MainTab: Hashable {
    case drive
    case devices
    case backups
    case settings
}

struct IrisDriveRootView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var selectedTab = MainTab.drive

    var body: some View {
        if !model.isSetupComplete {
            if model.isRevoked {
                RevokedDeviceSetupView(model: model)
            } else if model.isAwaitingApproval {
                AwaitingApprovalSetupView(model: model)
            } else {
                SetupWelcomeView(model: model)
            }
        } else {
            TabView(selection: $selectedTab) {
                NavigationStack {
                    DriveHomeView(model: model) {
                        selectedTab = .devices
                    }
                }
                .tabItem {
                    Label("My Drive", systemImage: "externaldrive.fill")
                }
                .tag(MainTab.drive)

                NavigationStack {
                    DevicesView(model: model)
                }
                .tabItem {
                    Label("Devices", systemImage: "person.2.fill")
                }
                .tag(MainTab.devices)

                NavigationStack {
                    BackupsView(model: model)
                }
                .tabItem {
                    Label("Backups", systemImage: "lock.shield.fill")
                }
                .tag(MainTab.backups)

                NavigationStack {
                    SettingsView(model: model)
                }
                .tabItem {
                    Label("Settings", systemImage: "gearshape.fill")
                }
                .tag(MainTab.settings)
            }
        }
    }
}

private struct RevokedDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

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

                Section("Device removed") {
                    Text("This device no longer has access to Iris Drive.")
                    LabeledContent("Owner", value: model.ownerPublicKey)
                    LabeledContent("This device", value: model.devicePublicKey)
                    Button {
                        model.relinkDevice()
                    } label: {
                        Label("Link this device again", systemImage: "link")
                    }
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy device ID", systemImage: "doc.on.doc")
                    }
                }

                Section {
                    Button(role: .destructive) {
                        model.logout()
                    } label: {
                        Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .accessibilityIdentifier("revokedDeviceView")
            .task {
                while model.isRevoked {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    guard !Task.isCancelled else { return }
                    model.refresh()
                }
            }
        }
    }
}

private struct AwaitingApprovalSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

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

                Section("Waiting for approval") {
                    LabeledContent("Owner", value: model.ownerPublicKey)
                    LabeledContent("This device", value: model.devicePublicKey)
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy device ID", systemImage: "doc.on.doc")
                    }
                }

                Section {
                    Button(role: .destructive) {
                        model.logout()
                    } label: {
                        Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .accessibilityIdentifier("awaitingApprovalView")
            .task {
                while model.isAwaitingApproval {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    guard !Task.isCancelled else { return }
                    model.refresh()
                }
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
                    .accessibilityIdentifier("welcomeCreateProfile")
                    Button {
                        path.append(.signIn)
                    } label: {
                        Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                    .accessibilityIdentifier("welcomeSignIn")
                }
            }
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
                    .accessibilityIdentifier("createUsername")
                    .onSubmit {
                        continueWithUsername(trimmedUsername)
                    }
                Button {
                    continueWithUsername(trimmedUsername)
                } label: {
                    Label(
                        trimmedUsername.isEmpty ? "Create profile" : "Continue",
                        systemImage: "plus"
                    )
                }
                .accessibilityIdentifier("createProfileSubmit")
            }
        }
        .navigationTitle("Create profile")
        .toolbar(.visible, for: .navigationBar)
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
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct SignInSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let openLinkDevice: () -> Void

    var body: some View {
        Form {
            Section {
                SecureField("Secret key", text: $model.restoreSecret)
                    .onSubmit {
                        model.restoreProfile()
                    }
                Button {
                    model.restoreProfile()
                } label: {
                    Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button(action: openLinkDevice) {
                    Label("Link this device", systemImage: "link")
                }
                .accessibilityIdentifier("openLinkDevice")
            }
        }
        .navigationTitle("Sign in")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct LinkDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var ownerPublicKey = ""
    @State private var submittedOwnerPublicKey = ""
    @State private var scannerPresented = false

    init(model: IrisDriveMobileModel) {
        self.model = model
        _ownerPublicKey = State(initialValue: iosUiTestValue("IRIS_DRIVE_UI_TEST_OWNER_INVITE"))
    }

    var body: some View {
        Form {
            Section {
                TextField("Owner public key or invite link", text: $ownerPublicKey)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("linkOwnerInput")
                    .onSubmit {
                        submitLinkDevice(ownerPublicKey, force: true)
                    }
                    .onChange(of: ownerPublicKey) { _, newValue in
                        submitLinkDevice(newValue, force: false)
                    }
                Button {
                    submitLinkDevice(ownerPublicKey, force: true)
                } label: {
                    Label("Link device", systemImage: "link")
                }
                .accessibilityIdentifier("linkDeviceSubmit")
                .disabled(ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    scannerPresented = true
                } label: {
                    Label("Scan invite QR", systemImage: "qrcode.viewfinder")
                }
            }
        }
        .navigationTitle("Link this device")
        .toolbar(.visible, for: .navigationBar)
        .onAppear {
            submitLinkDevice(ownerPublicKey, force: false)
        }
        .sheet(isPresented: $scannerPresented) {
            QRCodeScannerSheet { code in
                ownerPublicKey = code
                submitLinkDevice(code, force: false)
            }
        }
    }

    private func submitLinkDevice(_ value: String, force: Bool) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard force || isCompleteDeviceLinkOwnerInput(trimmed) else { return }
        guard submittedOwnerPublicKey != trimmed else { return }
        submittedOwnerPublicKey = trimmed
        model.ownerPublicKey = trimmed
        model.linkDevice()
    }
}

private struct DriveHomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let showDevices: () -> Void

    private var onlineDeviceCount: Int {
        model.devices.filter(\.isOnline).count
    }

    private var totalDeviceCount: Int {
        max(model.authorizedDeviceCount, model.devices.count)
    }

    private var deviceSummaryText: String {
        "\(onlineDeviceCount)/\(totalDeviceCount) online"
    }

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
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("filesSummaryRow")
                    .accessibilityValue("\(model.fileCount)")
                LabeledContent("Storage", value: byteString(model.visibleFileBytes))
                Button(action: showDevices) {
                    LabeledContent("Devices", value: deviceSummaryText)
                }
                .accessibilityIdentifier("devicesSummaryButton")
                .accessibilityLabel("Devices")
                .accessibilityValue(deviceSummaryText)
            }

            Section("Files") {
                Button {
                    model.openDriveFolder()
                } label: {
                    Label("Open in Files", systemImage: "folder")
                }
                .accessibilityIdentifier("openInFilesButton")
                if !model.fileProviderError.isEmpty {
                    Text(model.fileProviderError)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("openInFilesError")
                }
                Button {
                    model.copySnapshotLink()
                } label: {
                    Label("Copy drive.iris.to link", systemImage: "doc.on.doc")
                }
                .disabled(model.snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    model.openSnapshotLink()
                } label: {
                    Label("View on drive.iris.to", systemImage: "safari")
                }
                .disabled(model.snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Sync") {
                LabeledContent("State", value: model.syncStateTitle)
                if model.syncRunning {
                    Button {
                        model.stopSync()
                    } label: {
                        Label("Pause sync", systemImage: "pause.fill")
                    }
                } else {
                    Button {
                        model.startSync()
                    } label: {
                        Label("Resume sync", systemImage: "play.fill")
                    }
                }
            }
        }
        .navigationTitle("My Drive")
    }
}

private struct DevicesView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var showingAddDevice = false
    @State private var devicePendingDelete: IrisDriveDevice?

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
                                Text("\(device.role) | \(device.state) | \(device.isOnline ? "Online" : "Offline")")
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
                                devicePendingDelete = device
                            } label: {
                                Label("Delete", systemImage: "trash")
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Devices")
        .toolbar {
            if model.hasOwnerAuthority {
                Button {
                    showingAddDevice = true
                } label: {
                    Label("Add Device", systemImage: "plus")
                }
                .accessibilityIdentifier("addDeviceButton")
            }
        }
        .sheet(isPresented: $showingAddDevice) {
            AddDeviceSheet(model: model, isPresented: $showingAddDevice)
        }
        .alert(
            "Delete device?",
            isPresented: Binding(
                get: { devicePendingDelete != nil },
                set: { presented in
                    if !presented {
                        devicePendingDelete = nil
                    }
                }
            ),
            presenting: devicePendingDelete
        ) { device in
            Button("Delete", role: .destructive) {
                model.deleteDevice(id: device.id)
                devicePendingDelete = nil
            }
            Button("Cancel", role: .cancel) {
                devicePendingDelete = nil
            }
        } message: { device in
            Text("Delete \(device.label) from Iris Drive? This removes its access to future syncs.")
        }
    }
}

private struct AddDeviceSheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    @Binding var isPresented: Bool

    private var canAddManualDevice: Bool {
        !model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func submitManualDevice() {
        guard canAddManualDevice else { return }
        model.approveDevice()
        isPresented = false
    }

    var body: some View {
        NavigationStack {
            Form {
                if !model.deviceLinkInvite.isEmpty {
                    Section("Invite device") {
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
                        Button {
                            model.resetInvite()
                        } label: {
                            Label("Reset invite", systemImage: "arrow.clockwise")
                        }
                    }
                }

                if !model.inboundDeviceLinkRequests.isEmpty {
                    Section("Devices asking to join") {
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
                                    Label("Add", systemImage: "plus")
                                }
                            }
                        }
                    }
                }

                Section("Link manually") {
                    Text("Paste the Device ID shown on the other device when you link it manually.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    TextField("Device ID", text: $model.approveDeviceKey)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier("manualDeviceId")
                        .onSubmit {
                            submitManualDevice()
                        }
                    TextField("Name (optional)", text: $model.approveDeviceLabel)
                        .accessibilityIdentifier("manualDeviceName")
                        .onSubmit {
                            submitManualDevice()
                        }
                    Button {
                        submitManualDevice()
                    } label: {
                        Label("Add", systemImage: "plus")
                    }
                    .accessibilityIdentifier("manualDeviceAdd")
                    .disabled(!canAddManualDevice)
                }
            }
            .navigationTitle("Add a device")
            .toolbar {
                Button("Cancel") {
                    isPresented = false
                }
            }
            .onAppear {
                prefillUiTestDeviceFields()
            }
        }
    }

    private func prefillUiTestDeviceFields() {
        let request = iosUiTestValue("IRIS_DRIVE_UI_TEST_LINKED_DEVICE")
        if !request.isEmpty,
           model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            model.approveDeviceKey = request
        }

        let label = iosUiTestValue("IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL")
        if !label.isEmpty,
           model.approveDeviceLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            model.approveDeviceLabel = label
        }
    }
}

private func iosUiTestValue(_ name: String) -> String {
    #if DEBUG
    ProcessInfo.processInfo.environment[name] ?? ""
    #else
    ""
    #endif
}

private func isCompleteDeviceLinkOwnerInput(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.contains(where: { $0.isWhitespace }) else { return false }
    let lower = trimmed.lowercased()
    if lower.hasPrefix("npub1") {
        return lower.count >= 63
    }
    if lower.count == 64, lower.unicodeScalars.allSatisfy(isAsciiHexDigit) {
        return true
    }
    for prefix in [
        "iris-drive://invite/",
        "iris-drive:/invite/",
        "https://drive.iris.to/invite/",
    ] where lower.hasPrefix(prefix) {
        return lower.dropFirst(prefix.count).count >= 32
    }
    if lower.hasPrefix("iris-drive://link-device?")
        || lower.hasPrefix("iris-drive:/link-device?")
        || lower.hasPrefix("https://drive.iris.to/link-device?") {
        return lower.contains("owner=") && lower.contains("admin=") && lower.contains("secret=")
    }
    return false
}

private func isAsciiHexDigit(_ scalar: Unicode.Scalar) -> Bool {
    (48...57).contains(scalar.value) || (97...102).contains(scalar.value)
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
    @State private var showingLogoutConfirmation = false

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
                Button(role: .destructive) {
                    showingLogoutConfirmation = true
                } label: {
                    Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                }
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
        .confirmationDialog(
            "Log out of Iris Drive on this device?",
            isPresented: $showingLogoutConfirmation,
            titleVisibility: .visible
        ) {
            Button("Log out", role: .destructive) {
                model.logout()
            }
            Button("Cancel", role: .cancel) {}
        }
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
