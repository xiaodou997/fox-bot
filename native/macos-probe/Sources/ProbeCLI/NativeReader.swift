import AppKit
import ApplicationServices
import ProbeKit

/// This type has no UI action, setter, input, screenshot or networking method.
final class NativeReader: TreeReader {
    private var elements: [AXUIElement]
    private let app: TargetApp
    private let deadline: TimeInterval
    init(window: AXUIElement, app: TargetApp, deadline: TimeInterval) {
        elements = [window]; self.app = app; self.deadline = deadline
        AXUIElementSetMessagingTimeout(window, 0.12)
    }
    func expired() -> Bool { ProcessInfo.processInfo.systemUptime >= deadline }
    static func attribute(_ element: AXUIElement, _ name: String) -> (AXError, CFTypeRef?) {
        var result: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(element, name as CFString, &result)
        return (status, result)
    }
    static func boundedString(_ value: CFTypeRef?, max: Int = 128) -> String? {
        guard let value, CFGetTypeID(value) == CFStringGetTypeID() else { return nil }
        let string = value as! CFString
        guard CFStringGetLength(string) <= max else { return nil }
        return string as String
    }
    static func textState(_ element: AXUIElement, _ name: String) -> FieldState {
        let (status, value) = attribute(element, name)
        guard status == .success, let value, CFGetTypeID(value) == CFStringGetTypeID() else {
            return .unavailable
        }
        return CFStringGetLength((value as! CFString)) == 0 ? .empty : .nonempty
    }
    func facts(_ id: Int) -> NodeFacts {
        guard !expired(), elements.indices.contains(id) else { return NodeFacts(error: true) }
        let element = elements[id]
        let (status, roleValue) = Self.attribute(element, kAXRoleAttribute)
        guard status == .success, let role = Self.boundedString(roleValue) else { return NodeFacts(error: true) }
        let (subroleStatus, subroleValue) = Self.attribute(element, kAXSubroleAttribute)
        guard subroleStatus == .success || subroleStatus == .attributeUnsupported || subroleStatus == .noValue else {
            return NodeFacts(error: true) // Do not read values when protection cannot be checked.
        }
        let subrole = Self.boundedString(subroleValue)
        if role == "AXSecureTextField" || subrole == "AXSecureTextField" {
            return NodeFacts(role: .secureText, value: .protected)
        }
        let kind: NodeRole
        switch role {
        case kAXStaticTextRole: kind = .staticText
        case kAXTextAreaRole: kind = .textArea
        case kAXTextFieldRole: kind = .textField
        default: kind = .other
        }
        var classes: [String] = []
        if app == .qq, !expired() {
            var raw: CFArray?
            let status = AXUIElementCopyAttributeValues(element, "AXDOMClassList" as CFString, 0, 64, &raw)
            if status == .success, let array = raw as? [AnyObject] {
                classes = array.prefix(64).compactMap { Self.boundedString($0 as CFTypeRef) }
            }
        }
        let editor = app == .qq && kind == .textArea && classes.contains("ExEditor-qq-msg-editor")
        let message = app == .qq && classes.contains("msg-content-container")
        var value: FieldState = .notRead
        if kind == .staticText || editor || message {
            value = expired() ? .unavailable : Self.textState(element, kAXValueAttribute)
        }
        return NodeFacts(role: kind, value: value, messageCandidate: message, editorCandidate: editor)
    }
    func children(_ id: Int, limit: Int) -> ChildPage {
        guard !expired(), elements.indices.contains(id) else { return ChildPage(error: true) }
        let element = elements[id]
        var count = 0
        let counted = AXUIElementGetAttributeValueCount(element, kAXChildrenAttribute as CFString, &count)
        if counted == .attributeUnsupported || counted == .noValue { return ChildPage() }
        guard counted == .success, count >= 0 else { return ChildPage(error: true) }
        if count == 0 { return ChildPage() }
        if expired() { return ChildPage(error: true) }
        var raw: CFArray?
        let status = AXUIElementCopyAttributeValues(element, kAXChildrenAttribute as CFString,
                                                   0, min(count, limit), &raw)
        guard status == .success, let values = raw as? [AXUIElement] else { return ChildPage(error: true) }
        var ids: [Int] = [], truncated = count > limit
        for element in values.prefix(limit) {
            if let existing = elements.firstIndex(where: { CFEqual($0, element) }) { ids.append(existing) }
            else if elements.count < 2048 {
                AXUIElementSetMessagingTimeout(element, 0.12)
                ids.append(elements.count); elements.append(element)
            } else { truncated = true }
        }
        return ChildPage(ids, truncated: truncated)
    }
}
