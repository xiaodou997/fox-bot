import XCTest
@testable import OCRKit

final class SendGatePolicyTests: XCTestCase {
    private func readyFacts() -> NativeSendGateFacts {
        NativeSendGateFacts(
            captureReady: true,
            frontmost: true,
            stableTwoReads: true,
            applicationSessionMatches: true,
            conversationResolved: true,
            conversationMatches: true,
            draftMatches: true
        )
    }

    func testUnattendedReadyNeedsNoInputStateEvidence() {
        let decision = NativeSendGatePolicy.evaluate(readyFacts())
        XCTAssertTrue(decision.ready)
        XCTAssertEqual(decision.blockers, [])
    }

    func testEveryExecutionFactStillBlocksIndependently() {
        let cases: [(WritableKeyPath<NativeSendGateFacts, Bool>, NativeSendGateBlocker)] = [
            (\.captureReady, .captureUnavailable),
            (\.frontmost, .appNotFrontmost),
            (\.stableTwoReads, .unstableSurface),
            (\.applicationSessionMatches, .applicationSessionMismatch),
            (\.conversationResolved, .conversationUnresolved),
            (\.conversationMatches, .conversationMismatch),
            (\.draftMatches, .draftMismatch)
        ]
        for (keyPath, blocker) in cases {
            var facts = readyFacts()
            facts[keyPath: keyPath] = false
            let decision = NativeSendGatePolicy.evaluate(facts)
            XCTAssertFalse(decision.ready, blocker.rawValue)
            XCTAssertEqual(decision.blockers, [blocker])
        }
    }

    func testMultipleUnsafeFactsAreAllReported() {
        var facts = readyFacts()
        facts.frontmost = false
        facts.draftMatches = false
        let decision = NativeSendGatePolicy.evaluate(facts)
        XCTAssertFalse(decision.ready)
        XCTAssertEqual(
            decision.blockers,
            [.appNotFrontmost, .draftMismatch]
        )
    }

    func testMissingExecutionEvidenceNeverBecomesReady() {
        let facts = NativeSendGateFacts(
            captureReady: false,
            frontmost: false,
            stableTwoReads: false,
            applicationSessionMatches: false,
            conversationResolved: false,
            conversationMatches: false,
            draftMatches: false
        )
        let decision = NativeSendGatePolicy.evaluate(facts)
        XCTAssertFalse(decision.ready)
        XCTAssertEqual(decision.blockers, NativeSendGateBlocker.allCases)
    }
}
