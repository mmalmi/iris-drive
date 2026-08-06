#!/usr/bin/env swift

import ApplicationServices
import Foundation

enum LinkDriverError: Error, CustomStringConvertible {
    case usage
    case accessibilityPermission
    case unexpectedProcess
    case missingDialog(String)
    case missingElement(String)
    case pressFailed(String, AXError)
    case setValueFailed(String, AXError)

    var description: String {
        switch self {
        case .usage:
            return "usage: macos-device-link-ax.swift <pid> <Cancel|Approve|SignIn|ManualPrepare|AssertJoined> <timeout-seconds> [request-url]"
        case .accessibilityPermission:
            return "Accessibility permission is required for the macOS device-link smoke"
        case .unexpectedProcess:
            return "stage=target_process category=unexpected_process"
        case .missingDialog(let stage):
            return "stage=\(stage) category=missing_dialog"
        case .missingElement(let stage):
            return "stage=\(stage) category=missing_element"
        case .pressFailed(let stage, let error):
            return "stage=\(stage) category=press_failed code=\(error.rawValue)"
        case .setValueFailed(let stage, let error):
            return "stage=\(stage) category=value_failed code=\(error.rawValue)"
        }
    }
}

private enum Control {
    static let signIn = "welcomeSignIn"
    static let devices = "sidebarDevices"
    static let addDevice = "addDeviceToggle"
    static let request = "manualDeviceApprovalInput"
    static let approve = "deviceApprovalApprove"
    static let cancel = "deviceApprovalCancel"
    static let joined = "driveTitle"
}

func reportStage(_ stage: String) {
    fputs("IRIS_MACOS_AX_STAGE=\(stage)\n", stderr)
}

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
        return nil
    }
    return value
}

func text(_ element: AXUIElement, _ name: String) -> String {
    attribute(element, name) as? String ?? ""
}

func boolAttribute(_ element: AXUIElement, _ name: String) -> Bool? {
    attribute(element, name) as? Bool
}

func descendants(_ root: AXUIElement) -> [AXUIElement] {
    var found: [AXUIElement] = []
    var pending = [root]
    var visited = Set<CFHashCode>()
    while let element = pending.popLast(), found.count < 20_000 {
        guard visited.insert(CFHash(element)).inserted else { continue }
        found.append(element)
        if let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] {
            pending.append(contentsOf: children.reversed())
        }
    }
    return found
}

func visibleElement(_ application: AXUIElement, identifier: String) -> AXUIElement? {
    descendants(application).first {
        text($0, kAXIdentifierAttribute) == identifier
            && boolAttribute($0, kAXHiddenAttribute) != true
    }
}

func waitForElement(
    _ application: AXUIElement, identifier: String, timeout: TimeInterval
) -> AXUIElement? {
    let deadline = Date().addingTimeInterval(timeout)
    repeat {
        if let element = visibleElement(application, identifier: identifier) {
            return element
        }
        Thread.sleep(forTimeInterval: 0.1)
    } while Date() < deadline
    return nil
}

func press(
    _ application: AXUIElement, identifier: String, stage: String, timeout: TimeInterval
) throws {
    let deadline = Date().addingTimeInterval(timeout)
    var lastError = AXError.actionUnsupported
    var sawElement = false
    repeat {
        if var element = visibleElement(application, identifier: identifier) {
            sawElement = true
            for _ in 0 ..< 8 {
                var actionNames: CFArray?
                let namesError = AXUIElementCopyActionNames(element, &actionNames)
                if namesError == .success,
                   let names = actionNames as? [String],
                   names.contains(kAXPressAction) {
                    let error = AXUIElementPerformAction(element, kAXPressAction as CFString)
                    if error == .success {
                        return
                    }
                    lastError = error
                    break
                }
                guard let parent = attribute(element, kAXParentAttribute) else { break }
                element = parent as! AXUIElement
            }
        }
        Thread.sleep(forTimeInterval: 0.1)
    } while Date() < deadline
    if !sawElement {
        throw LinkDriverError.missingElement(stage)
    }
    throw LinkDriverError.pressFailed(stage, lastError)
}

func setValue(
    _ application: AXUIElement, identifier: String, value: String,
    stage: String, timeout: TimeInterval
) throws {
    guard let field = waitForElement(application, identifier: identifier, timeout: timeout) else {
        throw LinkDriverError.missingElement(stage)
    }
    let focusError = AXUIElementSetAttributeValue(
        field,
        kAXFocusedAttribute as CFString,
        kCFBooleanTrue
    )
    guard focusError == .success else {
        throw LinkDriverError.setValueFailed(stage, focusError)
    }
    var pid = pid_t()
    let pidError = AXUIElementGetPid(application, &pid)
    guard pidError == .success else {
        throw LinkDriverError.setValueFailed(stage, pidError)
    }

    func postKey(_ keyCode: CGKeyCode, flags: CGEventFlags = []) -> Bool {
        let source = CGEventSource(stateID: .hidSystemState)
        guard let down = CGEvent(
            keyboardEventSource: source,
            virtualKey: keyCode,
            keyDown: true
        ), let up = CGEvent(
            keyboardEventSource: source,
            virtualKey: keyCode,
            keyDown: false
        ) else { return false }
        down.flags = flags
        up.flags = flags
        down.postToPid(pid)
        up.postToPid(pid)
        return true
    }

    guard postKey(0, flags: .maskCommand) else {
        throw LinkDriverError.setValueFailed(stage, .cannotComplete)
    }
    let utf16 = Array(value.utf16)
    let source = CGEventSource(stateID: .hidSystemState)
    guard let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
          let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false)
    else {
        throw LinkDriverError.setValueFailed(stage, .cannotComplete)
    }
    utf16.withUnsafeBufferPointer { buffer in
        down.keyboardSetUnicodeString(
            stringLength: buffer.count,
            unicodeString: buffer.baseAddress
        )
    }
    down.postToPid(pid)
    up.postToPid(pid)

    let deadline = Date().addingTimeInterval(2)
    repeat {
        if text(field, kAXValueAttribute) == value {
            return
        }
        Thread.sleep(forTimeInterval: 0.05)
    } while Date() < deadline
    throw LinkDriverError.setValueFailed(stage, .cannotComplete)
}

func run() throws {
    let arguments = CommandLine.arguments
    guard arguments.count == 4 || arguments.count == 5,
          let pid = pid_t(arguments[1]),
          let timeout = TimeInterval(arguments[3]),
          timeout > 0,
          ["Cancel", "Approve", "SignIn", "ManualPrepare", "AssertJoined"]
              .contains(arguments[2]),
          arguments[2] != "ManualPrepare" || arguments.count == 5
    else {
        throw LinkDriverError.usage
    }
    guard AXIsProcessTrusted() else {
        throw LinkDriverError.accessibilityPermission
    }

    let application = AXUIElementCreateApplication(pid)
    let processName = text(application, kAXTitleAttribute)
    guard processName.isEmpty || processName == "Iris Drive" else {
        throw LinkDriverError.unexpectedProcess
    }
    AXUIElementSetAttributeValue(
        application,
        kAXFrontmostAttribute as CFString,
        kCFBooleanTrue
    )

    switch arguments[2] {
    case "SignIn":
        try press(
            application, identifier: Control.signIn, stage: "sign_in_button", timeout: timeout
        )
        print("MACOS_DEVICE_LINK_SIGN_IN_OK")
    case "AssertJoined":
        guard waitForElement(
            application, identifier: Control.joined, timeout: timeout
        ) != nil else {
            throw LinkDriverError.missingElement("joined_ui")
        }
        print("MACOS_DEVICE_LINK_JOINED_UI_OK")
    case "ManualPrepare":
        for (identifier, stage) in [
            (Control.devices, "manual_devices"),
            (Control.addDevice, "manual_add_device"),
        ] {
            reportStage("\(stage)_waiting")
            try press(
                application, identifier: identifier, stage: "\(stage)_button", timeout: timeout
            )
            reportStage("\(stage)_pressed")
        }
        reportStage("manual_request_field_waiting")
        try setValue(
            application, identifier: Control.request, value: arguments[4],
            stage: "manual_request_field", timeout: timeout
        )
        reportStage("manual_request_entered")
        guard waitForElement(
            application, identifier: Control.approve, timeout: timeout
        ) != nil else {
            throw LinkDriverError.missingDialog("manual_confirmation")
        }
        reportStage("manual_confirmation_ready")
        print("MACOS_DEVICE_LINK_MANUAL_CONFIRMATION_READY")
    case "Approve", "Cancel":
        let identifier = arguments[2] == "Approve" ? Control.approve : Control.cancel
        guard waitForElement(
            application, identifier: identifier, timeout: timeout
        ) != nil else {
            throw LinkDriverError.missingDialog(
                "\(arguments[2].lowercased())_confirmation"
            )
        }
        try press(
            application, identifier: identifier,
            stage: "\(arguments[2].lowercased())_confirmation",
            timeout: timeout
        )
        print("MACOS_DEVICE_LINK_CONFIRMATION_\(arguments[2].uppercased())_OK")
    default:
        throw LinkDriverError.usage
    }
}

do {
    try run()
} catch LinkDriverError.accessibilityPermission {
    fputs("macOS device-link UI driver failed: \(LinkDriverError.accessibilityPermission)\n", stderr)
    exit(75)
} catch {
    fputs("macOS device-link UI driver failed: \(error)\n", stderr)
    exit(1)
}
