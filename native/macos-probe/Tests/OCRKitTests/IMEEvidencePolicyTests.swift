import XCTest
@testable import OCRKit

final class IMEEvidencePolicyTests: XCTestCase {
    private func facts(
        state: AuthoritativeCompositionState = .unavailable
    ) -> IMEEvidenceFacts {
        IMEEvidenceFacts(
            authoritativeState: state,
            appFocusedUIAvailable: false,
            systemFocusedUIAvailable: false,
            selectedTextRangeAvailable: false,
            inputSourceAvailable: true,
            inputMethodProcessCount: 1,
            inputMethodWindowCount: 12,
            inputMethodOnScreenWindowCount: 0,
            recentUserInput: false,
            targetFrontmost: true
        )
    }

    func testUnavailableAuthoritativeStateNeverBecomesSafeFromHeuristics() {
        let decision = IMEEvidencePolicy.evaluate(facts())
        XCTAssertFalse(decision.compositionVerified)
        XCTAssertFalse(decision.composing)
        XCTAssertEqual(decision.positiveBlockers, [])
    }

    func testVisibleInputMethodWindowIsPositiveBlockerOnly() {
        var value = facts()
        value.inputMethodOnScreenWindowCount = 1
        let decision = IMEEvidencePolicy.evaluate(value)
        XCTAssertFalse(decision.compositionVerified)
        XCTAssertEqual(decision.positiveBlockers, ["INPUT_METHOD_WINDOW_VISIBLE"])
    }

    func testAuthoritativeSafeStateCanVerifyComposition() {
        let decision = IMEEvidencePolicy.evaluate(facts(state: .safe))
        XCTAssertTrue(decision.compositionVerified)
        XCTAssertFalse(decision.composing)
    }

    func testAuthoritativeComposingStateBlocks() {
        let decision = IMEEvidencePolicy.evaluate(facts(state: .composing))
        XCTAssertTrue(decision.compositionVerified)
        XCTAssertTrue(decision.composing)
        XCTAssertTrue(decision.positiveBlockers.contains("AUTHORITATIVE_COMPOSING"))
    }
}
