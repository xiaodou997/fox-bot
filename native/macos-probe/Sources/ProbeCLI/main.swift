import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ProbeKit

struct ProbeReport: Encodable {
    var schemaVersion = "foxbot.macos-probe.v1"
    var app: String
    var bundleId: String
    var osVersion: String
    var applicationVersion: String? = nil
    var snapshotId = UUID().uuidString
    var readOnly = true
    var rawTextIncluded = false
    var screenshotTaken = false
    var networkRequests = 0
    var axReadRequested: Bool
    var accessibilityTrusted = false
    var screenCapturePreflight = false
    var runningInstances = 0
    var status = "METADATA_ONLY"
    var focusedWindowTitleState = FieldState.notRead
    var windowStable: Bool? = nil
    var tree: TreeSummary? = nil
    var accountIdentity = "UNVERIFIED"
    var conversationIdentity = "UNVERIFIED"
    var writeCapability = "NOT_IMPLEMENTED"
}

func focusedWindow(_ app: AXUIElement) -> AXUIElement? {
    let (status, value) = NativeReader.attribute(app, kAXFocusedWindowAttribute)
    guard status == .success, let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
    return (value as! AXUIElement)
}
func collect(_ app: TargetApp, allowRead: Bool) -> ProbeReport {
    let os = ProcessInfo.processInfo.operatingSystemVersion
    var report = ProbeReport(app: app.rawValue, bundleId: app.bundleID,
                             osVersion: "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)", axReadRequested: allowRead)
    // Query permission only: no prompt and no TCC settings mutation.
    report.accessibilityTrusted = AXIsProcessTrusted()
    report.screenCapturePreflight = CGPreflightScreenCaptureAccess()
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: app.bundleID).filter { !$0.isTerminated }
    report.runningInstances = apps.count
    let gate = probeGate(runningInstances: apps.count, allowRead: allowRead, trusted: report.accessibilityTrusted)
    guard apps.count == 1 else { report.status = gate; return report }
    let application = apps[0]
    if let url = application.bundleURL,
       let version = Bundle(url: url)?.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
       version.utf8.count <= 64, version.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || ".-_".contains($0)) }) {
        report.applicationVersion = version
    }
    guard gate == "AX_ALLOWED" else { report.status = gate; return report }
    let root = AXUIElementCreateApplication(application.processIdentifier)
    AXUIElementSetMessagingTimeout(root, 0.12)
    guard let window = focusedWindow(root) else { report.status = "NO_READABLE_FOCUSED_WINDOW"; return report }
    let deadline = ProcessInfo.processInfo.systemUptime + 1.5
    let reader = NativeReader(window: window, app: app, deadline: deadline)
    report.focusedWindowTitleState = NativeReader.textState(window, kAXTitleAttribute)
    let tree = summarize(reader, expired: { reader.expired() })
    guard !application.isTerminated, application.bundleIdentifier == app.bundleID,
          let currentWindow = focusedWindow(root), CFEqual(window, currentWindow) else {
        report.status = "WINDOW_CHANGED"; report.windowStable = false
        return report // Discard any summary from a stale target window.
    }
    report.windowStable = true
    report.tree = tree
    report.status = tree.completeTraversal ? "AX_SUMMARY" : "AX_PARTIAL_SUMMARY"
    return report
}

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments == ["--help"] || arguments.isEmpty {
    print("foxbot-macos-probe --app <qq|wechat> [--allow-ax-read]\nMetadata by default. No text output, screenshots, network, UI writes or permission prompts.")
} else if (arguments.count == 2 || arguments.count == 3), arguments[0] == "--app",
          let target = TargetApp(rawValue: arguments[1]),
          (arguments.count == 2 || arguments[2] == "--allow-ax-read") {
    let report = collect(target, allowRead: arguments.count == 3)
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    encoder.keyEncodingStrategy = .convertToSnakeCase
    do {
        let data = try encoder.encode(report)
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    } catch {
        FileHandle.standardError.write(Data("probe serialization failed\n".utf8))
        exit(2)
    }
} else {
    FileHandle.standardError.write(Data("invalid probe arguments\n".utf8))
    exit(2)
}
