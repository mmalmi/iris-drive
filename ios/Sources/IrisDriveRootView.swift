import PhotosUI
import SwiftUI
import UIKit

private let recoveryPhraseWordCount = 12

private enum MainTab: Hashable {
    case drive
    case devices
    case shares
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
                    Label("AppKeys", systemImage: "person.2.fill")
                }
                .tag(MainTab.devices)

                NavigationStack {
                    SharesView(model: model)
                }
                .tabItem {
                    Label("Shares", systemImage: "person.3.fill")
                }
                .tag(MainTab.shares)

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

                Section("AppKey removed") {
                    Text("This app install no longer has access to Iris Drive.")
                    LabeledContent("AppKey", value: model.currentAppKeyNpub)
                    LabeledContent("Current AppKey", value: model.devicePublicKey)
                    Button {
                        model.relinkDevice()
                    } label: {
                        Label("Link this app install again", systemImage: "link")
                    }
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy AppKey", systemImage: "doc.on.doc")
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
                    LabeledContent("AppKey", value: model.currentAppKeyNpub)
                    LabeledContent("Current AppKey", value: model.devicePublicKey)
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy AppKey", systemImage: "doc.on.doc")
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
    case restoreOptions
    case restorePhrase
    case restoreSecretKey
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
                        path.append(.restoreOptions)
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
                case .restoreOptions:
                    RestoreOptionsSetupView(
                        openLinkDevice: { path.append(.link) },
                        openRecoveryPhrase: { path.append(.restorePhrase) },
                        openSecretKey: { path.append(.restoreSecretKey) }
                    )
                case .restorePhrase:
                    RestoreRecoveryPhraseSetupView(model: model)
                case .restoreSecretKey:
                    RestoreSecretKeySetupView(model: model)
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

private struct RestoreOptionsSetupView: View {
    let openLinkDevice: () -> Void
    let openRecoveryPhrase: () -> Void
    let openSecretKey: () -> Void

    var body: some View {
        Form {
            Section {
                Button(action: openLinkDevice) {
                    Label("Link app install", systemImage: "link")
                }
                .accessibilityIdentifier("openLinkDevice")
                Button(action: openRecoveryPhrase) {
                    Label("Restore from recovery phrase", systemImage: "text.badge.checkmark")
                }
                .accessibilityIdentifier("openRecoveryPhrase")
                Button(action: openSecretKey) {
                    Label("Restore from secret key", systemImage: "key")
                }
                .accessibilityIdentifier("openSecretKey")
            }
        }
        .navigationTitle("Restore")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct RestoreRecoveryPhraseSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var words = Array(repeating: "", count: recoveryPhraseWordCount)
    @State private var index = 0
    @FocusState private var wordFieldFocused: Bool

    private var currentWord: Binding<String> {
        Binding(
            get: { words[index] },
            set: { value in
                applyInput(value)
            }
        )
    }

    private var phrase: String {
        words
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .joined(separator: " ")
    }

    private var currentWordIsFilled: Bool {
        !words[index].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var allWordsAreFilled: Bool {
        words.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    var body: some View {
        Form {
            Section("Word \(index + 1) of \(recoveryPhraseWordCount)") {
                TextField("Word \(index + 1)", text: currentWord)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .focused($wordFieldFocused)
                    .submitLabel(index == recoveryPhraseWordCount - 1 ? .done : .next)
                    .accessibilityIdentifier("recoveryWordInput")
                    .onSubmit { advanceOrRestore() }

                Button {
                    applyInput(UIPasteboard.general.string ?? "")
                } label: {
                    Label("Paste from Clipboard", systemImage: "doc.on.clipboard")
                }
                .accessibilityIdentifier("pasteRecoveryPhrase")

                HStack {
                    Button {
                        index = max(0, index - 1)
                    } label: {
                        Label("Back", systemImage: "chevron.left")
                    }
                    .disabled(index == 0)

                    Spacer()

                    Button {
                        advanceOrRestore()
                    } label: {
                        Label(
                            index == recoveryPhraseWordCount - 1 ? "Restore" : "Next",
                            systemImage: index == recoveryPhraseWordCount - 1 ? "checkmark" : "chevron.right"
                        )
                    }
                    .disabled(index == recoveryPhraseWordCount - 1 ? !allWordsAreFilled : !currentWordIsFilled)
                    .accessibilityIdentifier(index == recoveryPhraseWordCount - 1 ? "restoreRecoveryPhraseSubmit" : "restoreRecoveryPhraseNext")
                }
            }
        }
        .navigationTitle("Recovery phrase")
        .toolbar(.visible, for: .navigationBar)
        .onAppear { wordFieldFocused = true }
    }

    private func advanceOrRestore() {
        if index == recoveryPhraseWordCount - 1 {
            guard allWordsAreFilled else { return }
            model.restoreProfile(recoverySecret: phrase)
        } else if currentWordIsFilled {
            index += 1
            wordFieldFocused = true
        }
    }

    private func applyInput(_ value: String) {
        let parts = value
            .split(whereSeparator: { $0.isWhitespace })
            .map { String($0).lowercased() }
        if parts.count <= 1 {
            words[index] = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return
        }
        for (offset, word) in parts.enumerated() where index + offset < words.count {
            words[index + offset] = word
        }
        index = min(words.count - 1, index + parts.count - 1)
    }
}

private struct RestoreSecretKeySetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section {
                SecureField("nsec1... or hex secret key", text: $model.restoreSecret)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit {
                        model.restoreProfile()
                    }
                    .accessibilityIdentifier("restoreSecretKeyInput")
                Button {
                    model.restoreProfile()
                } label: {
                    Label("Restore", systemImage: "key")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("restoreSecretKeySubmit")
            }
        }
        .navigationTitle("Secret key")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct LinkDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var linkTarget = ""
    @State private var submittedLinkTarget = ""
    @State private var scannerPresented = false

    init(model: IrisDriveMobileModel) {
        self.model = model
        _linkTarget = State(initialValue: iosUiTestValue("IRIS_DRIVE_UI_TEST_OWNER_INVITE"))
    }

    var body: some View {
        Form {
            Section {
                TextField("IrisProfile invite link or admin AppKey", text: $linkTarget)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("linkTargetInput")
                    .onSubmit {
                        submitLinkDevice(linkTarget, force: true)
                    }
                    .onChange(of: linkTarget) { _, newValue in
                        submitLinkDevice(newValue, force: false)
                    }
                Button {
                    submitLinkDevice(linkTarget, force: true)
                } label: {
                    Label("Link app install", systemImage: "link")
                }
                .accessibilityIdentifier("linkDeviceSubmit")
                .disabled(!IrisDriveNativeLinkInput.isComplete(linkTarget.trimmingCharacters(in: .whitespacesAndNewlines)))
                Button {
                    scannerPresented = true
                } label: {
                    Label("Scan invite QR", systemImage: "qrcode.viewfinder")
                }
            }
        }
        .navigationTitle("Link app install")
        .toolbar(.visible, for: .navigationBar)
        .onAppear {
            submitLinkDevice(linkTarget, force: false)
        }
        .sheet(isPresented: $scannerPresented) {
            QRCodeScannerSheet { code in
                linkTarget = code
                submitLinkDevice(code, force: false)
            }
        }
    }

    private func submitLinkDevice(_ value: String, force _: Bool) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard IrisDriveNativeLinkInput.isComplete(trimmed) else { return }
        guard submittedLinkTarget != trimmed else { return }
        submittedLinkTarget = trimmed
        model.profileLinkTarget = trimmed
        model.linkDevice()
    }
}

private struct DriveHomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let showDevices: () -> Void

    private var onlineDeviceCount: Int {
        model.onlineDeviceCount
    }

    private var totalDeviceCount: Int {
        model.authorizedDeviceCount
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
                    LabeledContent("AppKeys", value: deviceSummaryText)
                }
                .accessibilityIdentifier("devicesSummaryButton")
                .accessibilityLabel("AppKeys")
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
                            LabeledContent("AppKey", value: model.devicePublicKey)
                        }
                        Text(device.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        if device.canAppointAdmin || device.canDemoteAdmin || device.canRevoke {
                            HStack {
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
                                        Label("Remove", systemImage: "trash")
                                    }
                                }
                            }
                            .buttonStyle(.bordered)
                        }
                    } label: {
                        HStack {
                            Image(systemName: device.isOnline ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(device.isOnline ? .green : .secondary)
                            VStack(alignment: .leading) {
                                Text(device.label)
                                Text("\(device.role) | \(device.state) | \(device.connectionLabel)")
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
        .navigationTitle("AppKeys")
        .toolbar {
            if model.canAdminProfile {
                Button {
                    showingAddDevice = true
                } label: {
                    Label("Add AppKey", systemImage: "plus")
                }
                .accessibilityIdentifier("addDeviceButton")
            }
        }
        .sheet(isPresented: $showingAddDevice) {
            AddDeviceSheet(model: model, isPresented: $showingAddDevice)
        }
        .alert(
            "Remove AppKey?",
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
            Button("Remove", role: .destructive) {
                model.deleteDevice(id: device.id)
                devicePendingDelete = nil
            }
            Button("Cancel", role: .cancel) {
                devicePendingDelete = nil
            }
        } message: { device in
            Text("Remove \(device.label) from Iris Drive? This removes its access to future syncs.")
        }
    }
}

private struct AddDeviceSheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    @Binding var isPresented: Bool

    private var canAddManualDevice: Bool {
        IrisDriveNativeLinkInput.isComplete(model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private func submitManualDevice() {
        guard canAddManualDevice else { return }
        model.approveDevice()
        isPresented = false
    }

    var body: some View {
        NavigationStack {
            Form {
                if !model.appKeyLinkInvite.isEmpty {
                    Section("Invite app install") {
                        QrCodeView(matrix: model.qrMatrix(for: model.appKeyLinkInvite))
                            .frame(width: 260, height: 260)
                            .frame(maxWidth: .infinity, alignment: .center)
                        Text(model.appKeyLinkInvite)
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

                if !model.inboundAppKeyLinkRequests.isEmpty {
                    Section("AppKeys asking to join") {
                        ForEach(model.inboundAppKeyLinkRequests) { request in
                            VStack(alignment: .leading, spacing: 8) {
                                Text(request.label.isEmpty ? "New AppKey" : request.label)
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
                                Button(role: .destructive) {
                                    model.rejectDevice(request: request.requestLink)
                                } label: {
                                    Label("Reject", systemImage: "xmark")
                                }
                            }
                        }
                    }
                }

                Section("Link manually") {
                    Text("Paste the AppKey shown by the app install you want to approve.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    TextField("AppKey", text: $model.approveDeviceKey)
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
            .navigationTitle("Add an AppKey")
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

private struct SharesView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section("Create Share") {
                TextField("Folder path", text: $model.shareSourceInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.createShare() }
                TextField("Name", text: $model.shareNameInput)
                    .onSubmit { model.createShare() }
                Button {
                    model.createShare()
                } label: {
                    Label("Create share", systemImage: "plus")
                }
                .disabled(model.shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Accept Invite") {
                TextField("Share invite", text: $model.shareInviteInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.acceptShareInvite() }
                Button {
                    model.acceptShareInvite()
                } label: {
                    Label("Accept invite", systemImage: "tray.and.arrow.down.fill")
                }
                .disabled(model.shareInviteInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    model.copyLastShareInvite()
                } label: {
                    Label("Copy last invite", systemImage: "doc.on.doc")
                }
                .disabled(model.lastShareInvite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Shared Folders") {
                if model.shares.isEmpty {
                    Text("No shared folders")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.shares) { share in
                    ShareRow(model: model, share: share)
                }
            }
        }
        .navigationTitle("Shares")
    }
}

private struct ShareRow: View {
    @ObservedObject var model: IrisDriveMobileModel
    let share: IrisDriveShare
    @State private var showingInvite = false
    @State private var revokeTarget: IrisDriveShareMember?

    var body: some View {
        DisclosureGroup {
            LabeledContent("Role", value: share.roleLabel.isEmpty ? share.role : share.roleLabel)
            LabeledContent("Key", value: share.keyStatusLabel.isEmpty ? share.keyStatus : share.keyStatusLabel)
            if let epoch = share.currentKeyEpoch {
                LabeledContent("Epoch", value: "\(epoch)")
            }
            if let shortcut = share.shortcutPaths.first {
                LabeledContent("Shortcut", value: shortText(shortcut))
            }
            HStack {
                if share.canAdmin {
                    Button {
                        showingInvite = true
                    } label: {
                        Label("Invite", systemImage: "person.badge.plus")
                    }
                }
                if share.repairNeeded || !share.missingKeyWraps.isEmpty {
                    Button {
                        model.repairShareWraps(shareId: share.shareId)
                    } label: {
                        Label("Repair", systemImage: "arrow.triangle.2.circlepath")
                    }
                }
                if share.shortcutPaths.isEmpty {
                    Button {
                        model.addShareShortcut(shareId: share.shareId, displayName: shareDisplayName(share))
                    } label: {
                        Label("Shortcut", systemImage: "link")
                    }
                }
            }
            .buttonStyle(.bordered)
            ForEach(share.members) { member in
                HStack {
                    VStack(alignment: .leading) {
                        Text(memberDisplayName(member))
                        Text(memberMetadata(member))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if share.canAdmin,
                       member.status != "revoked",
                       member.profileId != model.localProfileId {
                        Button(role: .destructive) {
                            revokeTarget = member
                        } label: {
                            Label("Revoke", systemImage: "trash")
                        }
                        .labelStyle(.iconOnly)
                    }
                }
            }
        } label: {
            VStack(alignment: .leading) {
                Text(shareDisplayName(share))
                Text("\(share.participantCount) people | \(share.keyStatusLabel.isEmpty ? share.keyStatus : share.keyStatusLabel)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .sheet(isPresented: $showingInvite) {
            InviteShareMemberSheet(model: model, share: share)
        }
        .alert(
            "Revoke access?",
            isPresented: Binding(
                get: { revokeTarget != nil },
                set: { presented in
                    if !presented {
                        revokeTarget = nil
                    }
                }
            ),
            presenting: revokeTarget
        ) { member in
            Button("Revoke", role: .destructive) {
                model.revokeShareMember(shareId: share.shareId, profileId: member.profileId)
                revokeTarget = nil
            }
            Button("Cancel", role: .cancel) {
                revokeTarget = nil
            }
        } message: { member in
            Text("Revoke \(memberDisplayName(member)) from \(shareDisplayName(share))?")
        }
    }
}

private struct InviteShareMemberSheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    let share: IrisDriveShare
    @Environment(\.dismiss) private var dismiss
    @State private var evidenceJson = ""
    @State private var profileId = ""
    @State private var appKey = ""
    @State private var role = "reader"
    @State private var representativeNpubHint = ""
    @State private var displayName = ""
    @State private var label = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Recipient Evidence") {
                    TextEditor(text: $evidenceJson)
                        .font(.system(.body, design: .monospaced))
                        .frame(minHeight: 120)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section("Member") {
                    TextField("IrisProfile UUID", text: $profileId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Recipient AppKey", text: $appKey)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Picker("Role", selection: $role) {
                        Text("Reader").tag("reader")
                        Text("Editor").tag("editor")
                        Text("Admin").tag("admin")
                    }
                }
                Section("Contact") {
                    TextField("Representative npub", text: $representativeNpubHint)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Name", text: $displayName)
                    TextField("AppKey label", text: $label)
                }
            }
            .navigationTitle("Invite")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Invite") {
                        if evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            model.inviteShareMember(
                                shareId: share.shareId,
                                profileId: profileId,
                                appKey: appKey,
                                role: role,
                                representativeNpubHint: representativeNpubHint,
                                displayName: displayName,
                                label: label
                            )
                        } else {
                            model.inviteShareMemberFromEvidence(
                                shareId: share.shareId,
                                evidenceJson: evidenceJson,
                                role: role,
                                displayName: displayName
                            )
                        }
                        dismiss()
                    }
                    .disabled(
                        evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
                            (
                                profileId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                                    appKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            )
                    )
                }
            }
        }
    }
}

private struct BackupsView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section("Actions") {
                Button {
                    model.syncBackups()
                } label: {
                    Label("Sync Now", systemImage: "arrow.up.circle")
                }
                .disabled(model.backups.isEmpty)
                Button {
                    model.checkBackups()
                } label: {
                    Label("Check All", systemImage: "checkmark.shield")
                }
                .disabled(model.backups.isEmpty)
            }

            Section("Add Backup") {
                TextField("Destination", text: $model.backupTargetInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addBackupTarget() }
                TextField("Name", text: $model.backupLabelInput)
                    .onSubmit { model.addBackupTarget() }
                Button {
                    model.addBackupTarget()
                } label: {
                    Label("Add Backup", systemImage: "plus")
                }
                .disabled(model.backupTargetInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Blossom") {
                TextField("Endpoint URL", text: $model.blossomEndpointInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addBlossomServer() }
                Button {
                    model.addBlossomServer()
                } label: {
                    Label("Add Blossom", systemImage: "plus")
                }
                .disabled(model.blossomEndpointInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Targets") {
                ForEach(model.backups) { backup in
                    DisclosureGroup {
                        Text(backup.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                        Button {
                            model.checkBackups(backup.target)
                        } label: {
                            Label("Check", systemImage: "checkmark.shield")
                        }
                        Button(role: .destructive) {
                            model.removeBackupTarget(backup.target)
                        } label: {
                            Label("Remove backup", systemImage: "trash")
                        }
                        if backup.kind == "blossom" {
                            Button(role: .destructive) {
                                model.removeBlossomServer(backup.target)
                            } label: {
                                Label("Remove Blossom", systemImage: "xmark.circle")
                            }
                        }
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

private struct RecoveryPhraseExportView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var export = NativeRecoverySecretExport()
    @State private var index = 0
    @State private var copied = false

    private var currentWord: String {
        guard export.words.indices.contains(index) else { return "" }
        return export.words[index]
    }

    var body: some View {
        Form {
            if !export.error.isEmpty {
                Section {
                    Text(export.error)
                        .foregroundStyle(.secondary)
                }
            } else if export.words.count == recoveryPhraseWordCount {
                Section("Word \(index + 1) of \(recoveryPhraseWordCount)") {
                    Text(currentWord)
                        .font(.title.monospaced().bold())
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.vertical, 16)
                        .accessibilityIdentifier("recoveryPhraseWord")
                    HStack {
                        Button {
                            index = max(0, index - 1)
                        } label: {
                            Label("Back", systemImage: "chevron.left")
                        }
                        .disabled(index == 0)

                        Spacer()

                        Button {
                            index = min(recoveryPhraseWordCount - 1, index + 1)
                        } label: {
                            Label("Next", systemImage: "chevron.right")
                        }
                        .disabled(index == recoveryPhraseWordCount - 1)
                    }
                }
                Section {
                    Button {
                        UIPasteboard.general.string = export.recoveryPhrase
                        copied = true
                    } label: {
                        Label(copied ? "Copied" : "Copy recovery phrase", systemImage: "doc.on.doc")
                    }
                    .accessibilityIdentifier("copyRecoveryPhrase")
                    Button {
                        UIPasteboard.general.string = export.secretKey
                    } label: {
                        Label("Copy secret key", systemImage: "key")
                    }
                }
            }
        }
        .navigationTitle("Recovery phrase")
        .task {
            export = model.exportRecoverySecret()
        }
    }
}

private struct SettingsView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var showingLogoutConfirmation = false

    var body: some View {
        Form {
            Section("Account") {
                TextField("App install label", text: $model.deviceLabel)
                    .onSubmit { model.persist() }
                LabeledContent("AppKey", value: model.currentAppKeyNpub)
                LabeledContent("Current AppKey", value: model.devicePublicKey)
                Button {
                    model.copyAppKey()
                } label: {
                    Label("Copy AppKey", systemImage: "doc.on.doc")
                }
                Button {
                    model.copyDeviceKey()
                } label: {
                    Label("Copy AppKey", systemImage: "doc.on.doc")
                }
                if model.canExportRecoveryPhrase {
                    NavigationLink {
                        RecoveryPhraseExportView(model: model)
                    } label: {
                        Label("Recovery phrase", systemImage: "text.badge.checkmark")
                    }
                    .accessibilityIdentifier("openRecoveryPhraseExport")
                }
                Button(role: .destructive) {
                    showingLogoutConfirmation = true
                } label: {
                    Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                }
            }

            Section("Network") {
                ForEach(model.relayStatuses) { relay in
                    HStack {
                        Circle()
                            .fill(relayHealthColor(relay.health))
                            .frame(width: 8, height: 8)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(relay.url)
                            Text(relay.statusLabel)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(role: .destructive) {
                            model.removeRelay(relay.url)
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
            "Log out of Iris Drive on this app install?",
            isPresented: $showingLogoutConfirmation,
            titleVisibility: .visible
        ) {
            Button("Log out", role: .destructive) {
                model.logout()
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func relayHealthColor(_ health: String) -> Color {
        switch health {
        case "online":
            return .green
        case "connecting":
            return .orange
        case "error":
            return .red
        default:
            return .secondary
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

private func shareDisplayName(_ share: IrisDriveShare) -> String {
    share.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Shared folder"
        : share.displayName
}

private func memberDisplayName(_ member: IrisDriveShareMember) -> String {
    member.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "IrisProfile"
        : member.displayName
}

private func memberMetadata(_ member: IrisDriveShareMember) -> String {
    [
        member.roleLabel.isEmpty ? member.role : member.roleLabel,
        member.statusLabel.isEmpty ? member.status : member.statusLabel,
        shortText(member.representativeNpubHint.isEmpty ? member.profileId : member.representativeNpubHint),
    ].joined(separator: " | ")
}

private func shortText(_ value: String) -> String {
    guard value.count > 32 else { return value }
    return "\(value.prefix(14))...\(value.suffix(10))"
}
