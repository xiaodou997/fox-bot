import XCTest
@testable import ProbeKit

final class Fixture: TreeReader {
    var nodes: [Int: NodeFacts] = [:], pages: [Int: ChildPage] = [:]
    var queried: [Int] = [], childQueries: [Int] = []
    func facts(_ id: Int) -> NodeFacts { queried.append(id); return nodes[id] ?? NodeFacts() }
    func children(_ id: Int, limit: Int) -> ChildPage { childQueries.append(id); return pages[id] ?? ChildPage() }
}
final class TreeSummaryTests: XCTestCase {
    func testReadGateDoesNotPromptOrReadWhenPermissionIsMissing() {
        XCTAssertEqual(probeGate(runningInstances: 1, allowRead: true, trusted: false), "PERMISSION_REQUIRED")
    }
    func testDefaultGateNeverReadsAXEvenWithPermission() {
        XCTAssertEqual(probeGate(runningInstances: 1, allowRead: false, trusted: true), "METADATA_ONLY")
    }
    func testInstanceAmbiguityAndAbsenceNeverSelectAnArbitraryApp() {
        XCTAssertEqual(probeGate(runningInstances: 0, allowRead: true, trusted: true), "NOT_RUNNING")
        XCTAssertEqual(probeGate(runningInstances: 2, allowRead: true, trusted: true), "AMBIGUOUS_INSTANCE")
    }
    func testExplicitReadRequiresAllPrerequisites() {
        XCTAssertEqual(probeGate(runningInstances: 1, allowRead: true, trusted: true), "AX_ALLOWED")
    }
    func testReadabilityIsNotVerifiedIdentity() {
        let fixture = Fixture()
        fixture.pages[0] = ChildPage([1, 2])
        fixture.nodes[1] = NodeFacts(role: .staticText, value: .nonempty, messageCandidate: true)
        fixture.nodes[2] = NodeFacts(role: .textArea, value: .empty, editorCandidate: true)
        let report = summarize(fixture)
        XCTAssertEqual(report.visitedNodes, 3)
        XCTAssertEqual(report.readableMessageCandidates, 1)
        XCTAssertEqual(report.editorState, .empty)
        XCTAssertEqual(report.conversationIdentity, "UNVERIFIED")
        XCTAssertEqual(report.sendCapability, "NOT_IMPLEMENTED")
    }
    func testUnreadableEditorIsNotEmpty() {
        let fixture = Fixture(); fixture.nodes[0] = NodeFacts(role: .textArea, value: .unavailable, editorCandidate: true)
        XCTAssertEqual(summarize(fixture).editorState, .unavailable)
    }
    func testMultipleEditorsAreAmbiguous() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage([1,2])
        fixture.nodes[1] = NodeFacts(role: .textArea, value: .empty, editorCandidate: true)
        fixture.nodes[2] = NodeFacts(role: .textArea, value: .nonempty, editorCandidate: true)
        XCTAssertEqual(summarize(fixture).editorState, .ambiguous)
    }
    func testNoEditorDoesNotClaimAnEmptyDraft() {
        XCTAssertEqual(summarize(Fixture()).editorState, .notRead)
    }
    func testCyclesAreBoundedAndMarkedPartial() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage([1]); fixture.pages[1] = ChildPage([0])
        let report = summarize(fixture)
        XCTAssertEqual(report.visitedNodes, 2); XCTAssertFalse(report.completeTraversal)
        XCTAssertTrue(report.partialReasons.contains("REPEATED_NODE"))
    }
    func testNodeAndChildLimitsBoundReaderWork() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage(Array(1...10000))
        let report = summarize(fixture, limits: ScanLimits(nodes: 8, children: 4))
        XCTAssertLessThanOrEqual(report.visitedNodes, 8)
        XCTAssertTrue(report.partialReasons.contains("CHILD_LIMIT"))
    }
    func testDepthLimitDoesNotClaimCompleteHistory() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage([1]); fixture.pages[1] = ChildPage([2])
        let report = summarize(fixture, limits: ScanLimits(depth: 1))
        XCTAssertEqual(report.visitedNodes, 2)
        XCTAssertTrue(report.partialReasons.contains("DEPTH_LIMIT"))
    }
    func testDeadlineStopsBeforeAnyRead() {
        let fixture = Fixture(); let report = summarize(fixture, expired: { true })
        XCTAssertEqual(fixture.queried, [])
        XCTAssertEqual(report.partialReasons, ["DEADLINE"])
    }
    func testSecureFieldsAreNotExpandedOrCountedAsMessages() {
        let fixture = Fixture(); fixture.nodes[0] = NodeFacts(role: .secureText, value: .protected, messageCandidate: true, editorCandidate: true)
        fixture.pages[0] = ChildPage([1])
        let report = summarize(fixture)
        XCTAssertEqual(fixture.childQueries, []); XCTAssertEqual(report.messageCandidates, 0)
        XCTAssertEqual(report.editorCandidates, 0); XCTAssertEqual(report.protectedNodes, 1)
    }
    func testReaderFailureIsNotAnEmptyCompleteTree() {
        let fixture = Fixture(); fixture.nodes[0] = NodeFacts(error: true); fixture.pages[0] = ChildPage(error: true)
        let report = summarize(fixture)
        XCTAssertEqual(report.readErrors, 1); XCTAssertFalse(report.completeTraversal)
        XCTAssertEqual(fixture.childQueries, [])
    }
    func testPartialTreeCannotConfirmOneEmptyEditor() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage([1], truncated: true)
        fixture.nodes[1] = NodeFacts(role: .textArea, value: .empty, editorCandidate: true)
        let report = summarize(fixture)
        XCTAssertEqual(report.editorCandidates, 1)
        XCTAssertEqual(report.editorState, .ambiguous)
        XCTAssertFalse(report.completeTraversal)
    }
    func testPartialTreeCannotConfirmOneNonemptyEditor() {
        let fixture = Fixture(); fixture.pages[0] = ChildPage([1], error: true)
        fixture.nodes[1] = NodeFacts(role: .textArea, value: .nonempty, editorCandidate: true)
        XCTAssertEqual(summarize(fixture).editorState, .ambiguous)
    }
    func testUnavailableTextIsReportedAsReadError() {
        let fixture = Fixture(); fixture.nodes[0] = NodeFacts(role: .staticText, value: .unavailable)
        let report = summarize(fixture)
        XCTAssertEqual(report.readErrors, 1)
        XCTAssertEqual(report.partialReasons, ["READ_ERROR"])
        XCTAssertEqual(report.readableStaticTextNodes, 0)
    }
    func testDeadlineAfterEditorReadCannotClaimUniqueEmptyDraft() {
        let fixture = Fixture(); fixture.nodes[0] = NodeFacts(role: .textArea, value: .empty, editorCandidate: true)
        let report = summarize(fixture, expired: { !fixture.queried.isEmpty })
        XCTAssertEqual(report.editorState, .ambiguous)
        XCTAssertEqual(report.partialReasons, ["DEADLINE"])
    }
    func testSerializationIsOnlyClosedMetadata() throws {
        let report = summarize(Fixture())
        let json = String(decoding: try JSONEncoder().encode(report), as: UTF8.self)
        XCTAssertTrue(json.contains("UNVERIFIED")); XCTAssertFalse(report.rawTextIncluded)
        XCTAssertFalse(json.contains("draft_text")); XCTAssertFalse(report.screenshotTaken)
    }
    func testTargetAllowlistIsExact() {
        XCTAssertNil(TargetApp(rawValue: "terminal")); XCTAssertNil(TargetApp(rawValue: "QQ"))
        XCTAssertEqual(TargetApp.qq.bundleID, "com.tencent.qq")
        XCTAssertEqual(TargetApp.wechat.bundleID, "com.tencent.xinWeChat")
    }
}
