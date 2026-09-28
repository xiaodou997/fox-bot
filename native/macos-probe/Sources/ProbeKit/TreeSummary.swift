import Foundation

public enum TargetApp: String, Codable, CaseIterable {
    case qq, wechat
    public var bundleID: String {
        switch self {
        case .qq: return "com.tencent.qq"
        case .wechat: return "com.tencent.xinWeChat"
        }
    }
}

/// A failed or ambiguous prerequisite must not reach an AX tree read.
public func probeGate(runningInstances: Int, allowRead: Bool, trusted: Bool) -> String {
    if runningInstances == 0 { return "NOT_RUNNING" }
    if runningInstances != 1 { return "AMBIGUOUS_INSTANCE" }
    if !allowRead { return "METADATA_ONLY" }
    if !trusted { return "PERMISSION_REQUIRED" }
    return "AX_ALLOWED"
}

/// Only closed states enter the report; never title, description, message or draft text.
public enum FieldState: String, Codable {
    case notRead = "NOT_READ", unavailable = "UNAVAILABLE", empty = "EMPTY"
    case nonempty = "NONEMPTY", protected = "PROTECTED", ambiguous = "AMBIGUOUS"
}
public enum NodeRole { case staticText, textArea, textField, secureText, other }
public struct NodeFacts {
    public var role: NodeRole
    public var value: FieldState
    public var messageCandidate: Bool
    public var editorCandidate: Bool
    public var error: Bool
    public init(role: NodeRole = .other, value: FieldState = .notRead,
                messageCandidate: Bool = false, editorCandidate: Bool = false, error: Bool = false) {
        self.role = role; self.value = value; self.messageCandidate = messageCandidate
        self.editorCandidate = editorCandidate; self.error = error
    }
}
public struct ChildPage {
    public var ids: [Int]
    public var truncated: Bool
    public var error: Bool
    public init(_ ids: [Int] = [], truncated: Bool = false, error: Bool = false) {
        self.ids = ids; self.truncated = truncated; self.error = error
    }
}
public protocol TreeReader {
    func facts(_ id: Int) -> NodeFacts
    func children(_ id: Int, limit: Int) -> ChildPage
}
public struct ScanLimits {
    public var nodes: Int, depth: Int, children: Int
    public init(nodes: Int = 512, depth: Int = 24, children: Int = 64) {
        self.nodes = min(max(nodes, 1), 2048)
        self.depth = min(max(depth, 1), 32)
        self.children = min(max(children, 1), 128)
    }
}
public struct TreeSummary: Codable {
    public var visitedNodes = 0
    public var staticTextNodes = 0
    public var readableStaticTextNodes = 0
    public var protectedNodes = 0
    public var messageCandidates = 0
    public var readableMessageCandidates = 0
    public var editorCandidates = 0
    public var editorState = FieldState.notRead
    public var readErrors = 0
    public var partialReasons: [String] = []
    public var completeTraversal = false
    public var accountIdentity = "UNVERIFIED"
    public var conversationIdentity = "UNVERIFIED"
    public var sendCapability = "NOT_IMPLEMENTED"
    public var screenshotTaken = false
    public var rawTextIncluded = false
    public init() {}
}

/// Bounded breadth-first walk shared by the native reader and deterministic fixtures.
/// 'complete' means traversal of this window, NOT complete chat history or verified identity.
public func summarize<R: TreeReader>(_ reader: R, root: Int = 0,
                                    limits: ScanLimits = ScanLimits(),
                                    expired: () -> Bool = { false }) -> TreeSummary {
    var report = TreeSummary()
    var queue = [(root, 0)], visited = Set<Int>(), scheduled: Set<Int> = [root]
    var cursor = 0, reasons = Set<String>()
    var editors: [FieldState] = []
    while cursor < queue.count {
        if expired() { reasons.insert("DEADLINE"); break }
        if report.visitedNodes >= limits.nodes { reasons.insert("NODE_LIMIT"); break }
        let (id, depth) = queue[cursor]; cursor += 1
        guard visited.insert(id).inserted else { continue }
        let facts = reader.facts(id)
        report.visitedNodes += 1
        if facts.error {
            report.readErrors += 1; reasons.insert("READ_ERROR")
            continue // Unknown role may hide a protected subtree; do not expand it.
        }
        if facts.value == .unavailable && (facts.role == .staticText || facts.messageCandidate || facts.editorCandidate) {
            report.readErrors += 1; reasons.insert("READ_ERROR")
        }
        if facts.role == .secureText {
            report.protectedNodes += 1
            continue // Never query children or payloads of a protected field.
        }
        if facts.role == .staticText {
            report.staticTextNodes += 1
            if facts.value == .nonempty { report.readableStaticTextNodes += 1 }
        }
        if facts.messageCandidate {
            report.messageCandidates += 1
            if facts.value == .nonempty { report.readableMessageCandidates += 1 }
        }
        if facts.editorCandidate { editors.append(facts.value) }
        if expired() { reasons.insert("DEADLINE"); break }
        let page = reader.children(id, limit: min(limits.children, limits.nodes))
        if page.error { report.readErrors += 1; reasons.insert("READ_ERROR") }
        if page.truncated { reasons.insert("CHILD_LIMIT") }
        if depth >= limits.depth && !page.ids.isEmpty { reasons.insert("DEPTH_LIMIT"); continue }
        for child in page.ids.prefix(limits.children) {
            if scheduled.contains(child) { reasons.insert("REPEATED_NODE"); continue }
            if scheduled.count >= limits.nodes { reasons.insert("NODE_LIMIT"); break }
            scheduled.insert(child); queue.append((child, depth + 1))
        }
        if page.ids.count > limits.children { reasons.insert("CHILD_LIMIT") }
    }
    report.editorCandidates = editors.count
    report.editorState = editors.isEmpty ? .notRead : (editors.count == 1 ? editors[0] : .ambiguous)
    // A single candidate in a truncated tree is not proof it is the only editor.
    if !reasons.isEmpty && (report.editorState == .empty || report.editorState == .nonempty) {
        report.editorState = .ambiguous
    }
    report.partialReasons = reasons.sorted()
    report.completeTraversal = reasons.isEmpty
    return report
}
