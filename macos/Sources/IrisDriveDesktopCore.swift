import Foundation

@_silgen_name("iris_drive_app_new")
private func irisDriveAppNew(
    _ dataDir: UnsafePointer<CChar>,
    _ appVersion: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer?

@_silgen_name("iris_drive_app_free")
private func irisDriveAppFree(_ handle: UnsafeMutableRawPointer?)

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
