import FileProvider
import Foundation

guard CommandLine.arguments.count == 3 else {
    fputs("usage: remove_fileprovider_domain.swift <domain-id> <display-name>\n", stderr)
    exit(2)
}

let domain = NSFileProviderDomain(
    identifier: NSFileProviderDomainIdentifier(CommandLine.arguments[1]),
    displayName: CommandLine.arguments[2]
)
let semaphore = DispatchSemaphore(value: 0)
var removalError: Error?
NSFileProviderManager.remove(domain) { error in
    removalError = error
    semaphore.signal()
}

guard semaphore.wait(timeout: .now() + 30) == .success else {
    fputs("FileProvider domain removal timed out\n", stderr)
    exit(75)
}
if let error = removalError as NSError? {
    // Removing an already-absent domain is an idempotent reset.
    if error.domain == NSFileProviderErrorDomain && error.code == NSFileProviderError.noSuchItem.rawValue {
        exit(0)
    }
    fputs("FileProvider domain removal failed: \(error)\n", stderr)
    exit(75)
}
