import Foundation

@_silgen_name("iris_drive_app_new")
private func irisDriveAppNew(
    _ dataDir: UnsafePointer<CChar>,
    _ appVersion: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer?

@_silgen_name("iris_drive_app_free")
private func irisDriveAppFree(_ handle: UnsafeMutableRawPointer?)

@_silgen_name("iris_drive_app_state_json")
private func irisDriveAppStateJson(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_app_refresh_json")
private func irisDriveAppRefreshJson(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_app_dispatch_json")
private func irisDriveAppDispatchJson(
    _ handle: UnsafeMutableRawPointer?,
    _ actionJson: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_validate_link_input_json")
private func irisDriveValidateLinkInputJson(_ text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_classify_link_input_json")
private func irisDriveClassifyLinkInputJson(_ text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_export_recovery_secret_json")
private func irisDriveExportRecoverySecretJson(_ dataDir: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_generate_recovery_key_json")
private func irisDriveGenerateRecoveryKeyJson() -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_recovery_pubkey_for_phrase_json")
private func irisDriveRecoveryPubkeyForPhraseJson(_ recoveryPhrase: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_link_for_cid_json")
private func irisDriveLinkForCidJson(_ rootCid: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_update_check_json")
private func irisDriveUpdateCheckJson(
    _ dataDir: UnsafePointer<CChar>,
    _ currentVersion: UnsafePointer<CChar>,
    _ mode: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_update_download_json")
private func irisDriveUpdateDownloadJson(
    _ dataDir: UnsafePointer<CChar>,
    _ currentVersion: UnsafePointer<CChar>,
    _ mode: UnsafePointer<CChar>,
    _ downloadDir: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_list_json")
private func irisDriveProviderListJson(_ dataDir: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_mkdir_json")
private func irisDriveProviderMkdirJson(
    _ dataDir: UnsafePointer<CChar>,
    _ path: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_string_free")
private func irisDriveStringFree(_ value: UnsafeMutablePointer<CChar>?)

final class IrisDriveDesktopCore {
    private var handle: UnsafeMutableRawPointer?

    init(dataDir: String, appVersion: String) {
        handle = dataDir.withCString { dataDirPointer in
            appVersion.withCString { versionPointer in
                irisDriveAppNew(dataDirPointer, versionPointer)
            }
        }
    }

    deinit {
        irisDriveAppFree(handle)
    }

    func stateJson() -> String {
        takeString(irisDriveAppStateJson(handle))
    }

    func refreshJson() -> String {
        takeString(irisDriveAppRefreshJson(handle))
    }

    func dispatchJson(_ actionJson: String) -> String {
        actionJson.withCString { pointer in
            takeString(irisDriveAppDispatchJson(handle, pointer))
        }
    }

    static func validateLinkInput(_ text: String) -> Bool {
        let json = text.withCString { pointer in
            takeString(irisDriveValidateLinkInputJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return false
        }
        return payload["is_complete"] as? Bool ?? false
    }

    static func classifyLinkInput(_ text: String) -> [String: Any] {
        let json = text.withCString { pointer in
            takeString(irisDriveClassifyLinkInputJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native link classifier returned invalid JSON"]
        }
        return payload
    }

    static func exportRecoverySecret(dataDir: String) -> [String: Any] {
        let json = dataDir.withCString { pointer in
            takeString(irisDriveExportRecoverySecretJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native recovery export returned invalid JSON"]
        }
        return payload
    }

    static func generateRecoveryKey() -> [String: Any] {
        let json = takeString(irisDriveGenerateRecoveryKeyJson())
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native recovery generator returned invalid JSON"]
        }
        return payload
    }

    static func recoveryPubkeyForPhrase(_ recoveryPhrase: String) -> [String: Any] {
        let json = recoveryPhrase.withCString { pointer in
            takeString(irisDriveRecoveryPubkeyForPhraseJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native recovery importer returned invalid JSON"]
        }
        return payload
    }

    static func driveLinkForCid(_ rootCid: String) -> [String: Any] {
        let json = rootCid.withCString { pointer in
            takeString(irisDriveLinkForCidJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native drive link encoder returned invalid JSON"]
        }
        return payload
    }

    static func updateCheck(dataDir: String, currentVersion: String, mode: String = "app") -> [String: Any] {
        let json = dataDir.withCString { dataDirPointer in
            currentVersion.withCString { versionPointer in
                mode.withCString { modePointer in
                    takeString(irisDriveUpdateCheckJson(dataDirPointer, versionPointer, modePointer))
                }
            }
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native updater returned invalid JSON"]
        }
        return payload
    }

    static func updateDownload(
        dataDir: String,
        currentVersion: String,
        mode: String = "app",
        downloadDir: String
    ) -> [String: Any] {
        let json = dataDir.withCString { dataDirPointer in
            currentVersion.withCString { versionPointer in
                mode.withCString { modePointer in
                    downloadDir.withCString { downloadDirPointer in
                        takeString(irisDriveUpdateDownloadJson(
                            dataDirPointer,
                            versionPointer,
                            modePointer,
                            downloadDirPointer
                        ))
                    }
                }
            }
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native updater returned invalid JSON"]
        }
        return payload
    }

    static func providerList(dataDir: String) -> [String: Any] {
        let json = dataDir.withCString { pointer in
            takeString(irisDriveProviderListJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native provider list returned invalid JSON"]
        }
        return payload
    }

    static func providerMkdir(dataDir: String, path: String) -> [String: Any] {
        let json = dataDir.withCString { dataDirPointer in
            path.withCString { pathPointer in
                takeString(irisDriveProviderMkdirJson(dataDirPointer, pathPointer))
            }
        }
        guard let data = json.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["error": "native provider mkdir returned invalid JSON"]
        }
        return payload
    }

    private func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        Self.takeString(pointer)
    }

    private static func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native app-core returned null"}"# }
        defer { irisDriveStringFree(pointer) }
        return String(cString: pointer)
    }
}

extension IrisDriveDesktopCore: @unchecked Sendable {}
