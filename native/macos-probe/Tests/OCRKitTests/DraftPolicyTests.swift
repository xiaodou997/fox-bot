import XCTest
@testable import OCRKit

final class DraftPolicyTests: XCTestCase {
    private func snapshot(_ lines: [OCRLine], complete: Bool = true) -> OCRSnapshot {
        OCRSnapshot(
            lines: lines,
            statistics: OCRStatistics(
                lineCount: lines.count,
                characterCount: lines.reduce(0) { $0 + $1.text.count },
                lowConfidenceLines: 0,
                partialReasons: complete ? [] : ["INVALID_OBSERVATION"],
                completeRecognition: complete
            )
        )
    }

    private func line(_ text: String, _ x: CGFloat = 0.30, _ y: CGFloat = 0.815) -> OCRLine {
        OCRLine(text: text, confidence: 0.9,
                bounds: CGRect(x: x, y: y, width: 0.20, height: 0.02))
    }

    func testEmptyDraftIsOnlyHeuristicWhenRecognitionIsComplete() {
        XCTAssertEqual(WeChatDraftPolicy.readState(snapshot([])), .emptyHeuristic)
        XCTAssertEqual(WeChatDraftPolicy.readState(snapshot([], complete: false)), .unreadable)
    }

    func testVisibleDraftIsNonempty() {
        XCTAssertEqual(WeChatDraftPolicy.readState(snapshot([line("draft")])), .nonempty)
    }

    func testControlsOutsideDraftRegionDoNotBecomeDraftText() {
        XCTAssertEqual(
            WeChatDraftPolicy.readState(snapshot([line("发送", 0.94, 0.95)])),
            .emptyHeuristic
        )
    }

    func testWechatEmptyPlaceholderDoesNotBecomeDraftText() {
        XCTAssertEqual(
            WeChatDraftPolicy.readState(snapshot([line("按住鼠标 语音输入文字")])),
            .emptyHeuristic
        )
    }

    func testVerificationAllowsVisionCaretButNotDifferentText() {
        XCTAssertTrue(WeChatDraftPolicy.verified(
            "FoxBot G3a Draft 731",
            snapshot: snapshot([line("FoxBot G3a Draft 731|")])
        ))
        XCTAssertFalse(WeChatDraftPolicy.verified(
            "FoxBot G3a Draft 731",
            snapshot: snapshot([line("FoxBot G3a Draft 732|")])
        ))
    }

    func testVerificationAllowsObservedAllCJKCaretOneButNeverNormalizesDigitsGenerally() {
        XCTAssertEqual(
            WeChatDraftPolicy.verifiedText(
                "南京是一座历史文化名城",
                snapshot: snapshot([line("南京是一座历史文化名城1")])
            ),
            "南京是一座历史文化名城"
        )
        XCTAssertFalse(WeChatDraftPolicy.verified(
            "南京是一座历史文化名城",
            snapshot: snapshot([line("南京是一座历史文化古城1")])
        ))
        XCTAssertFalse(WeChatDraftPolicy.verified(
            "reply",
            snapshot: snapshot([line("reply1")])
        ))
        XCTAssertTrue(WeChatDraftPolicy.verified(
            "版本1",
            snapshot: snapshot([line("版本1")])
        ))
    }
}
