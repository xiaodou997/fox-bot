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
            draftMatches: true,
            recentUserInput: false,
            composingVerifiedSafe: true
        )
    }

    func testAllTrustedFactsAreRequiredForReady() {
        let decision = NativeSendGatePolicy.evaluate(readyFacts())
        XCTAssertTrue(decision.ready)
        XCTAssertEqual(decision.blockers, [])
    }

    func testUnverifiedCompositionFailsClosed() {
        var facts = readyFacts()
        facts.composingVerifiedSafe = false
        let decision = NativeSendGatePolicy.evaluate(facts)
        XCTAssertFalse(decision.ready)
        XCTAssertEqual(decision.blockers, [.composingUnverified])
    }

    func testMultipleUnsafeFactsAreAllReported() {
        var facts = readyFacts()
        facts.frontmost = false
        facts.draftMatches = false
        facts.recentUserInput = true
        let decision = NativeSendGatePolicy.evaluate(facts)
        XCTAssertFalse(decision.ready)
        XCTAssertEqual(
            decision.blockers,
            [.appNotFrontmost, .draftMismatch, .recentUserInput]
        )
    }
}
