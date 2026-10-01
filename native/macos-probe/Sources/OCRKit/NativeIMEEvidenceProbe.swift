import AppKit
import ApplicationServices
import Carbon
import CoreGraphics
import Foundation
import ProbeKit

public enum NativeIMEEvidenceProbe {
    private static func attribute(
        _ element: AXUIElement,
        _ name: String
    ) -> (AXError, CFTypeRef?) {
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(element, name as CFString, &value)
        return (status, value)
    }

    private static func focusedElement(_ root: AXUIElement) -> AXUIElement? {
        let (status, value) = attribute(root, kAXFocusedUIElementAttribute)
        guard status == .success,
              let value,
              CFGetTypeID(value) == AXUIElementGetTypeID()
        else {
            return nil
        }
        return (value as! AXUIElement)
    }

    private static func selectedRangeAvailable(_ element: AXUIElement?) -> Bool {
        guard let element else { return false }
        let (status, value) = attribute(element, kAXSelectedTextRangeAttribute)
        return status == .success && value != nil
    }

    private static func processID(_ element: AXUIElement?) -> pid_t? {
        guard let element else { return nil }
        var pid: pid_t = 0
        guard AXUIElementGetPid(element, &pid) == .success else { return nil }
        return pid
    }

    private static func inputSourceString(
        _ source: TISInputSource,
        key: CFString
    ) -> String? {
        guard let pointer = TISGetInputSourceProperty(source, key) else { return nil }
        return Unmanaged<CFTypeRef>
            .fromOpaque(pointer)
            .takeUnretainedValue() as? String
    }

    private static func recentUserInput() -> Bool {
        let types: [CGEventType] = [
            .keyDown, .flagsChanged, .leftMouseDown, .rightMouseDown,
            .otherMouseDown, .scrollWheel,
        ]
        let quiet = types
            .map {
                CGEventSource.secondsSinceLastEventType(
                    .combinedSessionState,
                    eventType: $0
                )
            }
            .min() ?? 0
        return quiet < 1.0
    }

    public static func collect(target: TargetApp = .wechat) -> IMEEvidenceFacts {
        let applications = NSRunningApplication
            .runningApplications(withBundleIdentifier: target.bundleID)
            .filter { !$0.isTerminated }
        let appRoot = applications.count == 1
            ? AXUIElementCreateApplication(applications[0].processIdentifier)
            : nil
        if let appRoot {
            AXUIElementSetMessagingTimeout(appRoot, 0.20)
        }
        let appFocused = appRoot.flatMap(focusedElement)
        let system = AXUIElementCreateSystemWide()
        AXUIElementSetMessagingTimeout(system, 0.20)
        let rawSystemFocused = focusedElement(system)
        let targetPID = applications.first?.processIdentifier
        let systemFocused =
            processID(rawSystemFocused) == targetPID ? rawSystemFocused : nil

        var sourceBundle = ""
        var inputSourceAvailable = false
        if let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() {
            inputSourceAvailable = true
            sourceBundle = inputSourceString(source, key: kTISPropertyBundleID) ?? ""
        }

        let inputMethodApps = NSWorkspace.shared.runningApplications.filter { application in
            guard !application.isTerminated else { return false }
            let bundle = application.bundleIdentifier ?? ""
            let path = application.executableURL?.path ?? ""
            return (!sourceBundle.isEmpty && bundle.hasPrefix(sourceBundle))
                || path.contains("/Input Methods/")
        }
        let pids = Set(inputMethodApps.map(\.processIdentifier))
        var windowCount = 0
        var onScreenWindowCount = 0
        if let rows = CGWindowListCopyWindowInfo(
            [.optionAll, .excludeDesktopElements],
            kCGNullWindowID
        ) as? [[String: Any]] {
            for row in rows {
                guard let pid = row[kCGWindowOwnerPID as String] as? Int,
                      pids.contains(pid_t(pid))
                else {
                    continue
                }
                windowCount += 1
                if (row[kCGWindowIsOnscreen as String] as? Bool) == true {
                    onScreenWindowCount += 1
                }
            }
        }

        // No documented cross-process API exposes another application's
        // NSTextInputClient.hasMarkedText/markedRange. This remains unavailable
        // until the target publishes an authoritative composition signal.
        return IMEEvidenceFacts(
            authoritativeState: .unavailable,
            appFocusedUIAvailable: appFocused != nil,
            systemFocusedUIAvailable: systemFocused != nil,
            selectedTextRangeAvailable:
                selectedRangeAvailable(appFocused) || selectedRangeAvailable(systemFocused),
            inputSourceAvailable: inputSourceAvailable,
            inputMethodProcessCount: inputMethodApps.count,
            inputMethodWindowCount: windowCount,
            inputMethodOnScreenWindowCount: onScreenWindowCount,
            recentUserInput: recentUserInput(),
            targetFrontmost:
                NSWorkspace.shared.frontmostApplication?.bundleIdentifier == target.bundleID
        )
    }
}
