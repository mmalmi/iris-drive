import FileProvider
import Foundation

struct FileProviderRuntimeConfig: Codable {
    let configDirectory: String
    let idriveExecutable: String?

    var domainUserInfo: [String: String] {
        var userInfo = ["config_dir": configDirectory]
        if let idriveExecutable, !idriveExecutable.isEmpty {
            userInfo["idrive_executable"] = idriveExecutable
        }
        return userInfo
    }

    enum CodingKeys: String, CodingKey {
        case configDirectory = "config_dir"
        case idriveExecutable = "idrive_executable"
    }
}

enum FileProviderDomainState {
    case unknown
    case registered
    case disabled
    case unavailable
}

private func currentFileProviderRegistrationIdentity() -> String {
    let profileIdentity = currentFileProviderProfileRegistrationIdentity()
    guard !profileIdentity.isEmpty else {
        return ""
    }
    return profileIdentity + "|" + currentFileProviderAppRegistrationIdentity()
}

private func currentFileProviderProfileRegistrationIdentity() -> String {
    let status = IrisDriveStatus.shared
    guard status.setupComplete else {
        return ""
    }

    let appKey = (status.currentAppKeyNpub ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let device = (status.deviceNpub ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    if !appKey.isEmpty && !device.isEmpty {
        return "\(appKey):\(device)"
    }

    let configDirectory = (status.configDirectory ?? "")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if !configDirectory.isEmpty {
        return "config:\(configDirectory)"
    }
    return ""
}

private func currentFileProviderAppRegistrationIdentity() -> String {
    let bundle = Bundle.main
    let appURL = bundle.bundleURL.standardizedFileURL.resolvingSymlinksInPath()
    let bundleIdentifier = bundle.bundleIdentifier ?? "unknown"
    let version = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    let build = bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0"
    let providerURL = bundle.builtInPlugInsURL?
        .appendingPathComponent("IrisDriveFileProvider.appex", isDirectory: true)
        .standardizedFileURL
        .resolvingSymlinksInPath()
    let providerPath = providerURL?.path ?? "missing"
    return "app:\(bundleIdentifier):\(version):\(build):\(appURL.path):provider:\(providerPath)"
}

func fileProviderRegistrationIdentityIsCurrent() -> Bool {
    let identity = currentFileProviderRegistrationIdentity()
    guard !identity.isEmpty else {
        return false
    }
    return UserDefaults.standard.string(
        forKey: irisDriveFileProviderRegistrationIdentityKey
    ) == identity
}

private func markFileProviderRegistrationCurrent(_ identity: String) {
    if identity.isEmpty {
        UserDefaults.standard.removeObject(forKey: irisDriveFileProviderRegistrationIdentityKey)
    } else {
        UserDefaults.standard.set(identity, forKey: irisDriveFileProviderRegistrationIdentityKey)
    }
}

private func clearFileProviderRegistrationIdentity() {
    UserDefaults.standard.removeObject(forKey: irisDriveFileProviderRegistrationIdentityKey)
}

func ensureFileProviderDomainRegistered(
    attempt: Int = 1,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    addFileProviderDomain(attempt: attempt, runtime: runtime, completion)
}

func irisDriveFileProviderDomain(
    runtime: FileProviderRuntimeConfig? = nil
) -> NSFileProviderDomain {
    let domain = NSFileProviderDomain(
        identifier: irisDriveDomainIdentifier,
        displayName: irisDriveFileProviderDomainDisplayName
    )
    if let runtime, #available(macOS 15.0, *) {
        domain.userInfo = runtime.domainUserInfo
    }
    if currentProcessHasEntitlement("com.apple.developer.fileprovider.testing-mode") {
        domain.testingModes = [.alwaysEnabled]
    }
    return domain
}

func resetFileProviderDomain(
    reason: String,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    let domain = irisDriveFileProviderDomain(runtime: runtime)
    irisDriveDebugLog("Iris Drive FileProvider domain reset: \(reason)")
    let finish: (Error?) -> Void = { error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider domain remove during reset failed: \(error)")
        } else {
            irisDriveDebugLog("Iris Drive FileProvider domain removed during reset")
        }
        ensureFileProviderDomainRegistered(runtime: runtime, completion)
    }
    removeFileProviderDomain(domain, reason: reason, finish)
}

func resetAllFileProviderDomains(
    reason: String,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    let domain = irisDriveFileProviderDomain(runtime: runtime)
    let registrationIdentity = currentFileProviderRegistrationIdentity()
    clearFileProviderRegistrationIdentity()
    irisDriveDebugLog("Iris Drive FileProvider all-domain reset: \(reason)")
    NSFileProviderManager.removeAllDomains { error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider all-domain reset failed: \(error)")
            completion(.unavailable)
            return
        }
        addFreshFileProviderDomain(
            domain,
            currentIdentity: registrationIdentity,
            reason: reason,
            completion
        )
    }
}

func removeFileProviderDomainRegistration(
    reason: String,
    runtime: FileProviderRuntimeConfig? = nil
) {
    let domain = irisDriveFileProviderDomain(runtime: runtime)
    clearFileProviderRegistrationIdentity()
    irisDriveDebugLog("Iris Drive FileProvider domain remove requested: \(reason)")
    let finish: (Error?) -> Void = { error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider domain remove without re-add failed: \(error)")
        } else {
            irisDriveDebugLog("Iris Drive FileProvider domain removed without re-add")
        }
    }
    removeFileProviderDomain(domain, reason: reason, finish)
}

private func removeFileProviderDomain(
    _ domain: NSFileProviderDomain,
    reason: String,
    _ completion: @escaping (Error?) -> Void
) {
    let finish: (Error?) -> Void = { error in
        guard let error, shouldRepairFileProviderDomain(after: error) else {
            completion(error)
            return
        }
        irisDriveDebugLog(
            "Iris Drive FileProvider domain remove failed; removing all domains for repair: \(reason): \(error)"
        )
        NSFileProviderManager.removeAllDomains { removeAllError in
            if let removeAllError {
                irisDriveDebugLog(
                    "Iris Drive FileProvider remove all domains for repair failed: \(removeAllError)"
                )
                completion(removeAllError)
            } else {
                irisDriveDebugLog("Iris Drive FileProvider all domains removed for repair")
                completion(nil)
            }
        }
    }
    if #available(macOS 13.0, *) {
        NSFileProviderManager.remove(domain, mode: .removeAll) { _, error in
            finish(error)
        }
        return
    }
    NSFileProviderManager.remove(domain) { error in
        finish(error)
    }
}

private func addFileProviderDomain(
    attempt: Int,
    runtime: FileProviderRuntimeConfig,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    irisDriveDebugLog(
        "Iris Drive FileProvider registration attempt \(attempt) " +
        "config=\(runtime.configDirectory) idrive=\(runtime.idriveExecutable ?? "nil")"
    )
    let domain = irisDriveFileProviderDomain(runtime: runtime)
    let registrationIdentity = currentFileProviderRegistrationIdentity()

    NSFileProviderManager.add(domain) { error in
        if let error {
            NSFileProviderManager.getDomainsWithCompletionHandler { domains, queryError in
                if let queryError {
                    irisDriveDebugLog("Iris Drive FileProvider domain query failed: \(queryError)")
                    if shouldRepairFileProviderDomain(after: queryError) {
                        repairAllFileProviderRegistrations(
                            reason: "domain query failed after add error",
                            runtime: runtime,
                            currentIdentity: registrationIdentity,
                            completion
                        )
                        return
                    }
                }
                if let existingDomain = domains.first(where: {
                    $0.identifier == irisDriveDomainIdentifier
                }) {
                    if shouldRepairFileProviderRegistration(
                        existingDomain,
                        currentIdentity: registrationIdentity
                    ) {
                        repairFileProviderRegistration(
                            existingDomain: existingDomain,
                            runtime: runtime,
                            currentIdentity: registrationIdentity,
                            completion
                        )
                        return
                    }

                    let state = fileProviderDomainState(for: existingDomain)
                    if state == .registered || state == .disabled {
                        markFileProviderRegistrationCurrent(registrationIdentity)
                        irisDriveDebugLog(
                            "Iris Drive FileProvider domain found after add error: \(error)"
                        )
                        completion(state)
                        return
                    }
                }

                if attempt < 5 {
                    let delay = Double(attempt)
                    irisDriveDebugLog(
                        "Iris Drive FileProvider registration attempt \(attempt) failed; retrying in \(delay)s: \(error)"
                    )
                    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + delay) {
                        ensureFileProviderDomainRegistered(
                            attempt: attempt + 1,
                            runtime: runtime,
                            completion
                        )
                    }
                    return
                }

                irisDriveDebugLog("Iris Drive FileProvider registration failed: \(error)")
                completion(.unavailable)
            }
        } else {
            markFileProviderRegistrationCurrent(registrationIdentity)
            queryFileProviderDomainStateWithError { state, queryError in
                if let queryError, shouldRepairFileProviderDomain(after: queryError) {
                    repairAllFileProviderRegistrations(
                        reason: "domain query failed after add",
                        runtime: runtime,
                        currentIdentity: registrationIdentity,
                        completion
                    )
                    return
                }
                if state == .registered || state == .disabled {
                    completion(state)
                } else {
                    irisDriveDebugLog("Iris Drive FileProvider domain registered")
                    completion(.registered)
                }
            }
        }
    }
}

private func shouldRepairFileProviderRegistration(
    _ existingDomain: NSFileProviderDomain,
    currentIdentity: String
) -> Bool {
    _ = existingDomain
    guard !currentIdentity.isEmpty else {
        return true
    }
    return UserDefaults.standard.string(
        forKey: irisDriveFileProviderRegistrationIdentityKey
    ) != currentIdentity
}

private func repairFileProviderRegistration(
    existingDomain: NSFileProviderDomain,
    runtime: FileProviderRuntimeConfig,
    currentIdentity: String,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    let freshDomain = irisDriveFileProviderDomain(runtime: runtime)
    clearFileProviderRegistrationIdentity()
    irisDriveDebugLog("Iris Drive repairing stale FileProvider domain registration")
    let addFreshDomain: (Error?) -> Void = { removeError in
        if let removeError {
            irisDriveDebugLog(
                "Iris Drive FileProvider domain removal before repair failed: \(removeError)"
            )
        }
        addFreshFileProviderDomain(
            freshDomain,
            currentIdentity: currentIdentity,
            reason: "domain repair",
            completion
        )
    }

    removeFileProviderDomain(
        existingDomain,
        reason: "stale registration repair",
        addFreshDomain
    )
}

private func repairAllFileProviderRegistrations(
    reason: String,
    runtime: FileProviderRuntimeConfig,
    currentIdentity: String,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    let freshDomain = irisDriveFileProviderDomain(runtime: runtime)
    clearFileProviderRegistrationIdentity()
    irisDriveDebugLog("Iris Drive repairing orphaned FileProvider domains: \(reason)")
    NSFileProviderManager.removeAllDomains { error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider remove all domains failed: \(error)")
            completion(.unavailable)
            return
        }
        addFreshFileProviderDomain(
            freshDomain,
            currentIdentity: currentIdentity,
            reason: "orphaned domain repair",
            completion
        )
    }
}

private func addFreshFileProviderDomain(
    _ domain: NSFileProviderDomain,
    currentIdentity: String,
    reason: String,
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    NSFileProviderManager.add(domain) { addError in
        if let addError {
            irisDriveDebugLog("Iris Drive FileProvider \(reason) failed: \(addError)")
            completion(.unavailable)
            return
        }
        markFileProviderRegistrationCurrent(currentIdentity)
        queryFileProviderDomainState { state in
            completion(state == .unavailable ? .registered : state)
        }
    }
}

private func queryFileProviderDomainState(
    _ completion: @escaping (FileProviderDomainState) -> Void
) {
    queryFileProviderDomainStateWithError { state, _ in
        completion(state)
    }
}

private func queryFileProviderDomainStateWithError(
    _ completion: @escaping (FileProviderDomainState, Error?) -> Void
) {
    NSFileProviderManager.getDomainsWithCompletionHandler { domains, error in
        if let error {
            irisDriveDebugLog("Iris Drive FileProvider domain query failed: \(error)")
        }
        guard let domain = domains.first(where: { $0.identifier == irisDriveDomainIdentifier }) else {
            completion(.unavailable, error)
            return
        }

        completion(fileProviderDomainState(for: domain), error)
    }
}

private func fileProviderDomainState(for domain: NSFileProviderDomain) -> FileProviderDomainState {
    irisDriveDebugLog(
        "Iris Drive FileProvider domain state userEnabled=\(domain.userEnabled) " +
        "hidden=\(domain.isHidden) disconnected=\(domain.isDisconnected)"
    )
    return domain.userEnabled ? .registered : .disabled
}

func shouldRepairFileProviderDomain(after error: Error) -> Bool {
    let nsError = error as NSError
    if nsError.domain == NSCocoaErrorDomain && nsError.code == NSFileReadNoPermissionError {
        return true
    }
    if nsError.domain == NSFileProviderErrorDomain && [-2001, -2014].contains(nsError.code) {
        return true
    }
    return false
}
