import AppKit
import ApplicationServices
import CoreGraphics
import ScreenCaptureKit
import OCRKit
import ProbeKit

/// Uses ScreenCaptureKit's exact single-window filter. No full-display capture,
/// input actions, window activation, permission request or screenshot file exists here.
final class NativeWindowSource: WindowSource {
    private var selectedWindows: [UInt32: SCWindow] = [:]

    /// WeChat 4.x exposes its AX root through com.tencent.xinWeChat while large
    /// compositor windows can be owned by the nested WeChatAppEx application.
    /// Accept only fixed bundle ids whose executable remains inside the unique root app bundle.
    private func familyMembers(_ app: TargetApp, rootPid: Int32) -> [Int32: String] {
        let roots = NSRunningApplication.runningApplications(withBundleIdentifier: app.bundleID)
            .filter { !$0.isTerminated }
        guard roots.count == 1, roots[0].processIdentifier == rootPid,
              let rootURL = roots[0].bundleURL?.standardizedFileURL else { return [:] }
        let allowed: Set<String> = switch app {
        case .wechat: [app.bundleID, "com.tencent.flue.WeChatAppEx"]
        case .qq: [app.bundleID]
        }
        let prefix = rootURL.path.hasSuffix("/") ? rootURL.path : rootURL.path + "/"
        var family: [Int32: String] = [:]
        for process in NSWorkspace.shared.runningApplications {
            guard !process.isTerminated, let bundle = process.bundleIdentifier, allowed.contains(bundle),
                  let executable = process.executableURL?.standardizedFileURL.path,
                  executable.hasPrefix(prefix) else { continue }
            family[process.processIdentifier] = bundle
        }
        return family
    }

    func metadata(_ app: TargetApp) -> CaptureMetadata {
        let os = ProcessInfo.processInfo.operatingSystemVersion
        let instances = NSRunningApplication.runningApplications(withBundleIdentifier: app.bundleID).filter { !$0.isTerminated }
        var version: String?
        if instances.count == 1, let url = instances[0].bundleURL,
           let value = Bundle(url: url)?.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
           !value.isEmpty, value.utf8.count <= 64,
           value.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || ".-_".contains($0)) }) {
            version = value
        }
        return CaptureMetadata(runningInstances: instances.count,
            pid: instances.count == 1 ? instances[0].processIdentifier : nil,
            launchTime: instances.count == 1 ? instances[0].launchDate?.timeIntervalSince1970 : nil,
            permission: CGPreflightScreenCaptureAccess(),
            osVersion: "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)", applicationVersion: version)
    }

    func windows(_ app: TargetApp, pid: Int32) async throws -> [CaptureWindow] {
        // Include off-screen/other-Space windows because AX can legitimately focus a window
        // that ScreenCaptureKit marks off-screen. Unique mode still filters them out later.
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
        guard content.windows.count <= 4096 else { throw OCRFailure.resourceLimit }
        let family = familyMembers(app, rootPid: pid)
        guard family[pid] == app.bundleID else { throw OCRFailure.enumerationFailed }
        selectedWindows.removeAll()
        var result: [CaptureWindow] = []
        for window in content.windows {
            guard let owner = window.owningApplication,
                  family[owner.processID] == owner.bundleIdentifier,
                  window.windowLayer == 0 else { continue }
            let filter = SCContentFilter(desktopIndependentWindow: window)
            let candidate = CaptureWindow(id: window.windowID, pid: pid, bundleID: app.bundleID,
                frame: window.frame, contentSize: filter.contentRect.size, scale: Double(filter.pointPixelScale),
                onScreen: window.isOnScreen, layer: window.windowLayer,
                ownerPid: owner.processID, ownerBundleID: owner.bundleIdentifier)
            result.append(candidate)
            selectedWindows[window.windowID] = window
        }
        return result
    }

    func focusedFrame(_ app: TargetApp, pid: Int32) throws -> CGRect {
        guard AXIsProcessTrusted() else { throw OCRFailure.accessibilityRequired }
        let root = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(root, 0.12)
        func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
            var value: CFTypeRef?
            guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success,
                  let value else { throw OCRFailure.focusedWindowUnavailable }
            return value
        }
        let raw = try attribute(root, kAXFocusedWindowAttribute)
        guard CFGetTypeID(raw) == AXUIElementGetTypeID() else { throw OCRFailure.focusedWindowUnavailable }
        let window = raw as! AXUIElement
        AXUIElementSetMessagingTimeout(window, 0.12)
        guard (try attribute(window, kAXRoleAttribute) as? String) == kAXWindowRole,
              (try attribute(window, kAXSubroleAttribute) as? String) == kAXStandardWindowSubrole,
              let minimized = try attribute(window, kAXMinimizedAttribute) as? Bool, !minimized else {
            throw OCRFailure.focusedWindowUnavailable
        }
        let position = try attribute(window, kAXPositionAttribute)
        let size = try attribute(window, kAXSizeAttribute)
        guard CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else {
            throw OCRFailure.focusedWindowUnavailable
        }
        var point = CGPoint.zero, dimensions = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &point),
              AXValueGetValue(size as! AXValue, .cgSize, &dimensions) else { throw OCRFailure.focusedWindowUnavailable }
        return CGRect(origin: point, size: dimensions)
    }

    func capture(_ candidate: CaptureWindow, plan: ImagePlan) async throws -> CGImage {
        guard CGPreflightScreenCaptureAccess(),
              let app = TargetApp.allCases.first(where: { $0.bundleID == candidate.bundleID }),
              familyMembers(app, rootPid: candidate.pid)[candidate.ownerPid] == candidate.ownerBundleID,
              let window = selectedWindows[candidate.id],
              window.owningApplication?.processID == candidate.ownerPid,
              window.owningApplication?.bundleIdentifier == candidate.ownerBundleID,
              window.windowLayer == 0, window.frame == candidate.frame else {
            throw OCRFailure.invalidGeometry
        }
        let filter = SCContentFilter(desktopIndependentWindow: window)
        guard filter.contentRect.size == candidate.contentSize,
              Double(filter.pointPixelScale) == candidate.scale else { throw OCRFailure.invalidGeometry }
        let config = SCStreamConfiguration()
        config.width = plan.width
        config.height = plan.height
        config.showsCursor = false
        config.ignoreShadowsSingleWindow = true
        config.includeChildWindows = false
        config.scalesToFit = true
        config.capturesAudio = false
        return try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
    }
}
